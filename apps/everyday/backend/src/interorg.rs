//! Opaque, transport-independent store-and-forward envelopes between
//! organizations. Relays validate signatures, TTL, proof-of-work and replay,
//! but cannot decrypt the transaction. The serialized envelope is suitable
//! for IP, BLE, Wi-Fi Direct, LoRa framing or manual bundle transfer.

use anyhow::{anyhow, bail, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD as B64, STANDARD_NO_PAD},
    Engine,
};
use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as XPublicKey, StaticSecret};

const VERSION: u8 = 1;
const MAX_CIPHERTEXT: usize = 64 * 1024;
const MAX_TTL_HOURS: i64 = 168;
const MAX_RELAY_ENVELOPES: i64 = 4096;
const MAX_PER_SENDER: i64 = 128;
const AAD: &[u8] = b"everyday/interorg-envelope/v1";
const IDENTITY_AAD: &[u8] = b"everyday/interorg-identity/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Envelope {
    pub version: u8,
    pub id: String,
    /// SHA-256 of the recipient organization's X25519 public key.
    pub destination: String,
    pub sender_signing_key: String,
    pub ephemeral_key: String,
    pub nonce: String,
    pub created_at: String,
    pub expires_at: String,
    pub ciphertext: String,
    pub signature: String,
    pub pow_nonce: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Payload {
    pub sender_workspace: String,
    pub recipient_workspace: String,
    pub kind: String,
    pub transaction_id: String,
    pub body: serde_json::Value,
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS interorg_envelopes(
           id TEXT PRIMARY KEY, destination TEXT NOT NULL,
           sender_key TEXT NOT NULL, envelope_json TEXT NOT NULL,
           received_at TEXT NOT NULL, expires_at TEXT NOT NULL,
           delivered INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS interorg_destination_idx
           ON interorg_envelopes(destination,delivered,expires_at);
         CREATE TABLE IF NOT EXISTS interorg_identities(
           workspace_id INTEGER PRIMARY KEY,
           public_key TEXT NOT NULL UNIQUE, destination TEXT NOT NULL UNIQUE,
           secret_ciphertext TEXT NOT NULL, nonce TEXT NOT NULL,
           created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS interorg_contacts(
           guid TEXT PRIMARY KEY, workspace_id INTEGER NOT NULL,
           name TEXT NOT NULL, remote_workspace_guid TEXT NOT NULL,
           destination TEXT NOT NULL, encryption_key TEXT NOT NULL,
           signing_key TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1,
           created_at TEXT NOT NULL,
           UNIQUE(workspace_id,remote_workspace_guid)
         );
         CREATE TABLE IF NOT EXISTS interorg_inbox(
           envelope_id TEXT PRIMARY KEY, workspace_id INTEGER NOT NULL,
           contact_guid TEXT NOT NULL, transaction_id TEXT NOT NULL,
           kind TEXT NOT NULL, body_json TEXT NOT NULL,
           received_at TEXT NOT NULL, accepted INTEGER NOT NULL DEFAULT 0
         );",
    )?;
    Ok(())
}

fn identity_wrap_key(conn: &Connection, workspace_id: i64) -> Result<[u8; 32]> {
    let node_key = crate::ledger::signing_key(conn)?;
    let hk = Hkdf::<Sha256>::new(Some(&workspace_id.to_be_bytes()), &node_key.to_bytes());
    let mut key = [0u8; 32];
    hk.expand(IDENTITY_AAD, &mut key)
        .map_err(|_| anyhow!("identity HKDF failed"))?;
    Ok(key)
}

fn decrypt_identity_secret(
    conn: &Connection,
    workspace_id: i64,
    ciphertext: &str,
    nonce: &str,
) -> Result<[u8; 32]> {
    let key = identity_wrap_key(conn, workspace_id)?;
    let nonce: [u8; 24] = B64
        .decode(nonce)?
        .try_into()
        .map_err(|_| anyhow!("invalid identity nonce"))?;
    let plain = XChaCha20Poly1305::new_from_slice(&key)?
        .decrypt(XNonce::from_slice(&nonce), B64.decode(ciphertext)?.as_ref())
        .map_err(|_| anyhow!("identity key authentication failed"))?;
    plain
        .try_into()
        .map_err(|_| anyhow!("invalid identity secret length"))
}

/// Creates one gateway address for an organization on this node. The private
/// X25519 key is wrapped with a key derived from the node signing identity, so
/// a copied plaintext SQLite row alone is insufficient to decrypt it.
pub fn ensure_identity(conn: &Connection, workspace_id: i64) -> Result<serde_json::Value> {
    init_schema(conn)?;
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT public_key,destination FROM interorg_identities WHERE workspace_id=?1",
            [workspace_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((public_key, destination)) = existing {
        let signing = STANDARD_NO_PAD.decode(crate::ledger::node_public_key(conn)?)?;
        return Ok(
            serde_json::json!({"workspaceId":workspace_id,"publicKey":public_key,"destination":destination,"signingKey":B64.encode(signing)}),
        );
    }
    conn.query_row(
        "SELECT 1 FROM workspaces WHERE id=?1",
        [workspace_id],
        |_| Ok(()),
    )
    .context("workspace not found")?;
    let secret = StaticSecret::random_from_rng(OsRng);
    let public = XPublicKey::from(&secret);
    let wrap_key = identity_wrap_key(conn, workspace_id)?;
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = XChaCha20Poly1305::new_from_slice(&wrap_key)?
        .encrypt(XNonce::from_slice(&nonce), secret.as_bytes().as_ref())
        .map_err(|_| anyhow!("identity encryption failed"))?;
    let public_key = B64.encode(public.as_bytes());
    let address = destination(public.as_bytes());
    conn.execute(
        "INSERT INTO interorg_identities(workspace_id,public_key,destination,secret_ciphertext,nonce,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
        params![workspace_id, public_key, address, B64.encode(encrypted), B64.encode(nonce), Utc::now().to_rfc3339()],
    )?;
    let signing = STANDARD_NO_PAD.decode(crate::ledger::node_public_key(conn)?)?;
    Ok(
        serde_json::json!({"workspaceId":workspace_id,"publicKey":public_key,"destination":address,"signingKey":B64.encode(signing)}),
    )
}

pub fn identity(conn: &Connection, workspace_id: i64) -> Result<Option<serde_json::Value>> {
    init_schema(conn)?;
    let row: Option<(String, String, String)> = conn.query_row(
        "SELECT public_key,destination,created_at FROM interorg_identities WHERE workspace_id=?1",
        [workspace_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()?;
    let Some((public_key, destination, created_at)) = row else {
        return Ok(None);
    };
    let signing = STANDARD_NO_PAD.decode(crate::ledger::node_public_key(conn)?)?;
    Ok(Some(serde_json::json!({
        "workspaceId":workspace_id,"publicKey":public_key,"destination":destination,
        "createdAt":created_at,"signingKey":B64.encode(signing)
    })))
}

#[allow(clippy::too_many_arguments)]
pub fn trust_contact(
    conn: &Connection,
    workspace_id: i64,
    name: &str,
    remote_workspace_guid: &str,
    encryption_key: &str,
    signing_key: &str,
) -> Result<serde_json::Value> {
    init_schema(conn)?;
    if name.trim().is_empty() || name.chars().count() > 120 || remote_workspace_guid.len() > 128 {
        bail!("invalid interorg contact");
    }
    let encryption: [u8; 32] = B64
        .decode(encryption_key)?
        .try_into()
        .map_err(|_| anyhow!("invalid contact encryption key"))?;
    let signing: [u8; 32] = B64
        .decode(signing_key)?
        .try_into()
        .map_err(|_| anyhow!("invalid contact signing key"))?;
    let _ = VerifyingKey::from_bytes(&signing)?;
    let guid: String = conn
        .query_row(
            "SELECT guid FROM interorg_contacts WHERE workspace_id=?1 AND remote_workspace_guid=?2",
            params![workspace_id, remote_workspace_guid],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let address = destination(&encryption);
    conn.execute(
        "INSERT INTO interorg_contacts(guid,workspace_id,name,remote_workspace_guid,destination,encryption_key,signing_key,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(workspace_id,remote_workspace_guid) DO UPDATE SET name=excluded.name,destination=excluded.destination,encryption_key=excluded.encryption_key,signing_key=excluded.signing_key,active=1",
        params![guid, workspace_id, name.trim(), remote_workspace_guid, address, encryption_key, signing_key, Utc::now().to_rfc3339()],
    )?;
    Ok(
        serde_json::json!({"guid":guid,"workspaceId":workspace_id,"name":name.trim(),"remoteWorkspaceGuid":remote_workspace_guid,"destination":address,"encryptionKey":encryption_key,"signingKey":signing_key,"active":true}),
    )
}

pub fn list_contacts(conn: &Connection, workspace_id: i64) -> Result<serde_json::Value> {
    let mut statement = conn.prepare("SELECT guid,name,remote_workspace_guid,destination,encryption_key,signing_key,active,created_at FROM interorg_contacts WHERE workspace_id=?1 ORDER BY name")?;
    let rows = statement.query_map([workspace_id], |row| Ok(serde_json::json!({
        "guid":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"remoteWorkspaceGuid":row.get::<_,String>(2)?,
        "destination":row.get::<_,String>(3)?,"encryptionKey":row.get::<_,String>(4)?,"signingKey":row.get::<_,String>(5)?,
        "active":row.get::<_,bool>(6)?,"createdAt":row.get::<_,String>(7)?
    })))?;
    Ok(serde_json::Value::Array(rows.flatten().collect()))
}

pub fn send_to_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_guid: &str,
    kind: &str,
    transaction_id: &str,
    body: serde_json::Value,
    work_bits: u8,
) -> Result<Envelope> {
    if kind.is_empty() || kind.len() > 80 || uuid::Uuid::parse_str(transaction_id).is_err() {
        bail!("invalid interorg transaction");
    }
    let sender_workspace: String = conn.query_row(
        "SELECT guid FROM workspaces WHERE id=?1",
        [workspace_id],
        |row| row.get(0),
    )?;
    let (recipient_workspace, encryption_key): (String, String) = conn
        .query_row(
            "SELECT remote_workspace_guid,encryption_key FROM interorg_contacts WHERE guid=?1 AND workspace_id=?2 AND active=1",
            params![contact_guid, workspace_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("active interorg contact not found")?;
    let recipient: [u8; 32] = B64
        .decode(encryption_key)?
        .try_into()
        .map_err(|_| anyhow!("invalid recipient encryption key"))?;
    let envelope = seal(
        &Payload {
            sender_workspace,
            recipient_workspace,
            kind: kind.to_owned(),
            transaction_id: transaction_id.to_owned(),
            body,
        },
        &recipient,
        &crate::ledger::signing_key(conn)?,
        72,
        work_bits,
    )?;
    relay_store(conn, &envelope, work_bits)?;
    Ok(envelope)
}

/// Attempts local delivery for all gateway identities. Unknown senders remain
/// opaque relay traffic; only contacts explicitly trusted by an administrator
/// can become application inbox rows.
pub fn receive_local(conn: &Connection, work_bits: u8) -> Result<usize> {
    init_schema(conn)?;
    let mut identities = conn.prepare(
        "SELECT i.workspace_id,w.guid,i.destination,i.secret_ciphertext,i.nonce
         FROM interorg_identities i JOIN workspaces w ON w.id=i.workspace_id",
    )?;
    let identities: Vec<(i64, String, String, String, String)> = identities
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .flatten()
        .collect();
    let mut imported = 0;
    for (workspace_id, workspace_guid, destination, ciphertext, nonce) in identities {
        let secret = decrypt_identity_secret(conn, workspace_id, &ciphertext, &nonce)?;
        for envelope in pending_for(conn, &destination, 256)? {
            let contact: Option<(String, String)> = conn
                .query_row(
                    "SELECT guid,remote_workspace_guid FROM interorg_contacts
                     WHERE workspace_id=?1 AND signing_key=?2 AND active=1",
                    params![workspace_id, envelope.sender_signing_key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((contact_guid, remote_workspace)) = contact else {
                continue;
            };
            let payload = match open_authorized(
                &envelope,
                &secret,
                &workspace_guid,
                std::slice::from_ref(&envelope.sender_signing_key),
                work_bits,
            ) {
                Ok(payload) => payload,
                Err(_) => {
                    conn.execute(
                        "UPDATE interorg_envelopes SET delivered=2 WHERE id=?1",
                        [&envelope.id],
                    )?;
                    continue;
                }
            };
            if payload.sender_workspace != remote_workspace {
                conn.execute(
                    "UPDATE interorg_envelopes SET delivered=2 WHERE id=?1",
                    [&envelope.id],
                )?;
                continue;
            }
            let changed = conn.execute(
                "INSERT OR IGNORE INTO interorg_inbox(envelope_id,workspace_id,contact_guid,transaction_id,kind,body_json,received_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![envelope.id, workspace_id, contact_guid, payload.transaction_id, payload.kind, payload.body.to_string(), Utc::now().to_rfc3339()],
            )?;
            if changed == 1 {
                conn.execute(
                    "UPDATE interorg_envelopes SET delivered=1 WHERE id=?1",
                    [&envelope.id],
                )?;
                imported += 1;
            }
        }
    }
    Ok(imported)
}

pub fn inbox(conn: &Connection, workspace_id: i64) -> Result<serde_json::Value> {
    let mut statement = conn.prepare(
        "SELECT i.envelope_id,i.transaction_id,i.kind,i.body_json,i.received_at,i.accepted,c.guid,c.name,c.remote_workspace_guid
         FROM interorg_inbox i JOIN interorg_contacts c ON c.guid=i.contact_guid
         WHERE i.workspace_id=?1 ORDER BY i.received_at DESC",
    )?;
    let rows = statement.query_map([workspace_id], |row| {
        let body: String = row.get(3)?;
        Ok(serde_json::json!({
            "envelopeId":row.get::<_,String>(0)?,"transactionId":row.get::<_,String>(1)?,"kind":row.get::<_,String>(2)?,
            "body":serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null),
            "receivedAt":row.get::<_,String>(4)?,"accepted":row.get::<_,bool>(5)?,
            "contact":{"guid":row.get::<_,String>(6)?,"name":row.get::<_,String>(7)?,"remoteWorkspaceGuid":row.get::<_,String>(8)?}
        }))
    })?;
    Ok(serde_json::Value::Array(rows.flatten().collect()))
}

pub fn destination(public_key: &[u8; 32]) -> String {
    hex::encode(Sha256::digest(public_key))
}

fn derive_key(shared: &[u8; 32], destination: &str) -> Result<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(Some(destination.as_bytes()), shared);
    let mut key = [0u8; 32];
    hk.expand(AAD, &mut key)
        .map_err(|_| anyhow!("interorg HKDF failed"))?;
    Ok(key)
}

fn transcript(e: &Envelope) -> Vec<u8> {
    [
        e.version.to_string(),
        e.id.clone(),
        e.destination.clone(),
        e.sender_signing_key.clone(),
        e.ephemeral_key.clone(),
        e.nonce.clone(),
        e.created_at.clone(),
        e.expires_at.clone(),
        e.ciphertext.clone(),
    ]
    .join("\0")
    .into_bytes()
}

fn pow_digest(e: &Envelope) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(transcript(e));
    hash.update(e.signature.as_bytes());
    hash.update(e.pow_nonce.to_le_bytes());
    hash.finalize().into()
}

fn has_work(hash: &[u8; 32], bits: u8) -> bool {
    let bytes = (bits / 8) as usize;
    let remainder = bits % 8;
    hash[..bytes].iter().all(|byte| *byte == 0)
        && (remainder == 0 || hash[bytes] >> (8 - remainder) == 0)
}

pub fn seal(
    payload: &Payload,
    recipient_key: &[u8; 32],
    sender_key: &SigningKey,
    ttl_hours: i64,
    work_bits: u8,
) -> Result<Envelope> {
    if !(1..=MAX_TTL_HOURS).contains(&ttl_hours) || work_bits > 24 {
        bail!("invalid interorg TTL or work factor");
    }
    let plaintext = serde_json::to_vec(payload)?;
    if plaintext.len() > MAX_CIPHERTEXT {
        bail!("interorg payload is too large");
    }
    let ephemeral_secret = StaticSecret::random_from_rng(OsRng);
    let ephemeral_key = XPublicKey::from(&ephemeral_secret);
    let recipient = XPublicKey::from(*recipient_key);
    let destination = destination(recipient_key);
    let key = derive_key(
        ephemeral_secret.diffie_hellman(&recipient).as_bytes(),
        &destination,
    )?;
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = XChaCha20Poly1305::new_from_slice(&key)?
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| anyhow!("interorg encryption failed"))?;
    let created = Utc::now();
    let mut envelope = Envelope {
        version: VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        destination,
        sender_signing_key: B64.encode(sender_key.verifying_key().as_bytes()),
        ephemeral_key: B64.encode(ephemeral_key.as_bytes()),
        nonce: B64.encode(nonce),
        created_at: created.to_rfc3339(),
        expires_at: (created + Duration::hours(ttl_hours)).to_rfc3339(),
        ciphertext: B64.encode(ciphertext),
        signature: String::new(),
        pow_nonce: 0,
    };
    envelope.signature = B64.encode(sender_key.sign(&transcript(&envelope)).to_bytes());
    while !has_work(&pow_digest(&envelope), work_bits) {
        envelope.pow_nonce = envelope.pow_nonce.checked_add(1).context("PoW exhausted")?;
    }
    Ok(envelope)
}

pub fn validate(envelope: &Envelope, now: DateTime<Utc>, work_bits: u8) -> Result<()> {
    if envelope.version != VERSION
        || uuid::Uuid::parse_str(&envelope.id).is_err()
        || envelope.destination.len() != 64
        || hex::decode(&envelope.destination).map_or(true, |bytes| bytes.len() != 32)
    {
        bail!("invalid interorg envelope header");
    }
    let created = DateTime::parse_from_rfc3339(&envelope.created_at)?.with_timezone(&Utc);
    let expires = DateTime::parse_from_rfc3339(&envelope.expires_at)?.with_timezone(&Utc);
    if expires <= now
        || created > now + Duration::minutes(5)
        || expires - created > Duration::hours(MAX_TTL_HOURS)
    {
        bail!("expired or invalid interorg TTL");
    }
    let ciphertext = B64.decode(&envelope.ciphertext)?;
    if ciphertext.len() < 16 || ciphertext.len() > MAX_CIPHERTEXT + 16 {
        bail!("invalid interorg ciphertext size");
    }
    let ephemeral = B64.decode(&envelope.ephemeral_key)?;
    let nonce = B64.decode(&envelope.nonce)?;
    if ephemeral.len() != 32 || nonce.len() != 24 {
        bail!("invalid interorg encryption header");
    }
    let sender: [u8; 32] = B64
        .decode(&envelope.sender_signing_key)?
        .try_into()
        .map_err(|_| anyhow!("invalid sender key"))?;
    let signature: [u8; 64] = B64
        .decode(&envelope.signature)?
        .try_into()
        .map_err(|_| anyhow!("invalid signature"))?;
    VerifyingKey::from_bytes(&sender)?
        .verify(&transcript(envelope), &Signature::from_bytes(&signature))?;
    if !has_work(&pow_digest(envelope), work_bits) {
        bail!("insufficient interorg proof-of-work");
    }
    Ok(())
}

/// Idempotently accepts an opaque envelope. A relay learns no payload fields.
pub fn relay_store(conn: &Connection, envelope: &Envelope, work_bits: u8) -> Result<bool> {
    init_schema(conn)?;
    validate(envelope, Utc::now(), work_bits)?;
    conn.execute(
        "DELETE FROM interorg_envelopes WHERE expires_at<=?1",
        [Utc::now().to_rfc3339()],
    )?;
    if conn
        .query_row(
            "SELECT 1 FROM interorg_envelopes WHERE id=?1",
            [&envelope.id],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Ok(false);
    }
    let sender_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM interorg_envelopes WHERE sender_key=?1 AND expires_at>?2",
        params![envelope.sender_signing_key, Utc::now().to_rfc3339()],
        |row| row.get(0),
    )?;
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM interorg_envelopes WHERE expires_at>?1",
        [Utc::now().to_rfc3339()],
        |row| row.get(0),
    )?;
    if sender_count >= MAX_PER_SENDER || total >= MAX_RELAY_ENVELOPES {
        bail!("interorg relay quota exceeded");
    }
    conn.execute(
        "INSERT INTO interorg_envelopes(id,destination,sender_key,envelope_json,received_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6)",
        params![envelope.id, envelope.destination, envelope.sender_signing_key, serde_json::to_string(envelope)?, Utc::now().to_rfc3339(), envelope.expires_at],
    )?;
    Ok(true)
}

pub fn pending_for(conn: &Connection, destination: &str, limit: usize) -> Result<Vec<Envelope>> {
    if limit == 0 || limit > 256 || destination.len() != 64 {
        bail!("invalid interorg query");
    }
    let mut statement = conn.prepare("SELECT envelope_json FROM interorg_envelopes WHERE destination=?1 AND delivered=0 AND expires_at>?2 ORDER BY received_at LIMIT ?3")?;
    let rows = statement.query_map(
        params![destination, Utc::now().to_rfc3339(), limit as i64],
        |row| row.get::<_, String>(0),
    )?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

/// Bounded gossip batch used by any concrete transport adapter. Envelopes are
/// already opaque; peers can exchange this batch without sharing an
/// organization capability. Duplicate IDs are harmless at the receiving DB.
pub fn gossip_batch(conn: &Connection, limit: usize) -> Result<Vec<Envelope>> {
    if limit == 0 || limit > 256 {
        bail!("invalid interorg gossip limit");
    }
    let mut statement = conn.prepare(
        "SELECT envelope_json FROM interorg_envelopes
         WHERE expires_at>?1 ORDER BY received_at DESC LIMIT ?2",
    )?;
    let rows = statement.query_map(params![Utc::now().to_rfc3339(), limit as i64], |row| {
        row.get::<_, String>(0)
    })?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

pub fn open(envelope: &Envelope, recipient_secret: &[u8; 32], work_bits: u8) -> Result<Payload> {
    validate(envelope, Utc::now(), work_bits)?;
    let secret = StaticSecret::from(*recipient_secret);
    let public = XPublicKey::from(&secret);
    if destination(public.as_bytes()) != envelope.destination {
        bail!("wrong interorg recipient");
    }
    let ephemeral: [u8; 32] = B64
        .decode(&envelope.ephemeral_key)?
        .try_into()
        .map_err(|_| anyhow!("invalid ephemeral key"))?;
    let nonce: [u8; 24] = B64
        .decode(&envelope.nonce)?
        .try_into()
        .map_err(|_| anyhow!("invalid nonce"))?;
    let key = derive_key(
        secret
            .diffie_hellman(&XPublicKey::from(ephemeral))
            .as_bytes(),
        &envelope.destination,
    )?;
    let plaintext = XChaCha20Poly1305::new_from_slice(&key)?
        .decrypt(
            XNonce::from_slice(&nonce),
            B64.decode(&envelope.ciphertext)?.as_ref(),
        )
        .map_err(|_| anyhow!("interorg authentication failed"))?;
    Ok(serde_json::from_slice(&plaintext)?)
}

/// Opens an envelope only when both the destination organization and sender's
/// signing identity match the recipient's trusted directory. Key discovery
/// and human approval remain separate from transport, preventing a relay from
/// inventing an organization merely by generating a fresh key pair.
pub fn open_authorized(
    envelope: &Envelope,
    recipient_secret: &[u8; 32],
    expected_workspace: &str,
    trusted_sender_keys: &[String],
    work_bits: u8,
) -> Result<Payload> {
    if !trusted_sender_keys
        .iter()
        .any(|key| key == &envelope.sender_signing_key)
    {
        bail!("untrusted interorg sender");
    }
    let payload = open(envelope, recipient_secret, work_bits)?;
    if payload.recipient_workspace != expected_workspace {
        bail!("interorg recipient workspace mismatch");
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Payload {
        Payload {
            sender_workspace: "org-a".into(),
            recipient_workspace: "org-b".into(),
            kind: "bit.transfer.offer".into(),
            transaction_id: "tx-1".into(),
            body: serde_json::json!({"amount":7,"memo":"смена"}),
        }
    }

    #[test]
    fn opaque_relay_cannot_read_and_recipient_can() {
        let recipient = StaticSecret::random_from_rng(OsRng);
        let sender = SigningKey::generate(&mut OsRng);
        let envelope = seal(
            &payload(),
            XPublicKey::from(&recipient).as_bytes(),
            &sender,
            24,
            8,
        )
        .unwrap();
        let serialized = serde_json::to_string(&envelope).unwrap();
        assert!(!serialized.contains("org-a"));
        assert!(!serialized.contains("amount"));
        assert_eq!(open(&envelope, recipient.as_bytes(), 8).unwrap(), payload());
        assert_eq!(
            open_authorized(
                &envelope,
                recipient.as_bytes(),
                "org-b",
                &[envelope.sender_signing_key.clone()],
                8
            )
            .unwrap(),
            payload()
        );
        assert!(open_authorized(
            &envelope,
            recipient.as_bytes(),
            "org-b",
            &["attacker".into()],
            8
        )
        .is_err());
        let wrong = StaticSecret::random_from_rng(OsRng);
        assert!(open(&envelope, wrong.as_bytes(), 8).is_err());
    }

    #[test]
    fn relay_rejects_tamper_replay_and_spam_without_work() {
        let db = Connection::open_in_memory().unwrap();
        let recipient = StaticSecret::random_from_rng(OsRng);
        let sender = SigningKey::generate(&mut OsRng);
        let envelope = seal(
            &payload(),
            XPublicKey::from(&recipient).as_bytes(),
            &sender,
            1,
            8,
        )
        .unwrap();
        assert!(relay_store(&db, &envelope, 8).unwrap());
        assert!(!relay_store(&db, &envelope, 8).unwrap());
        assert_eq!(
            pending_for(&db, &envelope.destination, 10).unwrap().len(),
            1
        );
        let mut forged = envelope.clone();
        forged.destination = "0".repeat(64);
        assert!(relay_store(&db, &forged, 8).is_err());
        let mut no_work = envelope.clone();
        no_work.id = uuid::Uuid::new_v4().to_string();
        no_work.signature = B64.encode(sender.sign(&transcript(&no_work)).to_bytes());
        no_work.pow_nonce = 0;
        while has_work(&pow_digest(&no_work), 8) {
            no_work.pow_nonce += 1;
        }
        assert!(relay_store(&db, &no_work, 8).is_err());
    }

    #[test]
    fn partitioned_three_hop_mesh_eventually_delivers_without_org_leakage() {
        let a = Connection::open_in_memory().unwrap();
        let relay = Connection::open_in_memory().unwrap();
        let b = Connection::open_in_memory().unwrap();
        let recipient = StaticSecret::random_from_rng(OsRng);
        let sender = SigningKey::generate(&mut OsRng);
        let envelope = seal(
            &payload(),
            XPublicKey::from(&recipient).as_bytes(),
            &sender,
            24,
            8,
        )
        .unwrap();

        // B is offline. A can only reach an organization-blind relay.
        assert!(relay_store(&a, &envelope, 8).unwrap());
        for opaque in gossip_batch(&a, 32).unwrap() {
            assert!(relay_store(&relay, &opaque, 8).unwrap());
        }
        assert!(pending_for(&relay, &envelope.destination, 8)
            .unwrap()
            .iter()
            .all(|value| !serde_json::to_string(value).unwrap().contains("org-a")));

        // B reconnects later, receives via relay, validates its destination and
        // the independently trusted sender key, while replay stays idempotent.
        for opaque in gossip_batch(&relay, 32).unwrap() {
            assert!(relay_store(&b, &opaque, 8).unwrap());
            assert!(!relay_store(&b, &opaque, 8).unwrap());
        }
        let delivered = pending_for(&b, &envelope.destination, 8).unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(
            open_authorized(
                &delivered[0],
                recipient.as_bytes(),
                "org-b",
                &[envelope.sender_signing_key],
                8,
            )
            .unwrap(),
            payload()
        );
    }

    #[test]
    fn organization_identity_is_wrapped_and_contacts_are_validated() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE kv(k TEXT PRIMARY KEY,v TEXT NOT NULL);
             CREATE TABLE workspaces(id INTEGER PRIMARY KEY,guid TEXT NOT NULL);",
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,guid) VALUES(1,'org-a')", [])
            .unwrap();
        let identity = ensure_identity(&db, 1).unwrap();
        assert_eq!(identity["destination"].as_str().unwrap().len(), 64);
        let (ciphertext, nonce): (String, String) = db
            .query_row(
                "SELECT secret_ciphertext,nonce FROM interorg_identities WHERE workspace_id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let secret = decrypt_identity_secret(&db, 1, &ciphertext, &nonce).unwrap();
        assert_eq!(
            B64.encode(XPublicKey::from(&StaticSecret::from(secret)).as_bytes()),
            identity["publicKey"]
        );
        assert!(!ciphertext.contains(identity["publicKey"].as_str().unwrap()));

        let remote_signer = SigningKey::generate(&mut OsRng);
        let contact = trust_contact(
            &db,
            1,
            "Организация Б",
            "org-b",
            identity["publicKey"].as_str().unwrap(),
            &B64.encode(remote_signer.verifying_key().as_bytes()),
        )
        .unwrap();
        assert_eq!(contact["remoteWorkspaceGuid"], "org-b");
        assert_eq!(list_contacts(&db, 1).unwrap().as_array().unwrap().len(), 1);
        assert!(trust_contact(&db, 1, "Атака", "org-x", "bad", "bad").is_err());
    }

    fn organization_db(guid: &str) -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE kv(k TEXT PRIMARY KEY,v TEXT NOT NULL);
             CREATE TABLE workspaces(id INTEGER PRIMARY KEY,guid TEXT NOT NULL);",
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,guid) VALUES(1,?1)", [guid])
            .unwrap();
        init_schema(&db).unwrap();
        db
    }

    #[test]
    fn trusted_organizations_exchange_transaction_after_partition() {
        let a = organization_db("org-a");
        let b = organization_db("org-b");
        let identity_a = ensure_identity(&a, 1).unwrap();
        let identity_b = ensure_identity(&b, 1).unwrap();
        let contact_b = trust_contact(
            &a,
            1,
            "Б",
            "org-b",
            identity_b["publicKey"].as_str().unwrap(),
            identity_b["signingKey"].as_str().unwrap(),
        )
        .unwrap();
        trust_contact(
            &b,
            1,
            "А",
            "org-a",
            identity_a["publicKey"].as_str().unwrap(),
            identity_a["signingKey"].as_str().unwrap(),
        )
        .unwrap();
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let envelope = send_to_contact(
            &a,
            1,
            contact_b["guid"].as_str().unwrap(),
            "invoice.offer",
            &transaction_id,
            serde_json::json!({"amount":125,"currency":"BIT","memo":"Поставка"}),
            8,
        )
        .unwrap();
        assert!(inbox(&b, 1).unwrap().as_array().unwrap().is_empty());

        // Connectivity returns later; B only receives the opaque transaction.
        let wrong_recipient = StaticSecret::random_from_rng(OsRng);
        let mut poison = seal(
            &payload(),
            XPublicKey::from(&wrong_recipient).as_bytes(),
            &crate::ledger::signing_key(&a).unwrap(),
            1,
            8,
        )
        .unwrap();
        poison.destination = identity_b["destination"].as_str().unwrap().to_owned();
        poison.signature = B64.encode(
            crate::ledger::signing_key(&a)
                .unwrap()
                .sign(&transcript(&poison))
                .to_bytes(),
        );
        poison.pow_nonce = 0;
        while !has_work(&pow_digest(&poison), 8) {
            poison.pow_nonce += 1;
        }
        assert!(relay_store(&b, &poison, 8).unwrap());
        assert!(relay_store(&b, &envelope, 8).unwrap());
        assert_eq!(receive_local(&b, 8).unwrap(), 1);
        assert_eq!(receive_local(&b, 8).unwrap(), 0);
        let inbox = inbox(&b, 1).unwrap();
        assert_eq!(inbox[0]["transactionId"], transaction_id);
        assert_eq!(inbox[0]["body"]["amount"], 125);
        assert_eq!(inbox[0]["contact"]["remoteWorkspaceGuid"], "org-a");
    }
}
