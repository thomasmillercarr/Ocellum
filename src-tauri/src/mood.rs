//! Mood (§8.4). A pure function of real local data — untouched leads, drafts
//! without a second touch, days since anything happened. Derived from
//! `lead`/`interaction` on every read; never stored, never settable.
//! `mood_event` is an append-only journal of deltas, not the mood itself.
use rusqlite::Connection;

pub const FLAT_AFTER_DAYS: f64 = 14.0;
pub const RESTLESS_AFTER_DAYS: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mood {
    Bright,
    Neutral,
    Restless,
    Flat,
}

impl Mood {
    pub fn as_str(self) -> &'static str {
        match self {
            Mood::Bright => "bright",
            Mood::Neutral => "neutral",
            Mood::Restless => "restless",
            Mood::Flat => "flat",
        }
    }
}

pub fn derive_mood(conn: &Connection) -> Mood {
    // Bright: outbound draft within the last day.
    let bright: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM interaction WHERE kind = 'draft_email'
              AND julianday('now') - julianday(created_at) < 1.0)",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if bright {
        return Mood::Bright;
    }
    // Days since anything happened at all (capture or interaction).
    let days_quiet: Option<f64> = conn
        .query_row(
            "SELECT julianday('now') - MAX(julianday(t)) FROM
               (SELECT created_at AS t FROM lead
                UNION ALL SELECT created_at FROM interaction)",
            [],
            |r| r.get(0),
        )
        .unwrap_or(None);
    match days_quiet {
        // Fresh install: nothing to be flat about yet.
        None => Mood::Neutral,
        Some(d) if d >= FLAT_AFTER_DAYS => Mood::Flat,
        Some(_) => {
            // Restless: a lead you drafted once and never touched again (§7's
            // decay framing: "you drafted this and never sent a second touch").
            let restless: bool = conn
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM lead l
                       WHERE (SELECT COUNT(*) FROM interaction i
                              WHERE i.lead_id = l.id AND i.kind = 'draft_email') = 1
                         AND NOT EXISTS(
                              SELECT 1 FROM interaction later
                              WHERE later.lead_id = l.id
                                AND later.created_at > (SELECT MAX(created_at) FROM interaction d
                                                        WHERE d.lead_id = l.id AND d.kind = 'draft_email'))
                         AND julianday('now') - (SELECT julianday(MAX(created_at)) FROM interaction d
                                                 WHERE d.lead_id = l.id AND d.kind = 'draft_email') >= ?1)",
                    [RESTLESS_AFTER_DAYS],
                    |r| r.get(0),
                )
                .unwrap_or(false);
            if restless {
                Mood::Restless
            } else {
                Mood::Neutral
            }
        }
    }
}

/// Journal a mood delta. Append-only; never read back into `derive_mood`.
pub fn record_mood_delta(conn: &Connection, delta: f64, reason: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO mood_event (delta, reason, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![delta, reason, crate::db::now()],
    )?;
    Ok(())
}

#[tauri::command]
pub fn get_mood(state: tauri::State<crate::chat::DbState>) -> String {
    derive_mood(&state.0.lock().unwrap()).as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA).unwrap();
        conn
    }

    fn ago(days: f64) -> String {
        (chrono::Utc::now() - chrono::Duration::seconds((days * 86400.0) as i64)).to_rfc3339()
    }

    fn lead_at(conn: &Connection, at: &str) -> i64 {
        conn.execute(
            "INSERT INTO lead (name, created_at) VALUES ('t', ?1)",
            [at],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn draft_at(conn: &Connection, lead_id: i64, at: &str) {
        conn.execute(
            "INSERT INTO interaction (lead_id, kind, created_at) VALUES (?1, 'draft_email', ?2)",
            params![lead_id, at],
        )
        .unwrap();
    }

    #[test]
    fn fourteen_days_of_no_outbound_is_flat() {
        let conn = mem();
        let lead = lead_at(&conn, &ago(15.0));
        draft_at(&conn, lead, &ago(15.0));
        assert_eq!(derive_mood(&conn), Mood::Flat);
    }

    #[test]
    fn fresh_draft_is_bright_and_journals_a_positive_delta() {
        let conn = mem();
        let lead = lead_at(&conn, &ago(0.1));
        draft_at(&conn, lead, &ago(0.1));
        record_mood_delta(&conn, 1.0, "draft_created").unwrap();
        assert_eq!(derive_mood(&conn), Mood::Bright);
        let (n, delta): (i64, f64) = conn
            .query_row("SELECT COUNT(*), SUM(delta) FROM mood_event", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(n, 1);
        assert!(delta > 0.0);
    }

    #[test]
    fn empty_db_is_neutral_not_flat() {
        let conn = mem();
        assert_eq!(derive_mood(&conn), Mood::Neutral);
    }

    #[test]
    fn one_draft_and_silence_is_restless() {
        let conn = mem();
        let lead = lead_at(&conn, &ago(10.0));
        draft_at(&conn, lead, &ago(6.0));
        assert_eq!(derive_mood(&conn), Mood::Restless);
    }

    #[test]
    fn a_second_touch_clears_restless() {
        let conn = mem();
        let lead = lead_at(&conn, &ago(10.0));
        draft_at(&conn, lead, &ago(6.0));
        // Second touch after the draft (an enrichment refresh, another draft…).
        draft_at(&conn, lead, &ago(2.0));
        assert_eq!(derive_mood(&conn), Mood::Neutral);
    }

    #[test]
    fn recent_capture_without_drafts_is_neutral() {
        let conn = mem();
        lead_at(&conn, &ago(1.0));
        assert_eq!(derive_mood(&conn), Mood::Neutral);
    }
}
