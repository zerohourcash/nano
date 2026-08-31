//! Версионированные Ed25519 account-chain локальных узлов.
//! V2 использует глобальные GUID и включает доказательство устройства.

use anyhow::{anyhow, bail, Context};
use base64::{
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
    Engine,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const KEY_NAME: &str = "ledger.node-signing-key.v1";
const DOMAIN_V1: &str = "everyday/ledger-event/v1";
const DOMAIN_V2: &str = "everyday/ledger-event/v2";

#[derive(Debug, Serialize)]
struct EventV1<'a> {
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

#[derive(Debug, Serialize)]
struct EventV2<'a> {
    domain: &'static str,
    workspace_guid: &'a str,
    actor_guid: &'a str,
    item_guid: Option<&'a str>,
    op_type: &'a str,
    from_label: Option<&'a str>,
    to_label: Option<&'a str>,
    quantity_delta: Option<f64>,
    comment: Option<&'a str>,
    created_at: &'a str,
    nonce: &'a str,
    prev_hash: Option<&'a str>,
    request_device_id: Option<&'a str>,
    request_public_key: Option<&'a str>,
    request_nonce: Option<&'a str>,
    request_signature: Option<&'a str>,
    request_hash: Option<&'a str>,
    request_timestamp: Option<&'a str>,
    request_path: Option<&'a str>,
}

#[derive(Default)]
struct RequestProof {
    device_id: Option<String>,
    public_key: Option<String>,
    nonce: Option<String>,
    signature: Option<String>,
    hash: Option<String>,
    timestamp: Option<String>,
    path: Option<String>,
}

fn signing_key(conn: &Connection) -> anyhow::Result<SigningKey> {
    let saved: Option<String> = conn
        .query_row("SELECT v FROM kv WHERE k=?1", [KEY_NAME], |r| r.get(0))
        .optional()?;
    if let Some(encoded) = saved {
        let raw = STANDARD_NO_PAD
            .decode(encoded)
            .context("invalid ledger key encoding")?;
        return Ok(SigningKey::from_bytes(
            &raw.try_into()
                .map_err(|_| anyhow!("invalid ledger signing key length"))?,
        ));
    }
    let key = SigningKey::generate(&mut OsRng);
    conn.execute(
        "INSERT INTO kv(k,v) VALUES (?1,?2)",
        params![KEY_NAME, STANDARD_NO_PAD.encode(key.to_bytes())],
    )?;
    Ok(key)
}

fn guid(conn: &Connection, table: &str, id: i64) -> anyhow::Result<String> {
    let sql = format!("SELECT guid FROM {table} WHERE id=?1");
    if let Some(value) = conn
        .query_row(&sql, [id], |r| r.get::<_, Option<String>>(0))?
        .filter(|value| !value.is_empty())
    {
        return Ok(value);
    }
    let generated = uuid::Uuid::new_v4().to_string();
    let update = format!("UPDATE {table} SET guid=?1 WHERE id=?2");
    if conn.execute(&update, params![generated, id])? != 1 {
        bail!("cannot assign global GUID for {table}:{id}");
    }
    Ok(generated)
}

fn pending_proof(conn: &Connection, actor_id: i64) -> RequestProof {
    conn.query_row(
        "SELECT device_id,public_key,nonce,signature,request_hash,request_timestamp,request_path
         FROM pending_device_proofs WHERE user_id=?1",
        [actor_id],
        |r| {
            Ok(RequestProof {
                device_id: r.get(0)?,
                public_key: r.get(1)?,
                nonce: r.get(2)?,
                signature: r.get(3)?,
                hash: r.get(4)?,
                timestamp: r.get(5)?,
                path: r.get(6)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or_default()
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
    let prev_hash: Option<String> = conn.query_row(
        "SELECT hash FROM history_entries WHERE workspace_id=?1 AND pubkey=?2 ORDER BY id DESC LIMIT 1",
        params![workspace_id,pubkey], |r| r.get(0),
    ).optional()?;
    let workspace_guid = guid(conn, "workspaces", workspace_id)?;
    let actor_guid = guid(conn, "users", actor_id)?;
    let item_guid = item_id.map(|id| guid(conn, "items", id)).transpose()?;
    let proof = pending_proof(conn, actor_id);
    let event = EventV2 {
        domain: DOMAIN_V2,
        workspace_guid: &workspace_guid,
        actor_guid: &actor_guid,
        item_guid: item_guid.as_deref(),
        op_type,
        from_label,
        to_label,
        quantity_delta,
        comment,
        created_at: &ts,
        nonce: &nonce,
        prev_hash: prev_hash.as_deref(),
        request_device_id: proof.device_id.as_deref(),
        request_public_key: proof.public_key.as_deref(),
        request_nonce: proof.nonce.as_deref(),
        request_signature: proof.signature.as_deref(),
        request_hash: proof.hash.as_deref(),
        request_timestamp: proof.timestamp.as_deref(),
        request_path: proof.path.as_deref(),
    };
    let bytes = serde_json::to_vec(&event)?;
    let hash = digest(&bytes);
    let signature = STANDARD_NO_PAD.encode(key.sign(&bytes).to_bytes());
    conn.execute(
        "INSERT INTO history_entries(workspace_id,item_id,type,actor_user_id,from_label,to_label,quantity_delta,comment,prev_hash,hash,signature,pubkey,event_version,request_device_id,request_public_key,request_nonce,request_signature,request_hash,request_timestamp,request_path,created_at,guid)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,2,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
        params![workspace_id,item_id,op_type,actor_id,from_label,to_label,quantity_delta,comment,
            prev_hash,hash,signature,pubkey,proof.device_id,proof.public_key,proof.nonce,proof.signature,
            proof.hash,proof.timestamp,proof.path,ts,nonce],
    )?;
    Ok(json!({
        "id":conn.last_insert_rowid(),"opId":hash,"workspaceId":workspace_id,
        "itemId":item_id,"type":op_type,"actorUserId":actor_id,"fromLabel":from_label,
        "toLabel":to_label,"quantityDelta":quantity_delta,"comment":comment,
        "prevHash":prev_hash,"signature":signature,"pubkey":pubkey,"eventVersion":2,
        "requestDeviceId":proof.device_id,"requestNonce":proof.nonce,
        "requestPublicKey":proof.public_key,
        "requestSignature":proof.signature,"requestHash":proof.hash,
        "requestTimestamp":proof.timestamp,"requestPath":proof.path,
        "createdAt":ts,"guid":nonce
    }))
}

fn verify_node_signature(pubkey: &str, signature: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let raw = STANDARD_NO_PAD.decode(pubkey)?;
    let key = VerifyingKey::from_bytes(
        &raw.try_into()
            .map_err(|_| anyhow!("invalid ledger public key"))?,
    )?;
    let sig = Signature::from_slice(&STANDARD_NO_PAD.decode(signature)?)?;
    key.verify(bytes, &sig).context("invalid ledger signature")
}

fn optional_string(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<String>> {
    row.get(index)
}

fn verify_device_proof(row: &Row<'_>) -> anyhow::Result<()> {
    let fields = (
        optional_string(row, 15)?,
        optional_string(row, 16)?,
        optional_string(row, 17)?,
        optional_string(row, 18)?,
        optional_string(row, 19)?,
        optional_string(row, 20)?,
        optional_string(row, 21)?,
    );
    if [
        &fields.0, &fields.1, &fields.2, &fields.3, &fields.4, &fields.5, &fields.6,
    ]
    .iter()
    .all(|field| field.is_none())
    {
        return Ok(());
    }
    let (
        Some(_device_id),
        Some(public_key),
        Some(nonce),
        Some(signature),
        Some(hash),
        Some(timestamp),
        Some(path),
    ) = fields
    else {
        bail!("incomplete device proof");
    };
    let message = format!("everyday/device-request/v1\nPOST\n{path}\n{timestamp}\n{nonce}\n{hash}");
    let raw = URL_SAFE_NO_PAD.decode(public_key)?;
    let key = VerifyingKey::from_bytes(
        &raw.try_into()
            .map_err(|_| anyhow!("invalid device public key"))?,
    )?;
    let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(signature)?)?;
    key.verify(message.as_bytes(), &signature)
        .context("invalid device proof")
}

pub fn verify_all(conn: &Connection) -> anyhow::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT workspace_id,item_id,type,actor_user_id,from_label,to_label,quantity_delta,comment,
                prev_hash,hash,signature,pubkey,created_at,guid,event_version,
                request_device_id,request_public_key,request_nonce,request_signature,request_hash,request_timestamp,request_path
         FROM history_entries ORDER BY workspace_id,id",
    )?;
    let mut rows = stmt.query([])?;
    let mut previous: HashMap<(i64, String), String> = HashMap::new();
    let mut verified = 0;
    while let Some(row) = rows.next()? {
        let ws: i64 = row.get(0)?;
        let signature: Option<String> = row.get(10)?;
        let pubkey: Option<String> = row.get(11)?;
        let (Some(signature), Some(pubkey)) = (signature, pubkey) else {
            continue;
        };
        let chain = (ws, pubkey.clone());
        let stored_prev: Option<String> = row.get(8)?;
        if stored_prev.as_deref() != previous.get(&chain).map(String::as_str) {
            bail!("broken ledger link in workspace {ws}");
        }
        let op_type: String = row.get(2)?;
        let from_label: Option<String> = row.get(4)?;
        let to_label: Option<String> = row.get(5)?;
        let comment: Option<String> = row.get(7)?;
        let created_at: String = row.get(12)?;
        let nonce: String = row.get(13)?;
        let version: i64 = row.get(14)?;
        let bytes = if version == 1 {
            serde_json::to_vec(&EventV1 {
                domain: DOMAIN_V1,
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
            })?
        } else if version == 2 {
            let workspace_guid = guid(conn, "workspaces", ws)?;
            let actor: i64 = row.get(3)?;
            let actor_guid = guid(conn, "users", actor)?;
            let item_id: Option<i64> = row.get(1)?;
            let item_guid = item_id.map(|id| guid(conn, "items", id)).transpose()?;
            let device_id = optional_string(row, 15)?;
            let request_public_key = optional_string(row, 16)?;
            let request_nonce = optional_string(row, 17)?;
            let request_signature = optional_string(row, 18)?;
            let request_hash = optional_string(row, 19)?;
            let request_timestamp = optional_string(row, 20)?;
            let request_path = optional_string(row, 21)?;
            serde_json::to_vec(&EventV2 {
                domain: DOMAIN_V2,
                workspace_guid: &workspace_guid,
                actor_guid: &actor_guid,
                item_guid: item_guid.as_deref(),
                op_type: &op_type,
                from_label: from_label.as_deref(),
                to_label: to_label.as_deref(),
                quantity_delta: row.get(6)?,
                comment: comment.as_deref(),
                created_at: &created_at,
                nonce: &nonce,
                prev_hash: stored_prev.as_deref(),
                request_device_id: device_id.as_deref(),
                request_public_key: request_public_key.as_deref(),
                request_nonce: request_nonce.as_deref(),
                request_signature: request_signature.as_deref(),
                request_hash: request_hash.as_deref(),
                request_timestamp: request_timestamp.as_deref(),
                request_path: request_path.as_deref(),
            })?
        } else {
            bail!("unsupported ledger event version {version}");
        };
        let stored_hash: String = row.get(9)?;
        if digest(&bytes) != stored_hash {
            bail!("ledger hash mismatch in workspace {ws}");
        }
        verify_node_signature(&pubkey, &signature, &bytes)?;
        if version == 2 {
            verify_device_proof(row)?;
        }
        previous.insert(chain, stored_hash);
        verified += 1;
    }
    Ok(verified)
}

pub fn verify_chat_links(conn: &Connection) -> anyhow::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT m.guid,m.workspace_id,m.user_id,m.text,m.ledger_hash,
                h.workspace_id,h.actor_user_id,h.type,h.from_label,h.comment
         FROM chat_messages m
         LEFT JOIN history_entries h ON h.hash=m.ledger_hash
         WHERE m.ledger_hash IS NOT NULL",
    )?;
    let mut rows = stmt.query([])?;
    let mut verified = 0;
    while let Some(row) = rows.next()? {
        let guid: String = row.get(0)?;
        let message_workspace: i64 = row.get(1)?;
        let message_user: i64 = row.get(2)?;
        let text: String = row.get(3)?;
        let ledger_hash: String = row.get(4)?;
        let event_workspace: Option<i64> = row.get(5)?;
        let event_user: Option<i64> = row.get(6)?;
        let event_type: Option<String> = row.get(7)?;
        let event_guid: Option<String> = row.get(8)?;
        let event_text: Option<String> = row.get(9)?;
        if event_workspace != Some(message_workspace)
            || event_user != Some(message_user)
            || event_type.as_deref() != Some("chat_message")
            || event_guid.as_deref() != Some(guid.as_str())
            || event_text.as_deref() != Some(text.as_str())
        {
            bail!("chat message {guid} is not bound to ledger event {ledger_hash}");
        }
        verified += 1;
    }
    Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE kv(k TEXT PRIMARY KEY,v TEXT NOT NULL);
             CREATE TABLE workspaces(id INTEGER PRIMARY KEY,guid TEXT); INSERT INTO workspaces VALUES(1,'ws-guid');
             CREATE TABLE users(id INTEGER PRIMARY KEY,guid TEXT); INSERT INTO users VALUES(7,'user-guid');
             CREATE TABLE items(id INTEGER PRIMARY KEY,guid TEXT); INSERT INTO items VALUES(4,'item-guid');
             CREATE TABLE pending_device_proofs(user_id INTEGER PRIMARY KEY,device_id TEXT,public_key TEXT,nonce TEXT,signature TEXT,request_hash TEXT,request_timestamp TEXT,request_path TEXT);
             CREATE TABLE history_entries(id INTEGER PRIMARY KEY,workspace_id INTEGER NOT NULL,item_id INTEGER,type TEXT NOT NULL,actor_user_id INTEGER NOT NULL,from_label TEXT,to_label TEXT,quantity_delta REAL,comment TEXT,prev_hash TEXT,hash TEXT NOT NULL UNIQUE,signature TEXT,pubkey TEXT,event_version INTEGER NOT NULL DEFAULT 1,request_device_id TEXT,request_public_key TEXT,request_nonce TEXT,request_signature TEXT,request_hash TEXT,request_timestamp TEXT,request_path TEXT,created_at TEXT NOT NULL,guid TEXT);"
        ).unwrap();
        db
    }

    #[test]
    fn v2_chain_verifies_and_detects_tampering() {
        let db = database();
        let device_key = SigningKey::generate(&mut OsRng);
        let request_nonce = "request-nonce";
        let request_hash = "body-hash";
        let timestamp = "123";
        let path = "/take";
        let message = format!(
            "everyday/device-request/v1\nPOST\n{path}\n{timestamp}\n{request_nonce}\n{request_hash}"
        );
        let device_signature =
            URL_SAFE_NO_PAD.encode(device_key.sign(message.as_bytes()).to_bytes());
        db.execute(
            "INSERT INTO pending_device_proofs VALUES(7,'phone',?1,?2,?3,?4,?5,?6)",
            params![
                URL_SAFE_NO_PAD.encode(device_key.verifying_key().to_bytes()),
                request_nonce,
                device_signature,
                request_hash,
                timestamp,
                path
            ],
        )
        .unwrap();
        let event = append(
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
        assert_eq!(event["eventVersion"], 2);
        assert_eq!(event["requestDeviceId"], "phone");
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
            "UPDATE history_entries SET request_hash='tampered' WHERE id=1",
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
        append(&db, 1, 7, None, "one", None, None, None, None).unwrap();
        append(&db, 1, 7, None, "two", None, None, None, None).unwrap();
        db.execute("DELETE FROM history_entries WHERE id=1", [])
            .unwrap();
        assert!(verify_all(&db)
            .unwrap_err()
            .to_string()
            .contains("broken ledger link"));
    }
}
