use axum::http::HeaderMap;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const DOMAIN: &str = "everyday/device-request/v1";
const MAX_CLOCK_SKEW_SECONDS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug)]
pub struct Proof {
    pub device_id: String,
    pub public_key: String,
    pub nonce: String,
    pub signature: String,
    pub request_hash: String,
    pub timestamp: String,
    pub path: String,
    pub request_body: Option<String>,
}

fn is_compact_knowledge_intent(body: &[u8]) -> bool {
    let Ok(envelope) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let input = envelope
        .get("0")
        .and_then(|value| value.get("json"))
        .or_else(|| envelope.get("json"));
    let Some(input) = input else { return false };
    if !["workspaceGuid", "pageGuid", "revisionGuid"]
        .iter()
        .all(|field| input.get(field).and_then(Value::as_str).is_some())
    {
        return false;
    }
    input
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .all(|attachment| {
            attachment
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|url| !url.starts_with("data:"))
        })
}

fn is_compact_chat_intent(body: &[u8]) -> bool {
    let Ok(envelope) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let input = envelope
        .get("0")
        .and_then(|value| value.get("json"))
        .or_else(|| envelope.get("json"));
    let Some(input) = input else { return false };
    if !["workspaceGuid", "messageGuid"]
        .iter()
        .all(|field| input.get(field).and_then(Value::as_str).is_some())
    {
        return false;
    }
    input
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .all(|attachment| {
            attachment
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|url| !url.starts_with("data:"))
        })
}

fn is_compact_writeoff_intent(body: &[u8]) -> bool {
    let Ok(envelope) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let input = envelope
        .get("0")
        .and_then(|value| value.get("json"))
        .or_else(|| envelope.get("json"));
    let Some(input) = input else { return false };
    input.get("operationGuid").and_then(Value::as_str).is_some()
        && input
            .get("photoUrl")
            .and_then(Value::as_str)
            .is_none_or(|url| url.starts_with("cas:"))
}

fn should_retain_request_body(path: &str, body: &[u8]) -> bool {
    path.starts_with("/api/trpc/inventory.")
        || matches!(
            path,
            "/api/trpc/items.addPhoto" | "/api/trpc/items.addDocument"
        )
        || (matches!(
            path,
            "/api/trpc/transfers.take" | "/api/trpc/transfers.takeMany"
        ) && body.len() <= 128 * 1024)
        || (path.starts_with("/api/trpc/interorg.") && body.len() <= 64 * 1024)
        || (path == "/api/trpc/bit.offer" && body.len() <= 16 * 1024)
        || (matches!(
            path,
            "/api/trpc/bit.transfer" | "/api/trpc/bit.sale" | "/api/trpc/bit.mint"
        ) && body.len() <= 16 * 1024)
        || (path == "/api/trpc/history.writeOff"
            && body.len() <= 16 * 1024
            && is_compact_writeoff_intent(body))
        || (matches!(
            path,
            "/api/trpc/history.replenish" | "/api/trpc/history.move"
        ) && body.len() <= 16 * 1024)
        || (path == "/api/trpc/sync.resolveConflict" && body.len() <= 16 * 1024)
        || (path == "/api/trpc/knowledge.save" && is_compact_knowledge_intent(body))
        || (path == "/api/trpc/chat.send" && is_compact_chat_intent(body))
}

pub fn requires_signature(procedure: &str) -> bool {
    matches!(
        procedure,
        "items.create"
            | "items.update"
            | "items.remove"
            | "items.addPhoto"
            | "items.addComment"
            | "items.reportFault"
            | "items.resolveFault"
            | "items.requestChange"
            | "items.decideChange"
            | "transfers.take"
            | "transfers.takeMany"
            | "transfers.returnItem"
            | "transfers.prepare"
            | "transfers.accept"
            | "transfers.reject"
            | "transfers.acceptAll"
            | "history.writeOff"
            | "history.replenish"
            | "history.move"
            | "inventory.create"
            | "inventory.checkItem"
            | "inventory.complete"
            | "chat.send"
            | "items.addDocument"
            | "bit.transfer"
            | "bit.sale"
            | "bit.offer"
            | "bit.acceptSale"
            | "bit.rejectSale"
            | "bit.mint"
            | "knowledge.save"
            | "interorg.ensureIdentity"
            | "interorg.trustContact"
            | "interorg.revokeContact"
            | "interorg.send"
            | "interorg.accept"
            | "interorg.importGossip"
            | "sync.importBundle"
            | "sync.approveNodeKey"
            | "sync.revokeNodeKey"
            | "sync.addPeer"
            | "sync.removePeer"
            | "sync.pullNow"
            | "sync.resolveConflict"
            | "sync.clearDiagnostics"
            | "sync.reportTransportStatus"
            | "content.setMode"
            | "content.ingest"
            | "content.pin"
            | "content.unpin"
            | "backup.export"
            | "backup.import"
            | "profile.update"
            | "profile.changePassword"
            | "profile.leaveWorkspace"
            | "profile.deleteAccount"
            | "auth.revokeDevice"
            | "admin.users.create"
            | "admin.users.update"
            | "admin.users.remove"
            | "admin.users.invite"
            | "admin.workspaces.create"
            | "admin.workspaces.update"
            | "admin.workspaces.remove"
            | "admin.workspaces.createInvite"
            | "admin.organizationNodes.create"
            | "admin.organizationNodes.update"
            | "admin.organizationNodes.remove"
            | "admin.storages.create"
            | "admin.storages.update"
            | "admin.storages.remove"
            | "admin.buildingSites.create"
            | "admin.buildingSites.update"
            | "admin.buildingSites.remove"
            | "admin.dictionaries.create"
            | "admin.dictionaries.update"
            | "admin.dictionaries.remove"
    )
}

pub fn register(conn: &Connection, user_id: i64, input: &Value) -> anyhow::Result<Value> {
    let device_id = input
        .get("deviceId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Это устройство")
        .trim();
    let public_key = input
        .get("publicKey")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if device_id.len() < 16
        || device_id.len() > 100
        || name.is_empty()
        || name.chars().count() > 100
    {
        anyhow::bail!("некорректные параметры устройства");
    }
    let raw = URL_SAFE_NO_PAD.decode(public_key)?;
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("публичный ключ должен содержать 32 байта"))?;
    VerifyingKey::from_bytes(&bytes)?;
    let existing: Option<i64> = conn
        .query_row(
            "SELECT user_id FROM user_devices WHERE device_id=?1",
            [device_id],
            |r| r.get(0),
        )
        .optional()?;
    if existing.is_some_and(|owner| owner != user_id) {
        anyhow::bail!("идентификатор устройства уже занят");
    }
    if let Some(existing_key) = conn
        .query_row(
            "SELECT public_key FROM user_devices WHERE device_id=?1 AND user_id=?2",
            params![device_id, user_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        if existing_key != public_key {
            anyhow::bail!("идентификатор устройства уже связан с другим ключом");
        }
        conn.execute(
            "UPDATE user_devices SET name=?1 WHERE device_id=?2 AND user_id=?3",
            params![name, device_id, user_id],
        )?;
    } else {
        conn.execute(
            "INSERT INTO user_devices(device_id,user_id,name,public_key,created_at)
             VALUES(?1,?2,?3,?4,?5)",
            params![
                device_id,
                user_id,
                name,
                public_key,
                Utc::now().to_rfc3339()
            ],
        )?;
    }
    let revoked = conn
        .query_row(
            "SELECT revoked_at IS NOT NULL FROM user_devices WHERE device_id=?1",
            [device_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(true);
    Ok(json!({"deviceId":device_id,"name":name,"publicKey":public_key,"revoked":revoked}))
}

pub fn list(conn: &Connection, user_id: i64) -> anyhow::Result<Value> {
    let mut stmt = conn.prepare(
        "SELECT device_id,name,public_key,created_at,last_seen_at,revoked_at FROM user_devices WHERE user_id=?1 ORDER BY created_at DESC",
    )?;
    let rows: Vec<Value> = stmt.query_map([user_id], |r| Ok(json!({
        "deviceId":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,
        "publicKey":r.get::<_,String>(2)?,"createdAt":r.get::<_,String>(3)?,
        "lastSeenAt":r.get::<_,Option<String>>(4)?,"revoked":r.get::<_,Option<String>>(5)?.is_some()
    })))?.filter_map(Result::ok).collect();
    Ok(Value::Array(rows))
}

pub fn revoke(conn: &Connection, user_id: i64, device_id: &str) -> anyhow::Result<bool> {
    Ok(conn.execute(
        "UPDATE user_devices SET revoked_at=?1 WHERE device_id=?2 AND user_id=?3 AND revoked_at IS NULL",
        params![Utc::now().to_rfc3339(),device_id,user_id],
    )? == 1)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> anyhow::Result<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing {name}"))
}

pub fn verify_request(
    conn: &Connection,
    user_id: i64,
    path: &str,
    body: &[u8],
    headers: &HeaderMap,
) -> anyhow::Result<Proof> {
    let device_id = header(headers, "x-everyday-device")?;
    let nonce = header(headers, "x-everyday-nonce")?;
    let timestamp = header(headers, "x-everyday-timestamp")?;
    let signature = header(headers, "x-everyday-signature")?;
    if nonce.len() < 16 || nonce.len() > 100 {
        anyhow::bail!("invalid nonce");
    }
    let seconds: i64 = timestamp.parse()?;
    if (Utc::now().timestamp() - seconds).abs() > MAX_CLOCK_SKEW_SECONDS {
        anyhow::bail!("request timestamp outside offline tolerance");
    }
    let public_key: String = conn.query_row(
        "SELECT public_key FROM user_devices WHERE device_id=?1 AND user_id=?2 AND revoked_at IS NULL",
        params![device_id,user_id], |r| r.get(0),
    )?;
    let body_hash = hex::encode(Sha256::digest(body));
    let message = format!("{DOMAIN}\nPOST\n{path}\n{timestamp}\n{nonce}\n{body_hash}");
    let pk_raw = URL_SAFE_NO_PAD.decode(&public_key)?;
    let key =
        VerifyingKey::from_bytes(&pk_raw.try_into().map_err(|_| anyhow::anyhow!("bad key"))?)?;
    let sig = Signature::from_slice(&URL_SAFE_NO_PAD.decode(signature)?)?;
    key.verify(message.as_bytes(), &sig)?;
    conn.execute(
        "INSERT INTO device_nonces(device_id,nonce,used_at) VALUES(?1,?2,?3)",
        params![device_id, nonce, Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "UPDATE user_devices SET last_seen_at=?1 WHERE device_id=?2",
        params![Utc::now().to_rfc3339(), device_id],
    )?;
    Ok(Proof {
        device_id: device_id.to_owned(),
        public_key,
        nonce: nonce.to_owned(),
        signature: signature.to_owned(),
        request_hash: body_hash,
        timestamp: timestamp.to_owned(),
        path: path.to_owned(),
        request_body: should_retain_request_body(path, body)
            .then(|| String::from_utf8(body.to_vec()))
            .transpose()?,
    })
}

pub fn set_pending(conn: &Connection, user_id: i64, proof: &Proof) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO pending_device_proofs(user_id,device_id,public_key,nonce,signature,request_hash,request_timestamp,request_path,request_body)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(user_id) DO UPDATE SET device_id=excluded.device_id,public_key=excluded.public_key,nonce=excluded.nonce,signature=excluded.signature,request_hash=excluded.request_hash,request_timestamp=excluded.request_timestamp,request_path=excluded.request_path,request_body=excluded.request_body",
        params![user_id,proof.device_id,proof.public_key,proof.nonce,proof.signature,proof.request_hash,proof.timestamp,proof.path,proof.request_body],
    )?;
    Ok(())
}

pub fn clear_pending(conn: &Connection, user_id: Option<i64>) {
    if let Some(user_id) = user_id {
        let _ = conn.execute(
            "DELETE FROM pending_device_proofs WHERE user_id=?1",
            [user_id],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE user_devices(device_id TEXT PRIMARY KEY,user_id INTEGER NOT NULL,name TEXT NOT NULL,public_key TEXT NOT NULL,created_at TEXT NOT NULL,last_seen_at TEXT,revoked_at TEXT);
             CREATE TABLE device_nonces(device_id TEXT NOT NULL,nonce TEXT NOT NULL,used_at TEXT NOT NULL,PRIMARY KEY(device_id,nonce));",
        ).unwrap();
        db
    }

    fn signed_headers(
        key: &SigningKey,
        device: &str,
        path: &str,
        body: &[u8],
        nonce: &str,
    ) -> HeaderMap {
        let timestamp = Utc::now().timestamp().to_string();
        let hash = hex::encode(Sha256::digest(body));
        let message = format!("{DOMAIN}\nPOST\n{path}\n{timestamp}\n{nonce}\n{hash}");
        let mut headers = HeaderMap::new();
        headers.insert("x-everyday-device", device.parse().unwrap());
        headers.insert("x-everyday-timestamp", timestamp.parse().unwrap());
        headers.insert("x-everyday-nonce", nonce.parse().unwrap());
        headers.insert(
            "x-everyday-signature",
            URL_SAFE_NO_PAD
                .encode(key.sign(message.as_bytes()).to_bytes())
                .parse()
                .unwrap(),
        );
        headers
    }

    #[test]
    fn verifies_exact_request_and_rejects_replay_or_tampering() {
        for operation in [
            "items.addPhoto",
            "transfers.acceptAll",
            "sync.approveNodeKey",
            "sync.revokeNodeKey",
            "sync.addPeer",
            "sync.removePeer",
            "sync.pullNow",
            "sync.resolveConflict",
            "content.pin",
            "content.unpin",
            "backup.export",
            "backup.import",
            "profile.update",
            "profile.changePassword",
            "profile.leaveWorkspace",
            "profile.deleteAccount",
            "auth.revokeDevice",
            "sync.reportTransportStatus",
        ] {
            assert!(
                requires_signature(operation),
                "unsigned mutation: {operation}"
            );
        }
        let db = database();
        let key = SigningKey::generate(&mut OsRng);
        register(
            &db,
            7,
            &json!({
                "deviceId":"phone-device-0001","name":"Телефон",
                "publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        let path = "/api/trpc/transfers.take";
        let body = br#"{"0":{"json":{"itemId":42,"qrLabel":"everyday:item:v2:signed"}}}"#;
        let headers = signed_headers(&key, "phone-device-0001", path, body, "unique-nonce-0001");
        let proof = verify_request(&db, 7, path, body, &headers).unwrap();
        assert_eq!(
            proof.request_body.as_deref(),
            std::str::from_utf8(body).ok(),
            "QR possession proof должен оставаться в проверяемом V3 Ledger body"
        );
        assert!(verify_request(&db, 7, path, body, &headers).is_err());

        let tampered_headers =
            signed_headers(&key, "phone-device-0001", path, body, "unique-nonce-0002");
        assert!(verify_request(&db, 7, path, br#"{"itemId":43}"#, &tampered_headers).is_err());

        let inventory_path = "/api/trpc/inventory.checkItem";
        let inventory_body = br#"{"0":{"json":{"sessionId":1,"itemId":42,"actualQty":7}}}"#;
        let inventory_headers = signed_headers(
            &key,
            "phone-device-0001",
            inventory_path,
            inventory_body,
            "unique-nonce-0003",
        );
        let inventory_proof =
            verify_request(&db, 7, inventory_path, inventory_body, &inventory_headers).unwrap();
        assert_eq!(
            inventory_proof.request_body.as_deref(),
            std::str::from_utf8(inventory_body).ok()
        );
    }

    #[test]
    fn administrative_identity_and_structure_changes_require_device_signature() {
        for procedure in [
            "admin.users.create",
            "admin.users.update",
            "admin.users.remove",
            "admin.users.invite",
            "admin.workspaces.create",
            "admin.workspaces.update",
            "admin.workspaces.remove",
            "admin.workspaces.createInvite",
            "admin.organizationNodes.create",
            "admin.organizationNodes.update",
            "admin.organizationNodes.remove",
            "admin.storages.create",
            "admin.storages.update",
            "admin.storages.remove",
            "admin.buildingSites.create",
            "admin.buildingSites.update",
            "admin.buildingSites.remove",
            "admin.dictionaries.create",
            "admin.dictionaries.update",
            "admin.dictionaries.remove",
        ] {
            assert!(requires_signature(procedure), "unsigned {procedure}");
        }
        assert!(!requires_signature("admin.users.list"));
        assert!(!requires_signature("admin.organizationNodes.list"));
    }

    #[test]
    fn inventory_master_data_changes_require_device_signature() {
        for procedure in [
            "items.create",
            "items.update",
            "items.remove",
            "items.addComment",
            "items.reportFault",
            "items.resolveFault",
            "items.requestChange",
            "items.decideChange",
            "items.addDocument",
        ] {
            assert!(requires_signature(procedure), "unsigned {procedure}");
        }
        assert!(!requires_signature("items.list"));
        assert!(!requires_signature("items.byCode"));
    }

    #[test]
    fn knowledge_request_body_is_retained_only_for_compact_cas_intent() {
        let compact = json!({"0":{"json":{
            "workspaceGuid":uuid::Uuid::new_v4().to_string(),
            "pageGuid":uuid::Uuid::new_v4().to_string(),
            "revisionGuid":uuid::Uuid::new_v4().to_string(),
            "attachments":[{"name":"Manual","url":format!("cas:{}", "a".repeat(64))}]
        }}});
        assert!(is_compact_knowledge_intent(compact.to_string().as_bytes()));
        let inline = json!({"0":{"json":{
            "workspaceGuid":uuid::Uuid::new_v4().to_string(),
            "pageGuid":uuid::Uuid::new_v4().to_string(),
            "revisionGuid":uuid::Uuid::new_v4().to_string(),
            "attachments":[{"name":"Manual","url":"data:text/plain;base64,QUJD"}]
        }}});
        assert!(!is_compact_knowledge_intent(inline.to_string().as_bytes()));
        assert!(!is_compact_knowledge_intent(
            json!({"0":{"json":{"attachments":[]}}})
                .to_string()
                .as_bytes()
        ));
        let chat = json!({"0":{"json":{
            "workspaceGuid":uuid::Uuid::new_v4().to_string(),
            "messageGuid":uuid::Uuid::new_v4().to_string(),
            "attachments":[{"name":"Note","url":format!("cas:{}", "b".repeat(64))}]
        }}});
        assert!(is_compact_chat_intent(chat.to_string().as_bytes()));
        let inline_chat = json!({"0":{"json":{
            "workspaceGuid":uuid::Uuid::new_v4().to_string(),
            "messageGuid":uuid::Uuid::new_v4().to_string(),
            "attachments":[{"name":"Note","url":"data:text/plain;base64,QUJD"}]
        }}});
        assert!(!is_compact_chat_intent(inline_chat.to_string().as_bytes()));
    }

    #[test]
    fn compact_interorg_intent_is_retained_for_v3_audit() {
        let body = r#"{"0":{"json":{"workspaceId":1,"contactGuid":"5c859a48-ec72-4c86-bf96-8f0fc60d7a4c","transactionId":"be333a8c-2452-438d-8e28-a4221bed51ce","kind":"message.notice","body":{"text":"Смена принята"}}}}"#.as_bytes();
        assert!(should_retain_request_body("/api/trpc/interorg.send", body));
        assert!(!should_retain_request_body(
            "/api/trpc/interorg.send",
            &vec![0; 64 * 1024 + 1]
        ));
        assert!(requires_signature("interorg.send"));
    }

    #[test]
    fn writeoff_request_body_is_retained_only_for_compact_cas_intent() {
        let operation = uuid::Uuid::new_v4().to_string();
        let compact = json!({"0":{"json":{
            "operationGuid":operation,
            "photoUrl":format!("cas:{}", "c".repeat(64))
        }}});
        assert!(should_retain_request_body(
            "/api/trpc/history.writeOff",
            compact.to_string().as_bytes()
        ));
        let inline = json!({"0":{"json":{
            "operationGuid":uuid::Uuid::new_v4().to_string(),
            "photoUrl":"data:image/png;base64,QUJD"
        }}});
        assert!(!should_retain_request_body(
            "/api/trpc/history.writeOff",
            inline.to_string().as_bytes()
        ));
        assert!(!should_retain_request_body(
            "/api/trpc/history.writeOff",
            &vec![0; 16 * 1024 + 1]
        ));
    }

    #[test]
    fn stock_operation_request_bodies_are_retained_with_a_strict_limit() {
        let body = br#"{"0":{"json":{"operationGuid":"d014d5ea-d7e9-4b39-b195-b90ec8451018","workspaceGuid":"f8390f35-63e0-4772-92e8-a440ec8c9286","itemGuid":"2e7caf4e-9da2-47f1-9801-d155990bfc19","quantity":4}}}"#;
        for path in ["/api/trpc/history.replenish", "/api/trpc/history.move"] {
            assert!(should_retain_request_body(path, body), "{path}");
            assert!(!should_retain_request_body(path, &vec![0; 16 * 1024 + 1]));
        }
    }

    #[test]
    fn conflict_resolution_retains_only_a_bounded_signed_intent() {
        let body = br#"{"0":{"json":{"id":1,"resolutionGuid":"d014d5ea-d7e9-4b39-b195-b90ec8451018","workspaceGuid":"f8390f35-63e0-4772-92e8-a440ec8c9286","itemGuid":"2e7caf4e-9da2-47f1-9801-d155990bfc19","responsibleUserGuid":null}}}"#;
        assert!(should_retain_request_body(
            "/api/trpc/sync.resolveConflict",
            body
        ));
        assert!(!should_retain_request_body(
            "/api/trpc/sync.resolveConflict",
            &vec![0; 16 * 1024 + 1]
        ));
    }

    #[test]
    fn portable_sale_offer_retains_a_bounded_signed_intent() {
        let body = br#"{"0":{"json":{"offerGuid":"d014d5ea-d7e9-4b39-b195-b90ec8451018","workspaceGuid":"f8390f35-63e0-4772-92e8-a440ec8c9286","itemGuid":"2e7caf4e-9da2-47f1-9801-d155990bfc19","buyerGuid":"fa06e502-6dc0-40e2-8e18-d536151ee68a","bitAmount":20}}}"#;
        assert!(should_retain_request_body("/api/trpc/bit.offer", body));
        assert!(!should_retain_request_body(
            "/api/trpc/bit.offer",
            &vec![0; 16 * 1024 + 1]
        ));
        assert!(requires_signature("bit.offer"));
    }

    #[test]
    fn direct_bit_operations_retain_only_bounded_signed_intents() {
        let body =
            br#"{"0":{"json":{"workspaceId":1,"recipientUserId":2,"amount":25,"memo":"Shift"}}}"#;
        for path in [
            "/api/trpc/bit.transfer",
            "/api/trpc/bit.mint",
            "/api/trpc/bit.sale",
        ] {
            assert!(should_retain_request_body(path, body), "{path}");
            assert!(!should_retain_request_body(path, &vec![0; 16 * 1024 + 1]));
        }
    }

    #[test]
    fn revoked_device_cannot_sign() {
        let db = database();
        let key = SigningKey::generate(&mut OsRng);
        register(
            &db,
            7,
            &json!({
                "deviceId":"phone-device-0002","name":"Телефон",
                "publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        assert!(revoke(&db, 7, "phone-device-0002").unwrap());
        let body = b"{}";
        let headers = signed_headers(
            &key,
            "phone-device-0002",
            "/api/trpc/transfers.returnItem",
            body,
            "unique-nonce-0003",
        );
        assert!(verify_request(&db, 7, "/api/trpc/transfers.returnItem", body, &headers).is_err());
    }

    #[test]
    fn device_identity_cannot_rotate_key_or_clear_revocation() {
        let db = database();
        let original = SigningKey::generate(&mut OsRng);
        let replacement = SigningKey::generate(&mut OsRng);
        let device_id = "phone-device-immutable-0003";
        register(
            &db,
            7,
            &json!({
                "deviceId":device_id,"name":"Телефон",
                "publicKey":URL_SAFE_NO_PAD.encode(original.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        assert!(register(
            &db,
            7,
            &json!({
                "deviceId":device_id,"name":"Подмена",
                "publicKey":URL_SAFE_NO_PAD.encode(replacement.verifying_key().to_bytes())
            }),
        )
        .is_err());
        assert!(revoke(&db, 7, device_id).unwrap());
        let repeated = register(
            &db,
            7,
            &json!({
                "deviceId":device_id,"name":"Переименован",
                "publicKey":URL_SAFE_NO_PAD.encode(original.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        assert_eq!(repeated["revoked"], true);
        assert!(db
            .query_row(
                "SELECT revoked_at FROM user_devices WHERE device_id=?1",
                [device_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
            .is_some());
    }
}
