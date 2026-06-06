//! Local SQLite cache of the AI text the daemon polls from the deployed Worker.
//!
//! The daemon (`CTA_DAEMON=1`) writes the three panels + a liveness heartbeat;
//! the TUI reads them so its render loop never touches the network. One tiny DB
//! at `${XDG_CACHE_HOME:-$HOME/.cache}/cta-tui/ai.db` (override with `CTA_DB`).

use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

/// One AI panel's latest text.
#[derive(Clone, Default)]
pub struct AiItem {
    pub summary: String,
    pub count: i64,
    pub updated_at: i64, // epoch seconds
}

/// The three AI panels, as last cached.
#[derive(Clone, Default)]
pub struct AiState {
    pub dispatch: AiItem,
    pub sitrep: AiItem,
    pub events: AiItem,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Resolve the DB path: `CTA_DB` override, else the XDG cache dir.
pub fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("CTA_DB") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".cache")
    });
    base.join("cta-tui").join("ai.db")
}

/// Open (creating the dir + schema). WAL mode so the daemon's writes and the
/// TUI's reads don't block each other.
pub fn open() -> Result<Connection> {
    let path = db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 3000)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ai (
            key        TEXT PRIMARY KEY,
            summary    TEXT NOT NULL,
            count      INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        )",
        [],
    )?;
    Ok(conn)
}

/// Upsert one panel's latest text.
pub fn upsert(conn: &Connection, key: &str, summary: &str, count: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO ai (key, summary, count, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET summary = ?2, count = ?3, updated_at = ?4",
        rusqlite::params![key, summary, count, now_secs()],
    )?;
    Ok(())
}

/// Bump the daemon liveness heartbeat.
pub fn touch_heartbeat(conn: &Connection) -> Result<()> {
    upsert(conn, "heartbeat", "", 0)
}

/// Age of the heartbeat in seconds (None if never written → no daemon yet).
pub fn heartbeat_age_secs(conn: &Connection) -> Option<i64> {
    conn.query_row("SELECT updated_at FROM ai WHERE key = 'heartbeat'", [], |r| r.get::<_, i64>(0))
        .ok()
        .map(|t| (now_secs() - t).max(0))
}

fn load_item(conn: &Connection, key: &str) -> AiItem {
    conn.query_row(
        "SELECT summary, count, updated_at FROM ai WHERE key = ?1",
        [key],
        |r| Ok(AiItem { summary: r.get(0)?, count: r.get(1)?, updated_at: r.get(2)? }),
    )
    .unwrap_or_default()
}

/// Read the three panels into an AiState.
pub fn read_all(conn: &Connection) -> AiState {
    AiState {
        dispatch: load_item(conn, "dispatch"),
        sitrep: load_item(conn, "sitrep"),
        events: load_item(conn, "events"),
    }
}
