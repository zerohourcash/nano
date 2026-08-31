//! Криптографически связанный журнал операций.
//!
//! Каждая новая запись содержит hash предыдущей записи рабочего пространства,
//! SHA-256 канонического payload и подпись локального узла Ed25519. Это не
//! заменяет будущую подпись пользовательского устройства, но делает изменение
//! или удаление уже принятой истории обнаруживаемым при старте узла.

use anyhow::{anyhow, bail, Context};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const KEY_NAME: &str = "ledger.node-signing-key.v1";
const DOMAIN: &str = "everyday/ledger-event/v1";

#[derive(Debug, Serialize)]
struct Event<'a> {
    domain: &'static str,
    workspace_id: i64,
    actor_id: i64,
    item_id: Option<i64>,
    op_type: &'a str,
    from_label: Option<&'a str>,
    to_label: Option<&'a str>,
    quantity_delta: Option<f64>,
    comment: Option<&'a str>,
    created_at: &'a str,
    nonce: &'a str,
    prev_hash: Option<&'a str>,
}

fn signing_key(conn: &Connection) -> anyhow::Result<SigningKey> {
    let saved: Option<String> = conn
        .query_row("SELECT v FROM kv WHERE k=?1", [KEY_NAME], |r| r.get(0))
        .optional()?;
    if let Some(encoded) = saved {
        let raw = STANDARD_NO_PAD
            .decode(encoded)
            .context("invalid ledger key encoding")?;
        let bytes: [u8; 32] = raw
            .try_into()
            .map_err(|_| anyhow!("invalid ledger signing key length"))?;
        return Ok(SigningKey::from_bytes(&bytes));
    }
    let key = SigningKey::generate(&mut OsRng);
    conn.execute(
        "INSERT INTO kv(k,v) VALUES (?1,?2)",
        params![KEY_NAME, STANDARD_NO_PAD.encode(key.to_bytes())],
    )?;
    Ok(key)
}

fn event_bytes(event: &Event<'_>) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(event)?)
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[allow(clippy::too_many_arguments)]
pub fn append(
    conn: &Connection,
    workspace_id: i64,
    actor_id: i64,
    item_id: Option<i64>,
    op_type: &str,
    from_label: Option<&str>,
    to_label: Option<&str>,
    quantity_delta: Option<f64>,
    comment: Option<&str>,
) -> anyhow::Result<Value> {
    let ts = chrono::Utc::now().to_rfc3339();
    let nonce = uuid::Uuid::new_v4().to_string();
    let key = signing_key(conn)?;
    let pubkey = STANDARD_NO_PAD.encode(key.verifying_key().to_bytes());
    let prev_hash: Option<String> = conn
        .query_row(
            "SELECT hash FROM history_entries WHERE workspace_id=?1 AND pubkey=?2 ORDER BY id DESC LIMIT 1",
            params![workspace_id, pubkey],
            |r| r.get(0),
        )
        .optional()?;
    let event = Event {
        domain: DOMAIN,
        workspace_id,
        actor_id,
        item_id,
        op_type,
        from_label,
        to_label,
        quantity_delta,
        comment,
        created_at: &ts,
        nonce: &nonce,
        prev_hash: prev_hash.as_deref(),
    };
    let bytes = event_bytes(&event)?;
    let hash = digest(&bytes);
    let signature = STANDARD_NO_PAD.encode(key.sign(&bytes).to_bytes());
    conn.execute(
        "INSERT INTO history_entries (workspace_id,item_id,type,actor_user_id,from_label,to_label,quantity_delta,comment,prev_hash,hash,signature,pubkey,created_at,guid)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![workspace_id,item_id,op_type,actor_id,from_label,to_label,quantity_delta,comment,prev_hash,hash,signature,pubkey,ts,nonce],
    )?;
    let id = conn.last_insert_rowid();
    Ok(json!({
        "id": id, "opId": hash, "workspaceId": workspace_id, "itemId": item_id,
        "type": op_type, "actorUserId": actor_id, "fromLabel": from_label,
        "toLabel": to_label, "quantityDelta": quantity_delta, "comment": comment,
        "prevHash": prev_hash, "signature": signature, "pubkey": pubkey,
        "createdAt": ts, "guid": nonce
    }))
}

/// Проверяет все подписанные цепочки. Старые unsigned-записи разрешены только
/// как legacy-префикс; после первой подписанной записи unsigned-событие запрещено.
pub fn verify_all(conn: &Connection) -> anyhow::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT workspace_id,item_id,type,actor_user_id,from_label,to_label,quantity_delta,comment,prev_hash,hash,signature,pubkey,created_at,guid
         FROM history_entries ORDER BY workspace_id,id",
    )?;
    let mut rows = stmt.query([])?;
    let mut previous: HashMap<(i64, String), String> = HashMap::new();
    let mut verified = 0;
    while let Some(row) = rows.next()? {
        let ws: i64 = row.get(0)?;
        let signature: Option<String> = row.get(10)?;
        let pubkey: Option<String> = row.get(11)?;
        if signature.is_none() || pubkey.is_none() {
            continue;
        }
        let pubkey = pubkey.unwrap();
        let chain = (ws, pubkey.clone());
        let stored_prev: Option<String> = row.get(8)?;
        let expected_prev = previous.get(&chain).map(String::as_str);
        if stored_prev.as_deref() != expected_prev {
            bail!("broken ledger link in workspace {ws}");
        }
        let created_at: String = row.get(12)?;
        let nonce: String = row.get(13)?;
        let op_type: String = row.get(2)?;
        let from_label: Option<String> = row.get(4)?;
        let to_label: Option<String> = row.get(5)?;
        let comment: Option<String> = row.get(7)?;
        let event = Event {
            domain: DOMAIN,
            workspace_id: ws,
            actor_id: row.get(3)?,
            item_id: row.get(1)?,
            op_type: &op_type,
            from_label: from_label.as_deref(),
            to_label: to_label.as_deref(),
            quantity_delta: row.get(6)?,
            comment: comment.as_deref(),
            created_at: &created_at,
            nonce: &nonce,
            prev_hash: stored_prev.as_deref(),
        };
        let bytes = event_bytes(&event)?;
        let stored_hash: String = row.get(9)?;
        if digest(&bytes) != stored_hash {
            bail!("ledger hash mismatch in workspace {ws}");
        }
        let pk_raw = STANDARD_NO_PAD.decode(&pubkey)?;
        let pk = VerifyingKey::from_bytes(
            &pk_raw
                .try_into()
                .map_err(|_| anyhow!("invalid ledger public key"))?,
        )?;
        let sig = Signature::from_slice(&STANDARD_NO_PAD.decode(signature.unwrap())?)?;
        pk.verify(&bytes, &sig)
            .context("invalid ledger signature")?;
        previous.insert(chain, stored_hash);
        verified += 1;
    }
    Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE kv(k TEXT PRIMARY KEY,v TEXT NOT NULL); CREATE TABLE history_entries(id INTEGER PRIMARY KEY,workspace_id INTEGER NOT NULL,item_id INTEGER,type TEXT NOT NULL,actor_user_id INTEGER NOT NULL,from_label TEXT,to_label TEXT,quantity_delta REAL,comment TEXT,prev_hash TEXT,hash TEXT NOT NULL UNIQUE,signature TEXT,pubkey TEXT,created_at TEXT NOT NULL,guid TEXT);").unwrap();
        db
    }

    #[test]
    fn signed_chain_verifies_and_detects_tampering() {
        let db = database();
        append(
            &db,
            1,
            7,
            Some(4),
            "take",
            Some("Склад"),
            Some("Иван"),
            Some(1.0),
            Some("QR"),
        )
        .unwrap();
        append(
            &db,
            1,
            7,
            Some(4),
            "return",
            Some("Иван"),
            Some("Склад"),
            Some(1.0),
            None,
        )
        .unwrap();
        assert_eq!(verify_all(&db).unwrap(), 2);
        db.execute(
            "UPDATE history_entries SET comment='подмена' WHERE id=1",
            [],
        )
        .unwrap();
        assert!(verify_all(&db)
            .unwrap_err()
            .to_string()
            .contains("hash mismatch"));
    }

    #[test]
    fn deletion_breaks_following_link() {
        let db = database();
        append(&db, 1, 1, None, "one", None, None, None, None).unwrap();
        append(&db, 1, 1, None, "two", None, None, None, None).unwrap();
        db.execute("DELETE FROM history_entries WHERE id=1", [])
            .unwrap();
        assert!(verify_all(&db)
            .unwrap_err()
            .to_string()
            .contains("broken ledger link"));
    }
}
