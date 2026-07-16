//! Local store. One SQLite file, schema per brief §7 plus two internal
//! tables: `setting` (key/value config — never credentials) and `model_call`
//! (per-call cost ledger; `interaction` is lead-bound per the brief, but the
//! budget meter must also cover lead-less calls like bubble chat).
use rusqlite::{params, Connection};
use serde::Serialize;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS lead (
    id INTEGER PRIMARY KEY,
    name TEXT,
    company TEXT,
    email TEXT,
    domain TEXT,
    source TEXT,
    raw_capture TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS enrichment (
    id INTEGER PRIMARY KEY,
    lead_id INTEGER NOT NULL REFERENCES lead(id),
    provider TEXT,
    payload_json TEXT,
    fetched_at TEXT NOT NULL,
    ttl INTEGER
);
CREATE TABLE IF NOT EXISTS interaction (
    id INTEGER PRIMARY KEY,
    lead_id INTEGER NOT NULL REFERENCES lead(id),
    kind TEXT NOT NULL CHECK (kind IN ('draft_email','voice_note','enrichment','triage','roleplay')),
    body TEXT,
    model_used TEXT,
    tokens_in INTEGER,
    tokens_out INTEGER,
    cost_pence REAL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS reminder (
    id INTEGER PRIMARY KEY,
    lead_id INTEGER NOT NULL REFERENCES lead(id),
    due_at TEXT NOT NULL,
    note TEXT,
    state TEXT NOT NULL DEFAULT 'pending',
    fired_at TEXT
);
CREATE TABLE IF NOT EXISTS dismissal (
    id INTEGER PRIMARY KEY,
    trigger_type TEXT NOT NULL,
    context_hash TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS mood_event (
    id INTEGER PRIMARY KEY,
    delta REAL NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS egress_log (
    id INTEGER PRIMARY KEY,
    destination TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    purpose TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS setting (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS model_call (
    id INTEGER PRIMARY KEY,
    feature TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    tokens_in INTEGER NOT NULL,
    tokens_out INTEGER NOT NULL,
    cost_pence REAL,
    created_at TEXT NOT NULL
);
";

pub fn open(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn month_prefix() -> String {
    chrono::Utc::now().format("%Y-%m").to_string()
}

pub fn setting_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

pub fn setting_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

/// Every byte that leaves this machine gets a row. No exceptions (§6).
pub fn insert_egress(
    conn: &Connection,
    destination: &str,
    bytes: usize,
    purpose: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO egress_log (destination, bytes, purpose, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![destination, bytes as i64, purpose, now()],
    )?;
    Ok(())
}

pub fn insert_model_call(
    conn: &Connection,
    feature: &str,
    provider: &str,
    model: &str,
    tokens_in: u64,
    tokens_out: u64,
    cost_pence: Option<f64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO model_call (feature, provider, model, tokens_in, tokens_out, cost_pence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![feature, provider, model, tokens_in as i64, tokens_out as i64, cost_pence, now()],
    )?;
    Ok(())
}

/// Spend this calendar month, pence. Unpriced calls count as zero.
pub fn month_spend_pence(conn: &Connection) -> f64 {
    conn.query_row(
        "SELECT COALESCE(SUM(cost_pence), 0) FROM model_call WHERE substr(created_at, 1, 7) = ?1",
        [month_prefix()],
        |r| r.get(0),
    )
    .unwrap_or(0.0)
}

#[derive(Serialize)]
pub struct FeatureSpend {
    pub feature: String,
    pub pence: f64,
    pub calls: i64,
}

pub fn month_spend_by_feature(conn: &Connection) -> Vec<FeatureSpend> {
    let mut stmt = match conn.prepare(
        "SELECT feature, COALESCE(SUM(cost_pence), 0), COUNT(*) FROM model_call
         WHERE substr(created_at, 1, 7) = ?1 GROUP BY feature ORDER BY 2 DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([month_prefix()], |r| {
        Ok(FeatureSpend {
            feature: r.get(0)?,
            pence: r.get(1)?,
            calls: r.get(2)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

#[derive(Serialize)]
pub struct MonthUsage {
    pub calls: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
}

pub fn month_usage(conn: &Connection) -> MonthUsage {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0)
         FROM model_call WHERE substr(created_at, 1, 7) = ?1",
        [month_prefix()],
        |r| {
            Ok(MonthUsage {
                calls: r.get(0)?,
                tokens_in: r.get(1)?,
                tokens_out: r.get(2)?,
            })
        },
    )
    .unwrap_or(MonthUsage {
        calls: 0,
        tokens_in: 0,
        tokens_out: 0,
    })
}

#[derive(Serialize)]
pub struct EgressRow {
    pub id: i64,
    pub destination: String,
    pub bytes: i64,
    pub purpose: String,
    pub created_at: String,
}

pub fn egress_list(conn: &Connection, limit: i64) -> Vec<EgressRow> {
    let mut stmt = match conn.prepare(
        "SELECT id, destination, bytes, purpose, created_at FROM egress_log
         ORDER BY id DESC LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([limit], |r| {
        Ok(EgressRow {
            id: r.get(0)?,
            destination: r.get(1)?,
            bytes: r.get(2)?,
            purpose: r.get(3)?,
            created_at: r.get(4)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

pub fn egress_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM egress_log", [], |r| r.get(0))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn settings_roundtrip_and_overwrite() {
        let conn = mem();
        assert_eq!(setting_get(&conn, "x"), None);
        setting_set(&conn, "x", "1").unwrap();
        setting_set(&conn, "x", "2").unwrap();
        assert_eq!(setting_get(&conn, "x").as_deref(), Some("2"));
    }

    #[test]
    fn egress_rows_accumulate() {
        let conn = mem();
        insert_egress(&conn, "api.anthropic.com", 100, "chat").unwrap();
        insert_egress(&conn, "api.anthropic.com", 200, "chat").unwrap();
        assert_eq!(egress_count(&conn), 2);
        let rows = egress_list(&conn, 10);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].bytes, 200); // newest first
    }

    #[test]
    fn month_spend_sums_current_month_only() {
        let conn = mem();
        insert_model_call(&conn, "chat", "anthropic", "m", 10, 20, Some(1.5)).unwrap();
        insert_model_call(&conn, "chat", "anthropic", "m", 10, 20, Some(2.0)).unwrap();
        // A row from another month must not count.
        conn.execute(
            "INSERT INTO model_call (feature, provider, model, tokens_in, tokens_out, cost_pence, created_at)
             VALUES ('chat', 'anthropic', 'm', 1, 1, 99.0, '2001-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        assert!((month_spend_pence(&conn) - 3.5).abs() < 1e-9);
        let by = month_spend_by_feature(&conn);
        assert_eq!(by.len(), 1);
        assert_eq!(by[0].calls, 2);
    }

    #[test]
    fn interaction_kind_is_a_closed_enum() {
        let conn = mem();
        conn.execute(
            "INSERT INTO lead (name, created_at) VALUES ('t', ?1)",
            [now()],
        )
        .unwrap();
        let bad = conn.execute(
            "INSERT INTO interaction (lead_id, kind, created_at) VALUES (1, 'status_update', ?1)",
            [now()],
        );
        assert!(bad.is_err(), "unknown interaction.kind must be rejected");
    }
}
