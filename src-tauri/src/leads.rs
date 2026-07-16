//! The core loop (§1): capture → enrich → draft → remind. Plus the
//! interruption policy machinery (§8.5): every unsolicited surface goes
//! through `surface()`, which enforces the hard-silence toggle and the
//! three-dismissals rule. No other code path may emit unsolicited UI.
use rusqlite::{params, Connection};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use crate::chat::{current_provider, run_chat, setting_or_default, DbState};
use crate::db;
use crate::providers::{ChatRequest, Msg};

// ---------------------------------------------------------------------------
// Capture — heuristic, zero-config, no LLM (§4.3: useful before any key).
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Serialize)]
pub struct ParsedLead {
    pub name: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub domain: Option<String>,
}

pub fn parse_capture(text: &str) -> ParsedLead {
    let email = find_email(text);
    let domain = email
        .as_deref()
        .and_then(|e| e.split('@').nth(1))
        .map(str::to_lowercase);
    let mut name = None;
    let mut company = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.contains('@') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let looks_like_company = ["ltd", "inc", "llc", "gmbh", "plc", "co.", "limited", "corp"]
            .iter()
            .any(|s| line.to_lowercase().contains(s));
        if looks_like_company && company.is_none() {
            company = Some(line.to_string());
        } else if name.is_none()
            && (2..=4).contains(&words.len())
            && words.iter().all(|w| w.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\''))
        {
            name = Some(line.to_string());
        }
    }
    // Fall back to the domain for a company guess: acme.example -> Acme.
    if company.is_none() {
        if let Some(d) = &domain {
            if let Some(stem) = d.split('.').next() {
                if !["gmail", "outlook", "hotmail", "yahoo", "icloud", "proton"].contains(&stem) {
                    let mut c = stem.to_string();
                    if let Some(f) = c.get_mut(0..1) {
                        f.make_ascii_uppercase();
                    }
                    company = Some(c);
                }
            }
        }
    }
    ParsedLead {
        name,
        company,
        email,
        domain,
    }
}

fn find_email(text: &str) -> Option<String> {
    // ponytail: hand-rolled scan beats pulling in the regex crate for one
    // pattern. Finds the first plausible token containing '@' and a dotted tld.
    for token in text.split(|c: char| c.is_whitespace() || ['<', '>', ',', ';', '('].contains(&c))
    {
        let token = token.trim_matches(|c: char| !c.is_alphanumeric());
        let Some((local, host)) = token.split_once('@') else {
            continue;
        };
        if !local.is_empty()
            && host.contains('.')
            && !host.ends_with('.')
            && host.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-')
        {
            return Some(token.to_string());
        }
    }
    None
}

pub fn insert_lead(conn: &Connection, raw: &str, source: &str) -> rusqlite::Result<i64> {
    let p = parse_capture(raw);
    conn.execute(
        "INSERT INTO lead (name, company, email, domain, source, raw_capture, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![p.name, p.company, p.email, p.domain, source, raw, db::now()],
    )?;
    Ok(conn.last_insert_rowid())
}

// ---------------------------------------------------------------------------
// Interruption policy (§8.5) — the only door for unsolicited surfaces.
// ---------------------------------------------------------------------------

pub const DISMISSAL_LIMIT: i64 = 3;

pub fn dismissal_count(conn: &Connection, trigger_type: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM dismissal WHERE trigger_type = ?1",
        [trigger_type],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

pub fn trigger_allowed(conn: &Connection, trigger_type: &str) -> bool {
    if setting_or_default(conn, "hard_silence") == "1" {
        return false;
    }
    dismissal_count(conn, trigger_type) < DISMISSAL_LIMIT
}

pub fn record_dismissal(
    conn: &Connection,
    trigger_type: &str,
    context_hash: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO dismissal (trigger_type, context_hash, created_at) VALUES (?1, ?2, ?3)",
        params![trigger_type, context_hash, db::now()],
    )?;
    Ok(())
}

/// Counts of surfaces actually shown, per trigger type (test observability).
#[derive(Default)]
pub struct SurfaceLog(pub Mutex<std::collections::HashMap<String, u64>>);

/// Attempt an unsolicited surface. Evidence is mandatory — never "it looks
/// like you're writing an email" (§8.5). Returns whether it was shown.
pub fn surface(app: &AppHandle, trigger_type: &str, evidence: &str, payload: serde_json::Value) -> bool {
    let dbs = app.state::<DbState>();
    let allowed = trigger_allowed(&dbs.0.lock().unwrap(), trigger_type);
    if !allowed {
        return false;
    }
    *app.state::<SurfaceLog>()
        .0
        .lock()
        .unwrap()
        .entry(trigger_type.to_string())
        .or_insert(0) += 1;
    let _ = app.emit(
        "surface",
        serde_json::json!({
            "trigger_type": trigger_type,
            "evidence": evidence,
            "payload": payload,
        }),
    );
    true
}

// ---------------------------------------------------------------------------
// Enrich (provider web search) and draft (to clipboard, never sent — §2).
// ---------------------------------------------------------------------------

struct LeadRow {
    name: Option<String>,
    company: Option<String>,
    email: Option<String>,
    domain: Option<String>,
    raw_capture: Option<String>,
}

fn lead_row(conn: &Connection, lead_id: i64) -> Result<LeadRow, String> {
    conn.query_row(
        "SELECT name, company, email, domain, raw_capture FROM lead WHERE id = ?1",
        [lead_id],
        |r| {
            Ok(LeadRow {
                name: r.get(0)?,
                company: r.get(1)?,
                email: r.get(2)?,
                domain: r.get(3)?,
                raw_capture: r.get(4)?,
            })
        },
    )
    .map_err(|_| format!("no lead with id {lead_id}"))
}

fn latest_enrichment(conn: &Connection, lead_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT payload_json FROM enrichment WHERE lead_id = ?1 ORDER BY id DESC LIMIT 1",
        [lead_id],
        |r| r.get(0),
    )
    .ok()
}

pub fn enrich(app: &AppHandle, lead_id: i64) -> Result<String, String> {
    let dbs = app.state::<DbState>();
    let (lead, provider) = {
        let conn = dbs.0.lock().unwrap();
        (lead_row(&conn, lead_id)?, current_provider(&conn)?)
    };
    if provider.name() == "ollama" {
        return Err("Enrichment needs the provider's web search — unavailable on Ollama.".into());
    }
    let who = format!(
        "{} at {} ({}, domain {})",
        lead.name.as_deref().unwrap_or("unknown name"),
        lead.company.as_deref().unwrap_or("unknown company"),
        lead.email.as_deref().unwrap_or("no email"),
        lead.domain.as_deref().unwrap_or("unknown")
    );
    let req = ChatRequest {
        system: crate::prompt::build_system_prompt(&[]),
        messages: vec![Msg {
            role: "user".into(),
            content: format!(
                "Research this sales lead using web search: {who}.\n\
                 Original capture:\n{}\n\n\
                 Return concise notes: what the company does, the person's \
                 likely role, any recent news, and 2-3 personalised talking \
                 points for a cold outreach email. Say clearly when you \
                 cannot find something — do not invent facts.",
                lead.raw_capture.as_deref().unwrap_or("")
            ),
        }],
        max_tokens: 1024,
        web_search: true,
    };
    let result = run_chat(&dbs.0, provider.as_ref(), "enrichment", &req, &mut |_| {})?;
    let conn = dbs.0.lock().unwrap();
    conn.execute(
        "INSERT INTO enrichment (lead_id, provider, payload_json, fetched_at, ttl)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            lead_id,
            provider.name(),
            serde_json::json!({ "notes": result.outcome.text }).to_string(),
            db::now(),
            7 * 24 * 3600,
        ],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO interaction (lead_id, kind, body, model_used, tokens_in, tokens_out, cost_pence, created_at)
         VALUES (?1, 'enrichment', ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            lead_id,
            result.outcome.text,
            provider.model(),
            result.outcome.usage.input_tokens as i64,
            result.outcome.usage.output_tokens as i64,
            result.cost_pence,
            db::now(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(result.outcome.text)
}

pub fn draft(app: &AppHandle, lead_id: i64) -> Result<String, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let dbs = app.state::<DbState>();
    let (lead, enrichment, provider) = {
        let conn = dbs.0.lock().unwrap();
        (
            lead_row(&conn, lead_id)?,
            latest_enrichment(&conn, lead_id),
            current_provider(&conn)?,
        )
    };
    let req = ChatRequest {
        system: crate::prompt::build_system_prompt(&[]),
        messages: vec![Msg {
            role: "user".into(),
            content: format!(
                "Draft a short, personalised cold outreach email to {} at {} ({}).\n\
                 Research notes:\n{}\n\nOriginal capture:\n{}\n\n\
                 Under 130 words, one specific hook from the notes, one clear \
                 ask. No placeholders like [Company] — use what you know. \
                 Output only the email (subject line first).",
                lead.name.as_deref().unwrap_or("the lead"),
                lead.company.as_deref().unwrap_or("their company"),
                lead.email.as_deref().unwrap_or("no email"),
                enrichment.as_deref().unwrap_or("(none — keep it generic but honest)"),
                lead.raw_capture.as_deref().unwrap_or(""),
            ),
        }],
        max_tokens: 1024,
        web_search: false,
    };
    let result = run_chat(&dbs.0, provider.as_ref(), "draft_email", &req, &mut |_| {})?;
    {
        let conn = dbs.0.lock().unwrap();
        conn.execute(
            "INSERT INTO interaction (lead_id, kind, body, model_used, tokens_in, tokens_out, cost_pence, created_at)
             VALUES (?1, 'draft_email', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                lead_id,
                result.outcome.text,
                provider.model(),
                result.outcome.usage.input_tokens as i64,
                result.outcome.usage.output_tokens as i64,
                result.cost_pence,
                db::now(),
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    // Draft goes to the clipboard. The user sends from their own client (§2).
    app.clipboard()
        .write_text(result.outcome.text.clone())
        .map_err(|e| format!("clipboard write failed: {e}"))?;
    Ok(result.outcome.text)
}

// ---------------------------------------------------------------------------
// Reminders — scheduled locally, fired by a scanner thread.
// ---------------------------------------------------------------------------

pub fn schedule_reminder(
    conn: &Connection,
    lead_id: i64,
    due_at: &str,
    note: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO reminder (lead_id, due_at, note, state) VALUES (?1, ?2, ?3, 'pending')",
        params![lead_id, due_at, note],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Fire due reminders. Hard silence defers them (they stay pending — the
/// user asked for them, but silence is honoured absolutely; they fire when
/// silence lifts).
pub fn fire_due_reminders(app: &AppHandle) {
    let dbs = app.state::<DbState>();
    let due: Vec<(i64, i64, Option<String>)> = {
        let conn = dbs.0.lock().unwrap();
        if setting_or_default(&conn, "hard_silence") == "1" {
            return;
        }
        let mut stmt = match conn.prepare(
            "SELECT id, lead_id, note FROM reminder WHERE state = 'pending' AND due_at <= ?1",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        stmt.query_map([db::now()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    for (id, lead_id, note) in due {
        let evidence = {
            let conn = dbs.0.lock().unwrap();
            let name: String = conn
                .query_row(
                    "SELECT COALESCE(name, email, 'a lead') FROM lead WHERE id = ?1",
                    [lead_id],
                    |r| r.get(0),
                )
                .unwrap_or_else(|_| "a lead".into());
            format!("You scheduled this follow-up for {name}.")
        };
        // Reminders are user-scheduled — solicited — so they bypass the
        // dismissal counter but still honour hard silence (checked above).
        *app.state::<SurfaceLog>()
            .0
            .lock()
            .unwrap()
            .entry("reminder".to_string())
            .or_insert(0) += 1;
        let _ = app.emit(
            "surface",
            serde_json::json!({
                "trigger_type": "reminder",
                "evidence": evidence,
                "payload": { "reminder_id": id, "lead_id": lead_id, "note": note },
            }),
        );
        let conn = dbs.0.lock().unwrap();
        let _ = conn.execute(
            "UPDATE reminder SET state = 'fired', fired_at = ?1 WHERE id = ?2",
            params![db::now(), id],
        );
    }
}

pub fn spawn_reminder_scanner(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        fire_due_reminders(&app);
    });
}

// ---------------------------------------------------------------------------
// Opt-in clipboard monitoring (§8.5: off on first run, visible when active).
// ---------------------------------------------------------------------------

pub fn spawn_clipboard_monitor(app: AppHandle) {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    std::thread::spawn(move || {
        let mut last = String::new();
        let mut announced = false;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let enabled = {
                let dbs = app.state::<DbState>();
                let conn = dbs.0.lock().unwrap();
                setting_or_default(&conn, "clipboard_monitor") == "1"
            };
            if enabled != announced {
                announced = enabled;
                let _ = app.emit("monitor-state", serde_json::json!({ "active": enabled }));
            }
            if !enabled {
                continue;
            }
            let Ok(text) = app.clipboard().read_text() else {
                continue;
            };
            if text == last || text.len() > 10_000 {
                continue;
            }
            last = text.clone();
            if let Some(email) = find_email(&text) {
                surface(
                    &app,
                    "clipboard_lead",
                    &format!("You copied text containing {email}."),
                    serde_json::json!({ "text": text }),
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct LeadSummary {
    pub id: i64,
    pub name: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub enriched: bool,
    pub drafts: i64,
    pub next_reminder: Option<String>,
}

#[tauri::command]
pub fn capture_lead(state: tauri::State<DbState>, text: String) -> Result<LeadSummary, String> {
    let conn = state.0.lock().unwrap();
    let id = insert_lead(&conn, &text, "paste").map_err(|e| e.to_string())?;
    Ok(lead_summary(&conn, id))
}

fn lead_summary(conn: &Connection, id: i64) -> LeadSummary {
    conn.query_row(
        "SELECT l.id, l.name, l.company, l.email,
                EXISTS(SELECT 1 FROM enrichment e WHERE e.lead_id = l.id),
                (SELECT COUNT(*) FROM interaction i WHERE i.lead_id = l.id AND i.kind = 'draft_email'),
                (SELECT MIN(due_at) FROM reminder r WHERE r.lead_id = l.id AND r.state = 'pending')
         FROM lead l WHERE l.id = ?1",
        [id],
        |r| {
            Ok(LeadSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                company: r.get(2)?,
                email: r.get(3)?,
                enriched: r.get(4)?,
                drafts: r.get(5)?,
                next_reminder: r.get(6)?,
            })
        },
    )
    .unwrap_or(LeadSummary {
        id,
        name: None,
        company: None,
        email: None,
        enriched: false,
        drafts: 0,
        next_reminder: None,
    })
}

#[tauri::command]
pub fn list_leads(state: tauri::State<DbState>) -> Vec<LeadSummary> {
    let conn = state.0.lock().unwrap();
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM lead ORDER BY id DESC LIMIT 50")
        .and_then(|mut s| {
            s.query_map([], |r| r.get(0))
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();
    ids.iter().map(|&id| lead_summary(&conn, id)).collect()
}

#[tauri::command]
pub fn enrich_lead(app: AppHandle, lead_id: i64) {
    std::thread::spawn(move || {
        let result = enrich(&app, lead_id);
        let _ = match result {
            Ok(notes) => app.emit("enrich-done", serde_json::json!({ "lead_id": lead_id, "notes": notes })),
            Err(e) => app.emit("loop-error", serde_json::json!({ "lead_id": lead_id, "message": e })),
        };
    });
}

#[tauri::command]
pub fn draft_lead_email(app: AppHandle, lead_id: i64) {
    std::thread::spawn(move || {
        let result = draft(&app, lead_id);
        let _ = match result {
            Ok(text) => app.emit("draft-done", serde_json::json!({ "lead_id": lead_id, "text": text })),
            Err(e) => app.emit("loop-error", serde_json::json!({ "lead_id": lead_id, "message": e })),
        };
    });
}

#[tauri::command]
pub fn remind_lead(
    state: tauri::State<DbState>,
    lead_id: i64,
    days: f64,
    note: String,
) -> Result<i64, String> {
    let due = chrono::Utc::now() + chrono::Duration::seconds((days * 86400.0) as i64);
    let conn = state.0.lock().unwrap();
    schedule_reminder(&conn, lead_id, &due.to_rfc3339(), &note).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn dismiss_surface(
    state: tauri::State<DbState>,
    trigger_type: String,
    context_hash: String,
) -> Result<(), String> {
    let conn = state.0.lock().unwrap();
    record_dismissal(&conn, &trigger_type, &context_hash).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(db::SCHEMA).unwrap();
        conn
    }

    #[test]
    fn capture_parses_name_email_company_domain() {
        let p = parse_capture("Jane Doe\nHead of Ops\njane.doe@acme.example\nAcme Ltd");
        assert_eq!(p.name.as_deref(), Some("Jane Doe"));
        assert_eq!(p.email.as_deref(), Some("jane.doe@acme.example"));
        assert_eq!(p.domain.as_deref(), Some("acme.example"));
        assert_eq!(p.company.as_deref(), Some("Acme Ltd"));
    }

    #[test]
    fn capture_falls_back_to_domain_company_and_handles_noise() {
        let p = parse_capture("contact: <bob@widgetco.io>; call later");
        assert_eq!(p.email.as_deref(), Some("bob@widgetco.io"));
        assert_eq!(p.company.as_deref(), Some("Widgetco"));
        let none = parse_capture("no contact details here at all");
        assert_eq!(none.email, None);
        // Free-mail domains are not company evidence.
        let free = parse_capture("someone@gmail.com");
        assert_eq!(free.company, None);
    }

    #[test]
    fn three_dismissals_permanently_disable_a_trigger() {
        let conn = mem();
        assert!(trigger_allowed(&conn, "clipboard_lead"));
        for _ in 0..3 {
            record_dismissal(&conn, "clipboard_lead", "h").unwrap();
        }
        assert!(!trigger_allowed(&conn, "clipboard_lead"));
        // Other trigger types are unaffected.
        assert!(trigger_allowed(&conn, "decay"));
    }

    #[test]
    fn hard_silence_blocks_every_trigger() {
        let conn = mem();
        db::setting_set(&conn, "hard_silence", "1").unwrap();
        assert!(!trigger_allowed(&conn, "clipboard_lead"));
        assert!(!trigger_allowed(&conn, "decay"));
        db::setting_set(&conn, "hard_silence", "0").unwrap();
        assert!(trigger_allowed(&conn, "clipboard_lead"));
    }

    #[test]
    fn clipboard_monitoring_is_off_by_default() {
        let conn = mem();
        assert_eq!(setting_or_default(&conn, "clipboard_monitor"), "0");
    }

    #[test]
    fn reminder_lifecycle_rows() {
        let conn = mem();
        let lead = insert_lead(&conn, "jane@acme.example", "test").unwrap();
        let past = (chrono::Utc::now() - chrono::Duration::seconds(5)).to_rfc3339();
        schedule_reminder(&conn, lead, &past, "follow up").unwrap();
        let due: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM reminder WHERE state = 'pending' AND due_at <= ?1",
                [db::now()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(due, 1);
    }
}
