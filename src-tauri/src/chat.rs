//! Chat orchestration: settings, keychain, budget meter, egress log, and the
//! single chokepoint every model call goes through (`run_chat`). Spend cap and
//! egress row happen here or not at all.
use rusqlite::Connection;
use serde::Serialize;
use std::sync::Mutex;

use crate::db;
use crate::providers::{
    detect_claude_code, AnthropicProvider, ChatOutcome, ChatRequest, ClaudeCodeProvider, Msg,
    OllamaProvider, OpenAiProvider, Provider,
};

pub struct DbState(pub Mutex<Connection>);

/// Seed prices, USD per million tokens, editable in settings. 2026-07-16.
pub const DEFAULT_PRICES_JSON: &str = r#"{
  "claude-opus-4-8": {"in": 5.0, "out": 25.0},
  "claude-opus-4-7": {"in": 5.0, "out": 25.0},
  "claude-sonnet-5": {"in": 3.0, "out": 15.0},
  "claude-sonnet-4-6": {"in": 3.0, "out": 15.0},
  "claude-haiku-4-5": {"in": 1.0, "out": 5.0},
  "claude-fable-5": {"in": 10.0, "out": 50.0},
  "gpt-5.5": {"in": 5.0, "out": 30.0},
  "gpt-5.4": {"in": 2.5, "out": 15.0},
  "gpt-5.4-mini": {"in": 0.75, "out": 4.5},
  "gpt-5.4-nano": {"in": 0.2, "out": 1.25}
}"#;

const SETTING_DEFAULTS: [(&str, &str); 13] = [
    ("provider", "anthropic"),
    ("anthropic_model", "claude-opus-4-8"),
    ("openai_model", "gpt-5.4"),
    ("ollama_model", "llama3.2"),
    ("anthropic_url", "https://api.anthropic.com"),
    ("openai_url", "https://api.openai.com"),
    ("ollama_url", "http://localhost:11434"),
    ("cap_enabled", "1"),
    ("cap_pence", "500"),
    ("fx_gbp_per_usd", "0.79"),
    ("max_tokens", "1024"),
    ("prices_json", DEFAULT_PRICES_JSON),
    ("clipboard_monitor", "0"),
];

pub fn setting_or_default(conn: &Connection, key: &str) -> String {
    db::setting_get(conn, key).unwrap_or_else(|| {
        SETTING_DEFAULTS
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
            .unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------
// Keychain — Windows Credential Manager. Keys never touch the DB or disk.
// ---------------------------------------------------------------------------

fn keyring_service() -> &'static str {
    if std::env::var("OCELLUM_TEST").as_deref() == Ok("1") {
        "ocellum-test"
    } else {
        "ocellum"
    }
}

pub fn store_key(provider: &str, key: &str) -> Result<(), String> {
    keyring::Entry::new(keyring_service(), &format!("{provider}_api_key"))
        .and_then(|e| e.set_password(key))
        .map_err(|e| format!("keychain store failed: {e}"))
}

pub fn read_key(provider: &str) -> Option<String> {
    keyring::Entry::new(keyring_service(), &format!("{provider}_api_key"))
        .ok()?
        .get_password()
        .ok()
}

pub fn remove_key(provider: &str) -> Result<(), String> {
    keyring::Entry::new(keyring_service(), &format!("{provider}_api_key"))
        .and_then(|e| e.delete_credential())
        .map_err(|e| format!("keychain delete failed: {e}"))
}

// ---------------------------------------------------------------------------
// Cost
// ---------------------------------------------------------------------------

pub fn cost_pence(
    prices_json: &str,
    fx_gbp_per_usd: f64,
    model: &str,
    tokens_in: u64,
    tokens_out: u64,
) -> Option<f64> {
    let prices: serde_json::Value = serde_json::from_str(prices_json).ok()?;
    let entry = prices.get(model)?;
    let usd = (tokens_in as f64 / 1e6) * entry["in"].as_f64()?
        + (tokens_out as f64 / 1e6) * entry["out"].as_f64()?;
    Some(usd * fx_gbp_per_usd * 100.0)
}

// ---------------------------------------------------------------------------
// Provider construction from settings + keychain
// ---------------------------------------------------------------------------

pub fn current_provider(conn: &Connection) -> Result<Box<dyn Provider>, String> {
    match setting_or_default(conn, "provider").as_str() {
        "anthropic" => {
            let key = read_key("anthropic").ok_or(
                "No Anthropic API key set. Add one in Settings — you need a console.anthropic.com \
                 account (separate from a Claude subscription).",
            )?;
            Ok(Box::new(AnthropicProvider {
                api_key: key,
                model: setting_or_default(conn, "anthropic_model"),
                base_url: setting_or_default(conn, "anthropic_url"),
            }))
        }
        "openai" => {
            let key = read_key("openai").ok_or(
                "No OpenAI API key set. Add one in Settings — you need a platform.openai.com \
                 account (separate from ChatGPT Plus).",
            )?;
            Ok(Box::new(OpenAiProvider {
                api_key: key,
                model: setting_or_default(conn, "openai_model"),
                base_url: setting_or_default(conn, "openai_url"),
            }))
        }
        "ollama" => Ok(Box::new(OllamaProvider {
            model: setting_or_default(conn, "ollama_model"),
            base_url: setting_or_default(conn, "ollama_url"),
        })),
        "claude_code" => {
            let exe = detect_claude_code()
                .ok_or("Claude Code not found on PATH. Install it, or pick another provider.")?;
            Ok(Box::new(ClaudeCodeProvider { exe }))
        }
        other => Err(format!("unknown provider: {other}")),
    }
}

// ---------------------------------------------------------------------------
// The chokepoint. Cap check → egress row → call → cost ledger.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ChatResult {
    pub outcome: ChatOutcome,
    pub cost_pence: Option<f64>,
}

pub fn run_chat(
    conn: &Mutex<Connection>,
    provider: &dyn Provider,
    feature: &str,
    req: &ChatRequest,
    on_delta: &mut dyn FnMut(&str),
) -> Result<ChatResult, String> {
    let is_metered = matches!(provider.name(), "anthropic" | "openai");
    let (destination, request_bytes, fx, prices) = {
        let conn = conn.lock().unwrap();
        if is_metered && setting_or_default(&conn, "cap_enabled") == "1" {
            let cap: f64 = setting_or_default(&conn, "cap_pence").parse().unwrap_or(500.0);
            let spent = db::month_spend_pence(&conn);
            if spent >= cap {
                return Err(format!(
                    "Monthly spend cap reached (£{:.2} of £{:.2}). Raise or disable it in Settings.",
                    spent / 100.0,
                    cap / 100.0
                ));
            }
        }
        let body = serde_json::json!({"system": req.system, "messages": req.messages});
        let bytes = serde_json::to_string(&body).map(|s| s.len()).unwrap_or(0);
        let dest = provider.destination();
        // The row goes in at send time — a failed call still egressed.
        db::insert_egress(&conn, &dest, bytes, feature).map_err(|e| e.to_string())?;
        (
            dest,
            bytes,
            setting_or_default(&conn, "fx_gbp_per_usd").parse::<f64>().unwrap_or(0.79),
            setting_or_default(&conn, "prices_json"),
        )
    };
    let _ = (destination, request_bytes);

    let outcome = provider.stream(req, on_delta)?;

    let cost = match outcome.reported_cost_usd {
        Some(usd) => Some(usd * fx * 100.0),
        None if is_metered => cost_pence(
            &prices,
            fx,
            &provider.model(),
            outcome.usage.input_tokens,
            outcome.usage.output_tokens,
        ),
        None => None, // ollama: free, no cost row value
    };
    {
        let conn = conn.lock().unwrap();
        db::insert_model_call(
            &conn,
            feature,
            provider.name(),
            &provider.model(),
            outcome.usage.input_tokens,
            outcome.usage.output_tokens,
            cost,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(ChatResult {
        outcome,
        cost_pence: cost,
    })
}

// ---------------------------------------------------------------------------
// Budget summary — shape differs by mode, by design (§5). Claude Code mode
// reports Ocellum's own usage only; there is no remaining-limit field to
// display because the CLI does not expose one. Do not invent it.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(tag = "mode")]
pub enum BudgetSummary {
    #[serde(rename = "api_key")]
    ApiKey {
        month_pence: f64,
        cap_pence: f64,
        cap_enabled: bool,
        by_feature: Vec<db::FeatureSpend>,
    },
    #[serde(rename = "claude_code")]
    ClaudeCode {
        calls: i64,
        tokens_in: i64,
        tokens_out: i64,
        note: String,
    },
    #[serde(rename = "free")]
    Free { calls: i64 },
}

pub fn budget_summary(conn: &Connection) -> BudgetSummary {
    match setting_or_default(conn, "provider").as_str() {
        "claude_code" => {
            let u = db::month_usage(conn);
            BudgetSummary::ClaudeCode {
                calls: u.calls,
                tokens_in: u.tokens_in,
                tokens_out: u.tokens_out,
                note: "Ocellum's own usage this month — not your remaining Claude Code limit. \
                       Ocellum shares your Claude Code rate limit."
                    .into(),
            }
        }
        "ollama" => BudgetSummary::Free {
            calls: db::month_usage(conn).calls,
        },
        _ => BudgetSummary::ApiKey {
            month_pence: db::month_spend_pence(conn),
            cap_pence: setting_or_default(conn, "cap_pence").parse().unwrap_or(500.0),
            cap_enabled: setting_or_default(conn, "cap_enabled") == "1",
            by_feature: db::month_spend_by_feature(conn),
        },
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

const SETTABLE_KEYS: [&str; 13] = [
    "provider",
    "anthropic_model",
    "openai_model",
    "ollama_model",
    "anthropic_url",
    "openai_url",
    "ollama_url",
    "cap_enabled",
    "cap_pence",
    "fx_gbp_per_usd",
    "max_tokens",
    "prices_json",
    "clipboard_monitor",
];

#[tauri::command]
pub fn get_settings(state: tauri::State<DbState>) -> serde_json::Value {
    let conn = state.0.lock().unwrap();
    let mut out = serde_json::Map::new();
    for (key, _) in SETTING_DEFAULTS {
        out.insert(key.into(), setting_or_default(&conn, key).into());
    }
    out.insert("has_anthropic_key".into(), read_key("anthropic").is_some().into());
    out.insert("has_openai_key".into(), read_key("openai").is_some().into());
    out.insert(
        "claude_code_path".into(),
        detect_claude_code().map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
    );
    serde_json::Value::Object(out)
}

#[tauri::command]
pub fn set_setting(state: tauri::State<DbState>, key: String, value: String) -> Result<(), String> {
    if !SETTABLE_KEYS.contains(&key.as_str()) {
        return Err(format!("not a settable key: {key}"));
    }
    let conn = state.0.lock().unwrap();
    db::setting_set(&conn, &key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_api_key(provider: String, key: String) -> Result<(), String> {
    if !matches!(provider.as_str(), "anthropic" | "openai") {
        return Err("keys are only stored for anthropic/openai".into());
    }
    store_key(&provider, &key)
}

#[tauri::command]
pub fn delete_api_key(provider: String) -> Result<(), String> {
    remove_key(&provider)
}

#[tauri::command]
pub fn get_budget(state: tauri::State<DbState>) -> BudgetSummary {
    budget_summary(&state.0.lock().unwrap())
}

#[tauri::command]
pub fn get_egress_log(state: tauri::State<DbState>, limit: i64) -> Vec<db::EgressRow> {
    db::egress_list(&state.0.lock().unwrap(), limit)
}

#[derive(Default)]
pub struct ChatState {
    pub history: Mutex<Vec<Msg>>,
    pub last_response: Mutex<String>,
}

#[tauri::command]
pub fn chat_send(app: tauri::AppHandle, text: String) {
    use tauri::{Emitter, Manager};
    std::thread::spawn(move || {
        let chat_state = app.state::<ChatState>();
        let dbs = app.state::<DbState>();
        let (provider, max_tokens) = {
            let conn = dbs.0.lock().unwrap();
            let max_tokens = setting_or_default(&conn, "max_tokens").parse().unwrap_or(1024);
            match current_provider(&conn) {
                Ok(p) => (p, max_tokens),
                Err(e) => {
                    let _ = app.emit("chat-error", serde_json::json!({ "message": e }));
                    return;
                }
            }
        };
        let messages = {
            let mut h = chat_state.history.lock().unwrap();
            h.push(Msg {
                role: "user".into(),
                content: text,
            });
            h.clone()
        };
        let req = ChatRequest {
            system: crate::prompt::build_system_prompt(&[]),
            messages,
            max_tokens,
            web_search: false,
        };
        let mut on_delta = |d: &str| {
            let _ = app.emit("chat-delta", serde_json::json!({ "text": d }));
        };
        match run_chat(&dbs.0, provider.as_ref(), "chat", &req, &mut on_delta) {
            Ok(result) => {
                chat_state.history.lock().unwrap().push(Msg {
                    role: "assistant".into(),
                    content: result.outcome.text.clone(),
                });
                *chat_state.last_response.lock().unwrap() = result.outcome.text.clone();
                let _ = app.emit(
                    "chat-done",
                    serde_json::json!({
                        "text": result.outcome.text,
                        "tokens_in": result.outcome.usage.input_tokens,
                        "tokens_out": result.outcome.usage.output_tokens,
                        "cost_pence": result.cost_pence,
                    }),
                );
            }
            Err(e) => {
                // Drop the failed user turn so a retry doesn't double it.
                chat_state.history.lock().unwrap().pop();
                let _ = app.emit("chat-error", serde_json::json!({ "message": e }));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_is_calculated_to_the_penny() {
        // 100k in + 50k out on opus 4.8: (0.1*5 + 0.05*25) USD = 1.75 USD
        // at 0.79 GBP/USD = 138.25 pence.
        let p = cost_pence(DEFAULT_PRICES_JSON, 0.79, "claude-opus-4-8", 100_000, 50_000).unwrap();
        assert!((p - 138.25).abs() < 1e-9, "got {p}");
        assert!(cost_pence(DEFAULT_PRICES_JSON, 0.79, "unknown-model", 1, 1).is_none());
    }

    #[test]
    fn spend_cap_stops_calls_and_egress_parity_holds() {
        use crate::providers::{Msg, Usage};
        struct FakeProvider;
        impl Provider for FakeProvider {
            fn name(&self) -> &'static str {
                "anthropic"
            }
            fn destination(&self) -> String {
                "fake.example".into()
            }
            fn model(&self) -> String {
                "claude-opus-4-8".into()
            }
            fn stream(
                &self,
                _req: &ChatRequest,
                on_delta: &mut dyn FnMut(&str),
            ) -> Result<ChatOutcome, String> {
                on_delta("ok");
                Ok(ChatOutcome {
                    text: "ok".into(),
                    usage: Usage {
                        input_tokens: 1_000_000, // £3.95 per call at seed prices
                        output_tokens: 0,
                    },
                    reported_cost_usd: None,
                })
            }
        }
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(db::SCHEMA).unwrap();
        db::setting_set(&conn, "cap_enabled", "1").unwrap();
        db::setting_set(&conn, "cap_pence", "700").unwrap();
        let conn = Mutex::new(conn);
        let req = ChatRequest {
            system: "s".into(),
            messages: vec![Msg {
                role: "user".into(),
                content: "hi".into(),
            }],
            max_tokens: 10,
            web_search: false,
        };

        // Two calls fit under the £7 cap (0, then 395 pence spent)…
        for _ in 0..2 {
            run_chat(&conn, &FakeProvider, "chat", &req, &mut |_| {}).unwrap();
        }
        // …the third finds 790 ≥ 700 and must be refused.
        let err = run_chat(&conn, &FakeProvider, "chat", &req, &mut |_| {}).unwrap_err();
        assert!(err.contains("cap"), "unexpected error: {err}");

        let c = conn.lock().unwrap();
        // Egress parity: 2 calls actually left the machine, exactly 2 rows.
        assert_eq!(db::egress_count(&c), 2);
        assert_eq!(db::month_usage(&c).calls, 2);
    }

    #[test]
    fn claude_code_budget_reports_own_usage_and_no_remaining_figure() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(db::SCHEMA).unwrap();
        db::setting_set(&conn, "provider", "claude_code").unwrap();
        db::insert_model_call(&conn, "chat", "claude_code", "m", 10, 20, None).unwrap();
        let json = serde_json::to_value(budget_summary(&conn)).unwrap();
        assert_eq!(json["mode"], "claude_code");
        assert_eq!(json["calls"], 1);
        // Own-usage fields only — no invented remaining-limit figure. The
        // note *label* may mention limits (the brief requires the warning);
        // the data must not.
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["calls", "mode", "note", "tokens_in", "tokens_out"]);
    }

    #[test]
    #[ignore] // touches the real Windows Credential Manager; run via gate
    fn keychain_roundtrip_no_disk() {
        std::env::set_var("OCELLUM_TEST", "1");
        let secret = "OCELLUM-TEST-SECRET-9f3a71";
        store_key("anthropic", secret).unwrap();
        assert_eq!(read_key("anthropic").as_deref(), Some(secret));
        remove_key("anthropic").unwrap();
        assert!(read_key("anthropic").is_none());
    }
}
