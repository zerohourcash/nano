use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const SESSION_DAYS: i64 = 30;

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn create_session(conn: &Connection, user_id: i64) -> anyhow::Result<String> {
    let mut raw = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let token = URL_SAFE_NO_PAD.encode(raw);
    let now = Utc::now();
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?1,?2,?3,?4)",
        params![
            token_hash(&token),
            user_id,
            now.to_rfc3339(),
            (now + Duration::days(SESSION_DAYS)).to_rfc3339()
        ],
    )?;
    Ok(token)
}

pub fn resolve_session(conn: &Connection, token: Option<&str>) -> Option<i64> {
    let token = token?;
    if token.len() < 32 || token.len() > 128 {
        return None;
    }
    conn.query_row(
        "SELECT s.user_id FROM sessions s JOIN users u ON u.id=s.user_id
         WHERE s.token_hash=?1 AND s.revoked_at IS NULL AND s.expires_at>?2 AND u.status!='disabled'",
        params![token_hash(token), Utc::now().to_rfc3339()],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

pub fn revoke_session(conn: &Connection, token: Option<&str>) -> anyhow::Result<()> {
    if let Some(token) = token {
        conn.execute(
            "UPDATE sessions SET revoked_at=?1 WHERE token_hash=?2 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), token_hash(token)],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_tokens_without_querying() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(resolve_session(&conn, Some("short")), None);
    }

    #[test]
    fn session_is_opaque_resolvable_and_revocable() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, status TEXT NOT NULL);
             CREATE TABLE sessions (
               id INTEGER PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE,
               user_id INTEGER NOT NULL, created_at TEXT NOT NULL,
               expires_at TEXT NOT NULL, revoked_at TEXT
             );
             INSERT INTO users(id,status) VALUES (7,'active');",
        )
        .unwrap();
        let token = create_session(&conn, 7).unwrap();
        assert!(token.len() >= 40);
        let stored: String = conn
            .query_row("SELECT token_hash FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_ne!(stored, token);
        assert_eq!(resolve_session(&conn, Some(&token)), Some(7));
        revoke_session(&conn, Some(&token)).unwrap();
        assert_eq!(resolve_session(&conn, Some(&token)), None);
    }
}
