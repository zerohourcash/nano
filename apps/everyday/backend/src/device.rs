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
}

pub fn requires_signature(procedure: &str) -> bool {
    matches!(
        procedure,
        "items.create"
            | "items.update"
            | "transfers.take"
            | "transfers.takeMany"
            | "transfers.returnItem"
            | "transfers.prepare"
            | "transfers.accept"
            | "transfers.reject"
            | "history.writeOff"
            | "history.replenish"
            | "history.move"
            | "inventory.checkItem"
            | "inventory.complete"
            | "chat.send"
            | "items.addDocument"
            | "bit.transfer"
            | "bit.sale"
            | "bit.mint"
            | "knowledge.save"
            | "sync.importBundle"
            | "sync.clearDiagnostics"
            | "sync.reportTransportStatus"
            | "content.setMode"
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
    })
}

pub fn set_pending(conn: &Connection, user_id: i64, proof: &Proof) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO pending_device_proofs(user_id,device_id,public_key,nonce,signature,request_hash,request_timestamp,request_path)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(user_id) DO UPDATE SET device_id=excluded.device_id,public_key=excluded.public_key,nonce=excluded.nonce,signature=excluded.signature,request_hash=excluded.request_hash,request_timestamp=excluded.request_timestamp,request_path=excluded.request_path",
        params![user_id,proof.device_id,proof.public_key,proof.nonce,proof.signature,proof.request_hash,proof.timestamp,proof.path],
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
        assert!(requires_signature("sync.reportTransportStatus"));
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
        let body = br#"{"itemId":42}"#;
        let headers = signed_headers(&key, "phone-device-0001", path, body, "unique-nonce-0001");
        verify_request(&db, 7, path, body, &headers).unwrap();
        assert!(verify_request(&db, 7, path, body, &headers).is_err());

        let tampered_headers =
            signed_headers(&key, "phone-device-0001", path, body, "unique-nonce-0002");
        assert!(verify_request(&db, 7, path, br#"{"itemId":43}"#, &tampered_headers).is_err());
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
        ] {
            assert!(requires_signature(procedure), "unsigned {procedure}");
        }
        assert!(!requires_signature("admin.users.list"));
        assert!(!requires_signature("admin.organizationNodes.list"));
    }

    #[test]
    fn inventory_master_data_changes_require_device_signature() {
        for procedure in ["items.create", "items.update", "items.addDocument"] {
            assert!(requires_signature(procedure), "unsigned {procedure}");
        }
        assert!(!requires_signature("items.list"));
        assert!(!requires_signature("items.byCode"));
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
