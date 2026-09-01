//! Signed, tenant-bound inventory labels. A QR is an identifier, not a bearer
//! credential: cloning a genuine label is still possible and physical item
//! details must be checked. V2 prevents fabrication or cross-tenant rebinding.

use anyhow::{anyhow, bail, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const PREFIX: &str = "everyday:item:v2:";
const DOMAIN: &str = "everyday/item-label/v2";
const MAX_LABEL_BYTES: usize = 2048;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LabelPayload {
    domain: String,
    version: u8,
    workspace_guid: String,
    item_guid: String,
    internal_id: String,
    issued_at: String,
    public_key: String,
}

#[derive(Debug)]
pub struct VerifiedLabel {
    pub item_guid: String,
    pub workspace_guid: String,
    pub internal_id: String,
    pub issued_at: String,
    pub public_key: String,
}

fn decode_public_key(value: &str) -> Result<[u8; 32]> {
    let raw = URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| STANDARD_NO_PAD.decode(value))
        .or_else(|_| STANDARD.decode(value))?;
    raw.try_into().map_err(|_| anyhow!("invalid QR public key"))
}

fn canonical_bytes(payload: &LabelPayload) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(payload)?)
}

pub fn issue(conn: &Connection, item_id: i64) -> Result<String> {
    let (workspace_id, internal_id): (i64, String) = conn
        .query_row(
            "SELECT i.workspace_id,i.internal_id FROM items i WHERE i.id=?1 AND i.archived=0",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("item not found for QR label")?;
    let workspace_guid = crate::ledger::guid(conn, "workspaces", workspace_id)?;
    let item_guid = crate::ledger::guid(conn, "items", item_id)?;
    let key = crate::ledger::signing_key(conn)?;
    let payload = LabelPayload {
        domain: DOMAIN.into(),
        version: 2,
        workspace_guid,
        item_guid,
        internal_id,
        issued_at: Utc::now().to_rfc3339(),
        public_key: URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes()),
    };
    let bytes = canonical_bytes(&payload)?;
    let signature = key.sign(&bytes);
    Ok(format!(
        "{PREFIX}{}.{}",
        URL_SAFE_NO_PAD.encode(bytes),
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

pub fn verify(conn: &Connection, label: &str) -> Result<VerifiedLabel> {
    let encoded = label
        .trim()
        .strip_prefix(PREFIX)
        .context("not a signed Everyday item label")?;
    if encoded.len() > MAX_LABEL_BYTES {
        bail!("signed QR label is too large")
    }
    let (payload_raw, signature_raw) =
        encoded.split_once('.').context("invalid signed QR label")?;
    if signature_raw.contains('.') {
        bail!("invalid signed QR label")
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload_raw)?;
    if bytes.len() > MAX_LABEL_BYTES {
        bail!("signed QR payload is too large")
    }
    let payload: LabelPayload = serde_json::from_slice(&bytes)?;
    if payload.domain != DOMAIN
        || payload.version != 2
        || uuid::Uuid::parse_str(&payload.workspace_guid).is_err()
        || uuid::Uuid::parse_str(&payload.item_guid).is_err()
        || payload.internal_id.is_empty()
        || payload.internal_id.chars().count() > 120
        || DateTime::parse_from_rfc3339(&payload.issued_at).is_err()
    {
        bail!("invalid signed QR payload")
    }
    if canonical_bytes(&payload)? != bytes {
        bail!("non-canonical signed QR payload")
    }
    let public_key = decode_public_key(&payload.public_key)?;
    let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(signature_raw)?)?;
    VerifyingKey::from_bytes(&public_key)?
        .verify(&bytes, &signature)
        .context("invalid signed QR signature")?;

    let local_key = decode_public_key(&crate::ledger::node_public_key(conn)?)?;
    let canonical_public_key = STANDARD_NO_PAD.encode(public_key);
    let trusted = public_key == local_key
        || conn
            .query_row(
                "SELECT 1 FROM trusted_node_keys k
                 JOIN trusted_node_key_workspaces s ON s.public_key=k.public_key
                 WHERE k.public_key=?1 AND s.workspace_guid=?2",
                params![canonical_public_key, payload.workspace_guid],
                |_| Ok(()),
            )
            .is_ok();
    if !trusted {
        bail!("QR label signer is not a trusted node")
    }
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM items i JOIN workspaces w ON w.id=i.workspace_id
             WHERE i.guid=?1 AND w.guid=?2 AND i.internal_id=?3 AND i.archived=0",
            params![
                payload.item_guid,
                payload.workspace_guid,
                payload.internal_id
            ],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !exists {
        bail!("signed QR does not match the current item")
    }
    Ok(VerifiedLabel {
        item_guid: payload.item_guid,
        workspace_guid: payload.workspace_guid,
        internal_id: payload.internal_id,
        issued_at: payload.issued_at,
        public_key: payload.public_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE kv(k TEXT PRIMARY KEY,v TEXT NOT NULL);
             CREATE TABLE trusted_node_keys(public_key TEXT PRIMARY KEY,label TEXT,approved_by INTEGER,source TEXT,created_at TEXT);
             CREATE TABLE trusted_node_key_workspaces(public_key TEXT NOT NULL,workspace_guid TEXT NOT NULL,first_verified_at TEXT NOT NULL,PRIMARY KEY(public_key,workspace_guid));
             CREATE TABLE workspaces(id INTEGER PRIMARY KEY,guid TEXT NOT NULL);
             CREATE TABLE items(id INTEGER PRIMARY KEY,workspace_id INTEGER NOT NULL,guid TEXT NOT NULL,internal_id TEXT NOT NULL,archived INTEGER NOT NULL DEFAULT 0);
             INSERT INTO workspaces VALUES(1,'00000000-0000-4000-8000-000000000001');
             INSERT INTO workspaces VALUES(2,'00000000-0000-4000-8000-000000000003');
             INSERT INTO items VALUES(1,1,'00000000-0000-4000-8000-000000000002','TOOL-1',0);
             INSERT INTO items VALUES(2,2,'00000000-0000-4000-8000-000000000004','TOOL-2',0);",
        )
        .unwrap();
        let public = crate::ledger::node_public_key(&db).unwrap();
        db.execute(
            "INSERT INTO trusted_node_keys(public_key,source,created_at) VALUES(?1,'local',?2)",
            params![public, Utc::now().to_rfc3339()],
        )
        .unwrap();
        db
    }

    #[test]
    fn signed_label_rejects_unknown_signer_tamper_and_rebinding() {
        let issuer = inventory_db();
        let verifier = inventory_db();
        let label = issue(&issuer, 1).unwrap();
        let other_workspace_label = issue(&issuer, 2).unwrap();
        assert!(verify(&issuer, &label).is_ok());
        assert!(verify(&verifier, &label)
            .unwrap_err()
            .to_string()
            .contains("not a trusted node"));

        let payload_raw = label
            .strip_prefix(PREFIX)
            .unwrap()
            .split_once('.')
            .unwrap()
            .0;
        let issuer_payload_key =
            serde_json::from_slice::<LabelPayload>(&URL_SAFE_NO_PAD.decode(payload_raw).unwrap())
                .unwrap()
                .public_key;
        let issuer_key = STANDARD_NO_PAD.encode(decode_public_key(&issuer_payload_key).unwrap());
        verifier
            .execute(
                "INSERT INTO trusted_node_keys(public_key,source,created_at) VALUES(?1,'approved',?2)",
                params![issuer_key, Utc::now().to_rfc3339()],
            )
            .unwrap();
        assert!(verify(&verifier, &label)
            .unwrap_err()
            .to_string()
            .contains("not a trusted node"));
        verifier
            .execute(
                "INSERT INTO trusted_node_key_workspaces(public_key,workspace_guid,first_verified_at) VALUES(?1,'00000000-0000-4000-8000-000000000001',?2)",
                params![issuer_key, Utc::now().to_rfc3339()],
            )
            .unwrap();
        assert!(verify(&verifier, &label).is_ok());
        assert!(verify(&verifier, &other_workspace_label)
            .unwrap_err()
            .to_string()
            .contains("not a trusted node"));
        verifier
            .execute("UPDATE items SET internal_id='OTHER' WHERE id=1", [])
            .unwrap();
        assert!(verify(&verifier, &label)
            .unwrap_err()
            .to_string()
            .contains("does not match"));

        let mut forged = label.into_bytes();
        let last = forged.len() - 1;
        forged[last] = if forged[last] == b'A' { b'B' } else { b'A' };
        assert!(verify(&issuer, &String::from_utf8(forged).unwrap()).is_err());
    }
}
