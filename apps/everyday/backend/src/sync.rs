use crate::{db, json as jsn, ledger};
use base64::{
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
    Engine,
};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// Запрос немедленной синхронизации из UI. Обработчик API не может сам сходить
/// в сеть (он держит блокировку базы), поэтому просто поднимает флаг, а цикл
/// обмена подхватывает его в течение секунды.
static SYNC_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub const MAX_PEERS: i64 = 32;
const TRANSPORT_BUNDLE_AAD: &[u8] = b"everyday-sync-bundle\0v2\0XChaCha20-Poly1305";
const TRANSPORT_BUNDLE_LIMIT: usize = 30 * 1024 * 1024;

pub fn request_sync_now() {
    SYNC_NOW.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn take_sync_request() -> bool {
    SYNC_NOW.swap(false, std::sync::atomic::Ordering::Relaxed)
}

pub fn kv_get(conn: &Connection, k: &str) -> Option<String> {
    conn.query_row("SELECT v FROM kv WHERE k=?1", params![k], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub fn kv_set(conn: &Connection, k: &str, v: &str) {
    let _ = conn.execute(
        "INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
        params![k, v],
    );
}

pub fn metric_add(conn: &Connection, key: &str, amount: u64) {
    let current = kv_get(conn, key)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    kv_set(conn, key, &current.saturating_add(amount).to_string());
}

fn metric(conn: &Connection, key: &str) -> u64 {
    kv_get(conn, key)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn item_comment_payload_hash(record: &Value) -> anyhow::Result<String> {
    let required = |name: &str| {
        record
            .get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("item comment has no {name}"))
    };
    let payload = json!({
        "domain":"everyday/item-comment/v1",
        "workspaceGuid":required("workspaceGuid")?, "itemGuid":required("itemGuid")?,
        "authorGuid":required("authorGuid")?, "guid":required("guid")?,
        "text":required("text")?, "createdAt":required("createdAt")?,
    });
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload)?)
    ))
}

fn item_comment_record_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/item-comment-record/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}

fn fault_payload_hash(record: &Value) -> anyhow::Result<String> {
    let text = |name: &str| {
        record
            .get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("fault record has no {name}"))
    };
    let payload = json!({
        "domain":"everyday/fault-record/v1","faultGuid":text("faultGuid")?,
        "parentHash":record.get("parentHash").filter(|v|!v.is_null()).and_then(Value::as_str),
        "depth":record.get("depth").and_then(Value::as_i64).ok_or_else(||anyhow::anyhow!("fault record has no depth"))?,
        "workspaceGuid":text("workspaceGuid")?,"itemGuid":text("itemGuid")?,
        "reporterGuid":text("reporterGuid")?,"actorGuid":text("actorGuid")?,
        "severity":text("severity")?,"description":text("description")?,
        "photoUrl":record.get("photoUrl").filter(|v|!v.is_null()).and_then(Value::as_str),
        "status":text("status")?,
        "resolution":record.get("resolution").filter(|v|!v.is_null()).and_then(Value::as_str),
        "createdAt":text("createdAt")?,
    });
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload)?)
    ))
}

fn fault_record_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/fault-record-ledger/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}

fn change_payload_hash(record: &Value) -> anyhow::Result<String> {
    let get = |name: &str| {
        record
            .get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("change record has no {name}"))
    };
    let payload = json!({"domain":"everyday/change-request-record/v1","requestGuid":get("requestGuid")?,
        "parentHash":record.get("parentHash").filter(|v|!v.is_null()).and_then(Value::as_str),
        "depth":record.get("depth").and_then(Value::as_i64).ok_or_else(||anyhow::anyhow!("change depth missing"))?,
        "workspaceGuid":get("workspaceGuid")?,"itemGuid":get("itemGuid")?,"requesterGuid":get("requesterGuid")?,"actorGuid":get("actorGuid")?,
        "patch":record.get("patch").filter(|v|v.is_object()).ok_or_else(||anyhow::anyhow!("change patch missing"))?,
        "before":record.get("before").filter(|v|v.is_object()).ok_or_else(||anyhow::anyhow!("change before missing"))?,
        "comment":record.get("comment").filter(|v|!v.is_null()).and_then(Value::as_str),"status":get("status")?,
        "reason":record.get("reason").filter(|v|!v.is_null()).and_then(Value::as_str),"createdAt":get("createdAt")?});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload)?)
    ))
}

fn change_record_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/change-request-record-ledger/v1\n{payload_hash}\n{ledger_hash}")
                .as_bytes()
        )
    )
}

fn config_payload_hash(record: &Value) -> anyhow::Result<String> {
    let get = |name: &str| {
        record
            .get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("config version has no {name}"))
    };
    let payload = json!({"domain":"everyday/config-version/v1","kind":get("kind")?,"entityGuid":get("entityGuid")?,
        "parentHash":record.get("parentHash").filter(|v|!v.is_null()).and_then(Value::as_str),"depth":record.get("depth").and_then(Value::as_i64).ok_or_else(||anyhow::anyhow!("config depth missing"))?,
        "workspaceGuid":get("workspaceGuid")?,"actorGuid":get("actorGuid")?,"active":record.get("active").and_then(Value::as_bool).ok_or_else(||anyhow::anyhow!("config active missing"))?,
        "fields":record.get("fields").filter(|v|v.is_object()).ok_or_else(||anyhow::anyhow!("config fields missing"))?,"updatedAt":get("updatedAt")?});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload)?)
    ))
}
fn config_version_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/config-version-ledger/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}

fn item_state_payload_hash(record: &Value) -> anyhow::Result<String> {
    let payload = json!({"domain":"everyday/item-state/v1","itemGuid":record.get("itemGuid"),"parentHash":record.get("parentHash"),"depth":record.get("depth"),"workspaceGuid":record.get("workspaceGuid"),"actorGuid":record.get("actorGuid"),"fields":record.get("fields"),"updatedAt":record.get("updatedAt")});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload)?)
    ))
}

fn item_state_version_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/item-state-ledger/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}
fn organization_node_payload_hash(record: &Value) -> anyhow::Result<String> {
    let payload = json!({"domain":"everyday/organization-node/v1","nodeGuid":record.get("nodeGuid"),"parentHash":record.get("parentHash"),"depth":record.get("depth"),"workspaceGuid":record.get("workspaceGuid"),"actorGuid":record.get("actorGuid"),"active":record.get("active"),"fields":record.get("fields"),"updatedAt":record.get("updatedAt")});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload)?)
    ))
}
fn organization_node_version_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/organization-node-ledger/v1\n{payload_hash}\n{ledger_hash}")
                .as_bytes()
        )
    )
}

fn next_journal_sequence(conn: &Connection) -> u64 {
    let next = kv_get(conn, "sync_journal_sequence")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_add(1);
    kv_set(conn, "sync_journal_sequence", &next.to_string());
    next
}

#[derive(Clone, Debug, Default)]
struct MembershipFields {
    rights: Option<String>,
    position: Option<String>,
    role_name: Option<String>,
    personnel_number: Option<String>,
}

impl MembershipFields {
    fn from_json(value: &Value) -> Self {
        Self {
            rights: value
                .get("rights")
                .and_then(Value::as_str)
                .map(str::to_owned),
            position: value
                .get("position")
                .and_then(Value::as_str)
                .map(str::to_owned),
            role_name: value
                .get("roleName")
                .and_then(Value::as_str)
                .map(str::to_owned),
            personnel_number: value
                .get("personnelNumber")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn custody_entry_hash(
    workspace_guid: &str,
    item_guid: &str,
    user_guid: &str,
    quantity_delta: f64,
    due_at: Option<&str>,
    comment: Option<&str>,
    photo_url: Option<&str>,
    ledger_hash: &str,
    created_at: &str,
) -> String {
    let canonical = json!([
        "everyday/custody-entry/v1",
        workspace_guid,
        item_guid,
        user_guid,
        quantity_delta,
        due_at,
        comment,
        photo_url,
        ledger_hash,
        created_at
    ]);
    hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical).unwrap_or_default(),
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn record_custody_entry(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    user_id: i64,
    quantity_delta: f64,
    due_at: Option<&str>,
    comment: Option<&str>,
    photo_url: Option<&str>,
    ledger_event: &Value,
) -> anyhow::Result<Value> {
    if !quantity_delta.is_finite() || quantity_delta.abs() < 1e-9 {
        anyhow::bail!("custody quantity must be finite and non-zero");
    }
    let workspace_guid = ledger::guid(conn, "workspaces", workspace_id)?;
    let item_guid = ledger::guid(conn, "items", item_id)?;
    let user_guid = ledger::guid(conn, "users", user_id)?;
    let ledger_hash = ledger_event
        .get("opId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("custody entry has no ledger hash"))?;
    let created_at = ledger_event
        .get("createdAt")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("custody entry has no timestamp"))?;
    let entry_hash = custody_entry_hash(
        &workspace_guid,
        &item_guid,
        &user_guid,
        quantity_delta,
        due_at,
        comment,
        photo_url,
        ledger_hash,
        created_at,
    );
    conn.execute(
        "INSERT INTO custody_entries(entry_hash,workspace_guid,item_guid,user_guid,quantity_delta,due_at,comment,photo_url,ledger_hash,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![entry_hash,workspace_guid,item_guid,user_guid,quantity_delta,due_at,comment,photo_url,ledger_hash,created_at],
    )?;
    Ok(json!({
        "entryHash":entry_hash,"workspaceGuid":workspace_guid,"itemGuid":item_guid,
        "userGuid":user_guid,"quantityDelta":quantity_delta,"dueAt":due_at,
        "comment":comment,"photoUrl":photo_url,"ledgerHash":ledger_hash,"createdAt":created_at
    }))
}

fn item_tombstone_hash(
    workspace_guid: &str,
    item_guid: &str,
    actor_guid: &str,
    ledger_hash: &str,
    deleted_at: &str,
) -> String {
    let canonical = json!([
        "everyday/item-tombstone/v1",
        workspace_guid,
        item_guid,
        actor_guid,
        ledger_hash,
        deleted_at
    ]);
    hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical).unwrap_or_default(),
    ))
}

pub fn record_item_tombstone(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    actor_id: i64,
    ledger_event: &Value,
) -> anyhow::Result<Value> {
    let workspace_guid = ledger::guid(conn, "workspaces", workspace_id)?;
    let item_guid = ledger::guid(conn, "items", item_id)?;
    let actor_guid = ledger::guid(conn, "users", actor_id)?;
    let ledger_hash = ledger_event
        .get("opId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("item tombstone has no ledger hash"))?;
    let deleted_at = ledger_event
        .get("createdAt")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("item tombstone has no timestamp"))?;
    let tombstone_hash = item_tombstone_hash(
        &workspace_guid,
        &item_guid,
        &actor_guid,
        ledger_hash,
        deleted_at,
    );
    conn.execute(
        "INSERT INTO item_tombstones(item_guid,workspace_guid,actor_guid,ledger_hash,deleted_at,tombstone_hash)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![item_guid,workspace_guid,actor_guid,ledger_hash,deleted_at,tombstone_hash],
    )?;
    conn.execute(
        "UPDATE items SET archived=1,archived_at=?1 WHERE id=?2",
        params![deleted_at, item_id],
    )?;
    Ok(json!({
        "itemGuid":item_guid,"workspaceGuid":workspace_guid,"actorGuid":actor_guid,
        "ledgerHash":ledger_hash,"deletedAt":deleted_at,"tombstoneHash":tombstone_hash
    }))
}

struct StoredMembershipVersion {
    revision: i64,
    version_hash: String,
    ledger_hash: Option<String>,
    updated_at: String,
    fields: MembershipFields,
}

fn membership_version_hash(
    workspace_guid: &str,
    user_guid: &str,
    revision: i64,
    active: bool,
    fields: &MembershipFields,
    ledger_hash: Option<&str>,
) -> String {
    let canonical = json!([
        "everyday/membership/v1",
        workspace_guid,
        user_guid,
        revision,
        active,
        fields.rights,
        fields.position,
        fields.role_name,
        fields.personnel_number,
        ledger_hash
    ]);
    hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical).unwrap_or_default(),
    ))
}

pub fn record_membership_version(
    conn: &Connection,
    workspace_id: i64,
    user_id: i64,
    active: bool,
    ledger_hash: Option<&str>,
    newly_created: bool,
) -> anyhow::Result<Value> {
    let workspace_guid = ledger::guid(conn, "workspaces", workspace_id)?;
    let user_guid = ledger::guid(conn, "users", user_id)?;
    let previous: Option<i64> = conn
        .query_row(
            "SELECT revision FROM membership_versions WHERE workspace_guid=?1 AND user_guid=?2",
            params![workspace_guid, user_guid],
            |row| row.get(0),
        )
        .optional()?;
    let revision = previous
        .unwrap_or(if newly_created { 0 } else { 1 })
        .saturating_add(1);
    let membership: Option<MembershipFields> = if active {
        conn.query_row(
            "SELECT rights_json,position,role_name,personnel_number FROM user_workspaces
             WHERE workspace_id=?1 AND user_id=?2",
            params![workspace_id, user_id],
            |row| {
                Ok(MembershipFields {
                    rights: row.get(0)?,
                    position: row.get(1)?,
                    role_name: row.get(2)?,
                    personnel_number: row.get(3)?,
                })
            },
        )
        .optional()?
    } else {
        None
    };
    if active && membership.is_none() {
        anyhow::bail!("active membership is missing");
    }
    let fields = membership.unwrap_or_default();
    let version_hash = membership_version_hash(
        &workspace_guid,
        &user_guid,
        revision,
        active,
        &fields,
        ledger_hash,
    );
    let updated_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO membership_versions(workspace_guid,user_guid,revision,active,rights_json,position,role_name,personnel_number,ledger_hash,version_hash,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(workspace_guid,user_guid) DO UPDATE SET revision=excluded.revision,
           active=excluded.active,rights_json=excluded.rights_json,position=excluded.position,
           role_name=excluded.role_name,personnel_number=excluded.personnel_number,
           ledger_hash=excluded.ledger_hash,version_hash=excluded.version_hash,updated_at=excluded.updated_at",
        params![workspace_guid,user_guid,revision,i64::from(active),fields.rights,fields.position,fields.role_name,
            fields.personnel_number,ledger_hash,version_hash,updated_at],
    )?;
    Ok(json!({
        "workspaceGuid":workspace_guid,"userGuid":user_guid,"revision":revision,
        "active":active,"rights":fields.rights,"position":fields.position,"roleName":fields.role_name,
        "personnelNumber":fields.personnel_number,"ledgerHash":ledger_hash,
        "versionHash":version_hash,"updatedAt":updated_at
    }))
}

pub fn ensure_node(conn: &Connection) -> (String, String) {
    if let (Some(id), Some(name)) = (kv_get(conn, "node_id"), kv_get(conn, "node_name")) {
        return (id, name);
    }
    let id = uuid::Uuid::new_v4().to_string().replace('-', "");
    let name = std::env::var("MESHKEEPER_NAME").unwrap_or_else(|_| {
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "MeshKeeper".into())
    });
    kv_set(conn, "node_id", &id);
    kv_set(conn, "node_name", &name);
    (id, name)
}

fn guid_of(conn: &Connection, table: &str, id: i64) -> String {
    let sql = format!("SELECT guid FROM {table} WHERE id=?1");
    conn.query_row(&sql, params![id], |r| r.get::<_, Option<String>>(0))
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let g = uuid::Uuid::new_v4().to_string().replace('-', "");
            let _ = conn.execute(
                &format!("UPDATE {table} SET guid=?1 WHERE id=?2"),
                params![g, id],
            );
            g
        })
}

fn sale_offer_payload(record: &Value) -> anyhow::Result<Value> {
    let required = |field: &str| {
        record
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("sale offer has no {field}"))
    };
    let amount = record
        .get("bitAmount")
        .and_then(Value::as_i64)
        .filter(|amount| *amount > 0)
        .ok_or_else(|| anyhow::anyhow!("sale offer amount is invalid"))?;
    let quantity = match record.get("quantity") {
        None | Some(Value::Null) => Value::Null,
        Some(value) => {
            let quantity = value
                .as_f64()
                .filter(|quantity| quantity.is_finite() && *quantity > 0.0)
                .ok_or_else(|| anyhow::anyhow!("sale offer quantity is invalid"))?;
            json!(quantity)
        }
    };
    Ok(json!({
        "domain":"everyday/bit-sale-offer/v1",
        "offerGuid":required("offerGuid")?, "workspaceGuid":required("workspaceGuid")?,
        "itemGuid":required("itemGuid")?, "sellerGuid":required("sellerGuid")?,
        "buyerGuid":required("buyerGuid")?, "bitAmount":amount,
        "quantity":quantity,
        "comment":record.get("comment").cloned().unwrap_or(Value::Null),
        "createdAt":required("createdAt")?, "ledgerHash":required("ledgerHash")?,
        "status":record.get("status").cloned().unwrap_or_else(||json!("pending")),
        "decisionLedgerHash":record.get("decisionLedgerHash").cloned().unwrap_or(Value::Null),
        "bitTransactionGuid":record.get("bitTransactionGuid").cloned().unwrap_or(Value::Null),
    }))
}

fn sale_offer_hash(record: &Value) -> anyhow::Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(
        &sale_offer_payload(record)?,
    )?)))
}

pub fn hello(conn: &Connection) -> Value {
    let (id, name) = ensure_node(conn);
    json!({
        "ok": true,
        "nodeId": id,
        "name": name,
        "protocol": "meshkeeper-sync/3",
        "ledger": "signed-account-chains",
        "features": ["signed-snapshot", "account-frontier", "full-fallback", "node-key-approval"],
        "nodePublicKey": ledger::node_public_key(conn).ok()
    })
}

pub fn frontier(conn: &Connection) -> Value {
    let mut heads = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT w.guid,h.pubkey,h.hash
         FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id
         WHERE h.pubkey IS NOT NULL AND h.id=(
           SELECT MAX(h2.id) FROM history_entries h2
           WHERE h2.workspace_id=h.workspace_id AND h2.pubkey=h.pubkey)
         ORDER BY w.guid,h.pubkey",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok(json!({
                "workspaceGuid": row.get::<_, Option<String>>(0)?,
                "publicKey": row.get::<_, String>(1)?,
                "head": row.get::<_, String>(2)?,
            }))
        }) {
            heads.extend(rows.flatten());
        }
    }
    Value::Array(heads)
}

fn retain_after_frontier(history: &mut Vec<Value>, recipient_frontier: &Value) {
    let known: HashMap<(String, String), String> = recipient_frontier
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                (
                    entry.get("workspaceGuid")?.as_str()?.to_string(),
                    entry.get("publicKey")?.as_str()?.to_string(),
                ),
                entry.get("head")?.as_str()?.to_string(),
            ))
        })
        .collect();
    // Неизвестная/расходящаяся голова означает полный fallback этой цепочки.
    let locally_found: HashSet<(String, String)> = history
        .iter()
        .filter_map(|event| {
            let key = (
                event.get("workspaceGuid")?.as_str()?.to_string(),
                event.get("pubkey")?.as_str()?.to_string(),
            );
            (known.get(&key).map(String::as_str) == event.get("opId").and_then(Value::as_str))
                .then_some(key)
        })
        .collect();
    let mut reached = HashSet::new();
    history.retain(|event| {
        let Some(key) = event
            .get("workspaceGuid")
            .and_then(Value::as_str)
            .zip(event.get("pubkey").and_then(Value::as_str))
            .map(|(workspace, key)| (workspace.to_string(), key.to_string()))
        else {
            return true;
        };
        if !locally_found.contains(&key) {
            return true;
        }
        if reached.contains(&key) {
            return true;
        }
        if known.get(&key).map(String::as_str) == event.get("opId").and_then(Value::as_str) {
            reached.insert(key);
        }
        false
    });
}

pub fn export_journal_since(conn: &Connection, recipient_frontier: Option<&Value>) -> Value {
    let (node_id, name) = ensure_node(conn);
    let _ = db::fill_guids(conn);
    let mut workspaces = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT id, name, timezone, internal_id_prefix, comment, guid FROM workspaces")
    {
        for row in stmt
            .query_map([], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "timezone": r.get::<_, String>(2)?,
                    "internalIdPrefix": r.get::<_, String>(3)?,
                    "comment": r.get::<_, Option<String>>(4)?,
                    "guid": r.get::<_, Option<String>>(5)?,
                }))
            })
            .into_iter()
            .flatten()
            .flatten()
        {
            workspaces.push(row);
        }
    }
    let mut organization_nodes = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id,guid,workspace_id,parent_id,kind,name,tab_label,responsible_user_id,display_order,color,icon,archived,created_at,updated_at FROM organization_nodes",
    ) {
        for row in stmt.query_map([], |r| {
            let ws: i64 = r.get(2)?;
            let parent: Option<i64> = r.get(3)?;
            let responsible: Option<i64> = r.get(7)?;
            Ok(json!({
                "guid": r.get::<_,String>(1)?,
                "workspaceGuid": guid_of(conn,"workspaces",ws),
                "parentGuid": parent.map(|id| guid_of(conn,"organization_nodes",id)),
                "kind": r.get::<_,String>(4)?, "name": r.get::<_,String>(5)?,
                "tabLabel": r.get::<_,Option<String>>(6)?,
                "responsibleGuid": responsible.map(|id| guid_of(conn,"users",id)),
                "displayOrder": r.get::<_,i64>(8)?, "color": r.get::<_,Option<String>>(9)?,
                "icon": r.get::<_,Option<String>>(10)?, "archived": r.get::<_,i64>(11)? != 0,
                "createdAt": r.get::<_,String>(12)?, "updatedAt": r.get::<_,String>(13)?,
            }))
        }).into_iter().flatten().flatten() {
            organization_nodes.push(row);
        }
    }
    let mut organization_node_versions = Vec::new();
    if let Ok(mut statement)=conn.prepare("SELECT version_hash,node_guid,parent_hash,depth,workspace_guid,actor_guid,active,fields_json,payload_hash,ledger_hash,updated_at FROM organization_node_versions ORDER BY node_guid,depth,version_hash"){if let Ok(rows)=statement.query_map([],|r|{let fields:String=r.get(7)?;Ok(json!({"versionHash":r.get::<_,String>(0)?,"nodeGuid":r.get::<_,String>(1)?,"parentHash":r.get::<_,Option<String>>(2)?,"depth":r.get::<_,i64>(3)?,"workspaceGuid":r.get::<_,String>(4)?,"actorGuid":r.get::<_,String>(5)?,"active":r.get::<_,i64>(6)?!=0,"fields":serde_json::from_str::<Value>(&fields).unwrap_or(Value::Null),"payloadHash":r.get::<_,String>(8)?,"ledgerHash":r.get::<_,String>(9)?,"updatedAt":r.get::<_,String>(10)?}))}){organization_node_versions.extend(rows.flatten());}}
    let mut inventory_records = Vec::new();
    if let Ok(mut statement)=conn.prepare("SELECT record_hash,session_guid,workspace_guid,actor_guid,kind,item_guid,fields_json,payload_hash,ledger_hash,created_at FROM inventory_records ORDER BY created_at,record_hash") {
        if let Ok(rows)=statement.query_map([],|r|{let fields:String=r.get(6)?;Ok(json!({"recordHash":r.get::<_,String>(0)?,"sessionGuid":r.get::<_,String>(1)?,"workspaceGuid":r.get::<_,String>(2)?,"actorGuid":r.get::<_,String>(3)?,"kind":r.get::<_,String>(4)?,"itemGuid":r.get::<_,Option<String>>(5)?,"fields":serde_json::from_str::<Value>(&fields).unwrap_or(Value::Null),"payloadHash":r.get::<_,String>(7)?,"ledgerHash":r.get::<_,String>(8)?,"createdAt":r.get::<_,String>(9)?}))}) { inventory_records.extend(rows.flatten()); }
    }
    let mut sale_offers = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT guid,workspace_id,item_id,from_user_id,to_user_id,bit_amount,comment,created_at,prepare_ledger_hash,status,accept_ledger_hash,bit_transaction_guid,quantity
         FROM transfers WHERE guid IS NOT NULL AND bit_amount IS NOT NULL AND status IN ('pending','accepted','rejected')
         ORDER BY created_at,guid",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            let mut record = json!({
                "offerGuid":row.get::<_,String>(0)?,
                "workspaceGuid":guid_of(conn,"workspaces",row.get(1)?),
                "itemGuid":guid_of(conn,"items",row.get(2)?),
                "sellerGuid":guid_of(conn,"users",row.get(3)?),
                "buyerGuid":guid_of(conn,"users",row.get(4)?),
                "bitAmount":row.get::<_,i64>(5)?, "comment":row.get::<_,Option<String>>(6)?,
                "createdAt":row.get::<_,String>(7)?, "ledgerHash":row.get::<_,String>(8)?,
                "status":row.get::<_,String>(9)?,
                "decisionLedgerHash":row.get::<_,Option<String>>(10)?,
                "bitTransactionGuid":row.get::<_,Option<String>>(11)?,
                "quantity":row.get::<_,Option<f64>>(12)?,
            });
            record["recordHash"] = json!(sale_offer_hash(&record).unwrap_or_default());
            Ok(record)
        }) {
            sale_offers.extend(rows.flatten().filter(|record| {
                record.get("recordHash").and_then(Value::as_str).is_some_and(|hash| !hash.is_empty())
            }));
        }
    }
    let mut users = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id, full_name, position, phone, status, role_rights, checkout_policy, guid, password_hash FROM users") {
        for row in stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "fullName": r.get::<_, String>(1)?,
                "position": r.get::<_, Option<String>>(2)?,
                "phone": r.get::<_, String>(3)?,
                "status": r.get::<_, String>(4)?,
                "roleRights": r.get::<_, Option<String>>(5)?,
                "checkoutPolicy": r.get::<_, Option<String>>(6)?,
                "guid": r.get::<_, Option<String>>(7)?,
                "passwordHash": r.get::<_, Option<String>>(8)?,
            }))
        }).into_iter().flatten().flatten() {
            users.push(row);
        }
    }
    let mut devices = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT d.device_id,u.guid,d.name,d.public_key,d.created_at,d.revoked_at
         FROM user_devices d JOIN users u ON u.id=d.user_id ORDER BY d.device_id",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok(json!({
                "deviceId":row.get::<_,String>(0)?,"userGuid":row.get::<_,String>(1)?,
                "name":row.get::<_,String>(2)?,"publicKey":row.get::<_,String>(3)?,
                "createdAt":row.get::<_,String>(4)?,"revokedAt":row.get::<_,Option<String>>(5)?,
            }))
        }) {
            devices.extend(rows.flatten());
        }
    }
    let mut items = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, internal_id, title, category_id, status_id, responsible_user_id, workspace_id, serial_number, qr_code, due_at, guid, calibrated_until, min_quantity, quantitative, quantity, unit, cost, comment, source_system, external_id, metadata_json, organization_node_id,archived,archived_at,brand_id,building_site_id,storage_id FROM items",
    ) {
        for row in stmt.query_map([], |r| {
            let id: i64 = r.get(0)?;
            let resp: Option<i64> = r.get(5)?;
            let ws: i64 = r.get(6)?;
            let st: Option<i64> = r.get(4)?;
            let slug: Option<String> = st.and_then(|sid| {
                conn.query_row("SELECT slug FROM statuses WHERE id=?1", params![sid], |x| x.get(0)).ok()
            });
            Ok(json!({
                "guid": r.get::<_, Option<String>>(10)?,
                "internalId": r.get::<_, String>(1)?,
                "title": r.get::<_, String>(2)?,
                "workspaceGuid": guid_of(conn, "workspaces", ws),
                "responsibleGuid": resp.map(|u| guid_of(conn, "users", u)),
                "organizationNodeGuid": r.get::<_,Option<i64>>(21)?.map(|node| guid_of(conn,"organization_nodes",node)),
                "categoryGuid": r.get::<_,Option<i64>>(3)?.map(|id| guid_of(conn,"categories",id)),
                "brandGuid": r.get::<_,Option<i64>>(24)?.map(|id| guid_of(conn,"brands",id)),
                "buildingSiteGuid": r.get::<_,Option<i64>>(25)?.map(|id| guid_of(conn,"building_sites",id)),
                "storageGuid": r.get::<_,Option<i64>>(26)?.map(|id| guid_of(conn,"storages",id)),
                "storageName": r.get::<_,Option<i64>>(26)?.and_then(|id|conn.query_row("SELECT name FROM storages WHERE id=?1",[id],|row|row.get::<_,String>(0)).ok()),
                "serialNumber": r.get::<_, Option<String>>(7)?,
                "qrCode": r.get::<_, Option<String>>(8)?,
                "dueAt": r.get::<_, Option<String>>(9)?,
                "calibratedUntil": r.get::<_, Option<String>>(11)?,
                "minQuantity": r.get::<_, Option<f64>>(12)?,
                "quantitative": r.get::<_, i64>(13)? != 0,
                "quantity": r.get::<_, Option<f64>>(14)?,
                "unit": r.get::<_, Option<String>>(15)?,
                "cost": r.get::<_, Option<f64>>(16)?,
                "comment": r.get::<_, Option<String>>(17)?,
                "sourceSystem": r.get::<_, Option<String>>(18)?,
                "externalId": r.get::<_, Option<String>>(19)?,
                "metadata": r.get::<_, Option<String>>(20)?
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .unwrap_or_else(|| json!({})),
                "statusSlug": slug,
                "statusGuid": st.map(|id|guid_of(conn,"statuses",id)),
                "archived": r.get::<_,i64>(22)? != 0,
                "archivedAt": r.get::<_,Option<String>>(23)?,
                "localId": id,
            }))
        }).into_iter().flatten().flatten() {
            items.push(row);
        }
    }
    let mut history = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, workspace_id, item_id, type, actor_user_id, from_label, to_label, quantity_delta, comment, hash, created_at, guid, prev_hash, signature, pubkey,event_version,request_device_id,request_public_key,request_nonce,request_signature,request_hash,request_timestamp,request_path,request_body FROM history_entries ORDER BY id",
    ) {
        for row in stmt.query_map([], |r| {
            let ws: i64 = r.get(1)?;
            let item: Option<i64> = r.get(2)?;
            let actor: i64 = r.get(4)?;
            Ok(json!({
                "workspaceGuid": guid_of(conn, "workspaces", ws),
                "itemGuid": item.map(|i| guid_of(conn, "items", i)),
                "type": r.get::<_, String>(3)?,
                "actorGuid": guid_of(conn, "users", actor),
                "fromLabel": r.get::<_, Option<String>>(5)?,
                "toLabel": r.get::<_, Option<String>>(6)?,
                "quantityDelta": r.get::<_, Option<f64>>(7)?,
                "comment": r.get::<_, Option<String>>(8)?,
                "opId": r.get::<_, String>(9)?,
                "createdAt": r.get::<_, String>(10)?,
                "guid": r.get::<_, Option<String>>(11)?,
                "prevHash": r.get::<_, Option<String>>(12)?,
                "signature": r.get::<_, Option<String>>(13)?,
                "pubkey": r.get::<_, Option<String>>(14)?,
                "eventVersion": r.get::<_, i64>(15)?,
                "requestDeviceId": r.get::<_, Option<String>>(16)?,
                "requestPublicKey": r.get::<_, Option<String>>(17)?,
                "requestNonce": r.get::<_, Option<String>>(18)?,
                "requestSignature": r.get::<_, Option<String>>(19)?,
                "requestHash": r.get::<_, Option<String>>(20)?,
                "requestTimestamp": r.get::<_, Option<String>>(21)?,
                "requestPath": r.get::<_, Option<String>>(22)?,
                "requestBody": r.get::<_, Option<String>>(23)?,
            }))
        }).into_iter().flatten().flatten() {
            history.push(row);
        }
    }
    if let Some(recipient_frontier) = recipient_frontier {
        let required_offer_events: HashSet<&str> = sale_offers
            .iter()
            .flat_map(|offer| [offer.get("ledgerHash"), offer.get("decisionLedgerHash")])
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let retained_offer_events: Vec<Value> = history
            .iter()
            .filter(|event| {
                event
                    .get("opId")
                    .and_then(Value::as_str)
                    .is_some_and(|hash| required_offer_events.contains(hash))
            })
            .cloned()
            .collect();
        retain_after_frontier(&mut history, recipient_frontier);
        let included: HashSet<String> = history
            .iter()
            .filter_map(|event| event.get("opId").and_then(Value::as_str).map(str::to_owned))
            .collect();
        history.extend(retained_offer_events.into_iter().filter(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .is_some_and(|hash| !included.contains(hash))
        }));
    }
    let mut invites = Vec::new();
    // Реплицируем возможность локального onboarding, но не сам bearer secret.
    // Получатель доказывает владение токеном, предъявляя значение из QR.
    if let Ok(mut stmt) = conn.prepare(
            "SELECT token, workspace_id, role, max_uses, used_count, revoked, created_at, expires_at FROM invites",
        ) {
            for row in stmt
                .query_map([], |r| {
                    let ws: i64 = r.get(1)?;
                    let token: String = r.get(0)?;
                    let digest = token.strip_prefix("sha256:").map(str::to_owned).unwrap_or_else(|| hex::encode(Sha256::digest(token.as_bytes())));
                    Ok(json!({
                        "tokenDigest": digest,
                        "workspaceGuid": guid_of(conn, "workspaces", ws),
                        "role": r.get::<_, String>(2)?,
                        "maxUses": r.get::<_, i64>(3)?,
                        "usedCount": r.get::<_, i64>(4)?,
                        "revoked": r.get::<_, i64>(5)? != 0,
                        "createdAt": r.get::<_, String>(6)?,
                        "expiresAt": r.get::<_, Option<String>>(7)?,
                    }))
                })
                .into_iter()
                .flatten()
                .flatten()
            {
                invites.push(row);
            }
    }
    let mut memberships = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT user_id,workspace_id,rights_json,position,role_name,personnel_number FROM user_workspaces")
    {
        let rows: Vec<_> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .into_iter()
            .flatten()
            .flatten()
            .collect();
        for (user, workspace, rights, position, role_name, personnel_number) in rows {
            let user_guid = guid_of(conn, "users", user);
            let workspace_guid = guid_of(conn, "workspaces", workspace);
            let version: Option<StoredMembershipVersion> = conn
                .query_row(
                    "SELECT revision,version_hash,ledger_hash,updated_at,rights_json,position,role_name,personnel_number FROM membership_versions
                     WHERE workspace_guid=?1 AND user_guid=?2 AND active=1",
                    params![workspace_guid, user_guid],
                    |row| Ok(StoredMembershipVersion {
                        revision: row.get(0)?,
                        version_hash: row.get(1)?,
                        ledger_hash: row.get(2)?,
                        updated_at: row.get(3)?,
                        fields: MembershipFields {
                            rights: row.get(4)?, position: row.get(5)?,
                            role_name: row.get(6)?, personnel_number: row.get(7)?,
                        },
                    }),
                )
                .optional()
                .ok()
                .flatten();
            let version = version.unwrap_or_else(|| {
                let fields = MembershipFields { rights, position, role_name, personnel_number };
                StoredMembershipVersion {
                    revision: 1,
                    version_hash: membership_version_hash(
                        &workspace_guid, &user_guid, 1, true, &fields, None,
                    ),
                    ledger_hash: None,
                    updated_at: "1970-01-01T00:00:00Z".to_string(),
                    fields,
                }
            });
            memberships.push(json!({
                "userGuid":user_guid,"workspaceGuid":workspace_guid,"revision":version.revision,
                "active":true,"rights":version.fields.rights,"position":version.fields.position,
                "roleName":version.fields.role_name,"personnelNumber":version.fields.personnel_number,
                "ledgerHash":version.ledger_hash,"versionHash":version.version_hash,
                "updatedAt":version.updated_at,
            }));
        }
    }
    if let Ok(mut statement) = conn.prepare(
        "SELECT workspace_guid,user_guid,revision,rights_json,position,role_name,personnel_number,
                ledger_hash,version_hash,updated_at FROM membership_versions WHERE active=0",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok(json!({
                "workspaceGuid":row.get::<_,String>(0)?,"userGuid":row.get::<_,String>(1)?,
                "revision":row.get::<_,i64>(2)?,"active":false,
                "rights":row.get::<_,Option<String>>(3)?,"position":row.get::<_,Option<String>>(4)?,
                "roleName":row.get::<_,Option<String>>(5)?,"personnelNumber":row.get::<_,Option<String>>(6)?,
                "ledgerHash":row.get::<_,Option<String>>(7)?,"versionHash":row.get::<_,String>(8)?,
                "updatedAt":row.get::<_,String>(9)?,
            }))
        }) {
            memberships.extend(rows.flatten());
        }
    }
    let mut custody = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT entry_hash,workspace_guid,item_guid,user_guid,quantity_delta,due_at,comment,
                photo_url,ledger_hash,created_at FROM custody_entries ORDER BY created_at,entry_hash",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok(json!({
                "entryHash":row.get::<_,String>(0)?,"workspaceGuid":row.get::<_,String>(1)?,
                "itemGuid":row.get::<_,String>(2)?,"userGuid":row.get::<_,String>(3)?,
                "quantityDelta":row.get::<_,f64>(4)?,"dueAt":row.get::<_,Option<String>>(5)?,
                "comment":row.get::<_,Option<String>>(6)?,"photoUrl":row.get::<_,Option<String>>(7)?,
                "ledgerHash":row.get::<_,String>(8)?,"createdAt":row.get::<_,String>(9)?,
            }))
        }) {
            custody.extend(rows.flatten());
        }
    }
    let mut item_tombstones = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT item_guid,workspace_guid,actor_guid,ledger_hash,deleted_at,tombstone_hash
         FROM item_tombstones ORDER BY deleted_at,item_guid",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok(json!({
                "itemGuid":row.get::<_,String>(0)?,"workspaceGuid":row.get::<_,String>(1)?,
                "actorGuid":row.get::<_,String>(2)?,"ledgerHash":row.get::<_,String>(3)?,
                "deletedAt":row.get::<_,String>(4)?,"tombstoneHash":row.get::<_,String>(5)?,
            }))
        }) {
            item_tombstones.extend(rows.flatten());
        }
    }
    let mut item_comments = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT record_hash,guid,workspace_guid,item_guid,author_guid,text,payload_hash,ledger_hash,created_at
         FROM item_comment_records ORDER BY created_at,guid",
    ) {
        if let Ok(rows) = statement.query_map([], |row| Ok(json!({
            "recordHash":row.get::<_,String>(0)?, "guid":row.get::<_,String>(1)?,
            "workspaceGuid":row.get::<_,String>(2)?, "itemGuid":row.get::<_,String>(3)?,
            "authorGuid":row.get::<_,String>(4)?, "text":row.get::<_,String>(5)?,
            "payloadHash":row.get::<_,String>(6)?, "ledgerHash":row.get::<_,String>(7)?,
            "createdAt":row.get::<_,String>(8)?,
        }))) { item_comments.extend(rows.flatten()); }
    }
    let mut faults = Vec::new();
    if let Ok(mut statement)=conn.prepare("SELECT record_hash,fault_guid,parent_hash,depth,workspace_guid,item_guid,reporter_guid,actor_guid,severity,description,photo_url,status,resolution,payload_hash,ledger_hash,created_at FROM fault_records ORDER BY fault_guid,depth,record_hash") {
        if let Ok(rows)=statement.query_map([],|r|Ok(json!({
            "recordHash":r.get::<_,String>(0)?,"faultGuid":r.get::<_,String>(1)?,"parentHash":r.get::<_,Option<String>>(2)?,"depth":r.get::<_,i64>(3)?,
            "workspaceGuid":r.get::<_,String>(4)?,"itemGuid":r.get::<_,String>(5)?,"reporterGuid":r.get::<_,String>(6)?,"actorGuid":r.get::<_,String>(7)?,
            "severity":r.get::<_,String>(8)?,"description":r.get::<_,String>(9)?,"photoUrl":r.get::<_,Option<String>>(10)?,"status":r.get::<_,String>(11)?,
            "resolution":r.get::<_,Option<String>>(12)?,"payloadHash":r.get::<_,String>(13)?,"ledgerHash":r.get::<_,String>(14)?,"createdAt":r.get::<_,String>(15)?,
        }))) { faults.extend(rows.flatten()); }
    }
    let mut change_requests = Vec::new();
    if let Ok(mut statement)=conn.prepare("SELECT record_hash,request_guid,parent_hash,depth,workspace_guid,item_guid,requester_guid,actor_guid,patch_json,before_json,comment,status,reason,payload_hash,ledger_hash,created_at FROM change_request_records ORDER BY request_guid,depth,record_hash") {
        if let Ok(rows)=statement.query_map([],|r|{
            let patch:String=r.get(8)?; let before:String=r.get(9)?;
            Ok(json!({"recordHash":r.get::<_,String>(0)?,"requestGuid":r.get::<_,String>(1)?,"parentHash":r.get::<_,Option<String>>(2)?,"depth":r.get::<_,i64>(3)?,
                "workspaceGuid":r.get::<_,String>(4)?,"itemGuid":r.get::<_,String>(5)?,"requesterGuid":r.get::<_,String>(6)?,"actorGuid":r.get::<_,String>(7)?,
                "patch":serde_json::from_str::<Value>(&patch).unwrap_or(Value::Null),"before":serde_json::from_str::<Value>(&before).unwrap_or(Value::Null),
                "comment":r.get::<_,Option<String>>(10)?,"status":r.get::<_,String>(11)?,"reason":r.get::<_,Option<String>>(12)?,"payloadHash":r.get::<_,String>(13)?,"ledgerHash":r.get::<_,String>(14)?,"createdAt":r.get::<_,String>(15)?}))
        }) { change_requests.extend(rows.flatten()); }
    }
    let mut config_versions = Vec::new();
    if let Ok(mut statement)=conn.prepare("SELECT version_hash,entity_guid,kind,parent_hash,depth,workspace_guid,actor_guid,active,fields_json,payload_hash,ledger_hash,updated_at FROM config_versions ORDER BY kind,entity_guid,depth,version_hash") {
        if let Ok(rows)=statement.query_map([],|r|{let fields:String=r.get(8)?;Ok(json!({"versionHash":r.get::<_,String>(0)?,"entityGuid":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"parentHash":r.get::<_,Option<String>>(3)?,"depth":r.get::<_,i64>(4)?,"workspaceGuid":r.get::<_,String>(5)?,"actorGuid":r.get::<_,String>(6)?,"active":r.get::<_,i64>(7)?!=0,"fields":serde_json::from_str::<Value>(&fields).unwrap_or(Value::Null),"payloadHash":r.get::<_,String>(9)?,"ledgerHash":r.get::<_,String>(10)?,"updatedAt":r.get::<_,String>(11)?}))}){config_versions.extend(rows.flatten());}
    }
    let mut item_state_versions = Vec::new();
    if let Ok(mut statement)=conn.prepare("SELECT version_hash,item_guid,parent_hash,depth,workspace_guid,actor_guid,fields_json,payload_hash,ledger_hash,updated_at FROM item_state_versions ORDER BY item_guid,depth,version_hash"){
        if let Ok(rows)=statement.query_map([],|r|{let fields:String=r.get(6)?;Ok(json!({"versionHash":r.get::<_,String>(0)?,"itemGuid":r.get::<_,String>(1)?,"parentHash":r.get::<_,Option<String>>(2)?,"depth":r.get::<_,i64>(3)?,"workspaceGuid":r.get::<_,String>(4)?,"actorGuid":r.get::<_,String>(5)?,"fields":serde_json::from_str::<Value>(&fields).unwrap_or(Value::Null),"payloadHash":r.get::<_,String>(7)?,"ledgerHash":r.get::<_,String>(8)?,"updatedAt":r.get::<_,String>(9)?}))}){item_state_versions.extend(rows.flatten());}
    }
    if recipient_frontier.is_some() {
        let sent: HashSet<&str> = history
            .iter()
            .filter_map(|event| event.get("opId").and_then(Value::as_str))
            .collect();
        config_versions.retain(|record| {
            record
                .get("ledgerHash")
                .and_then(Value::as_str)
                .is_some_and(|hash| sent.contains(hash))
        });
        item_state_versions.retain(|record| {
            record
                .get("ledgerHash")
                .and_then(Value::as_str)
                .is_some_and(|hash| sent.contains(hash))
        });
        organization_node_versions.retain(|record| {
            record
                .get("ledgerHash")
                .and_then(Value::as_str)
                .is_some_and(|hash| sent.contains(hash))
        });
        inventory_records.retain(|record| {
            record
                .get("ledgerHash")
                .and_then(Value::as_str)
                .is_some_and(|hash| sent.contains(hash))
        });
    }
    let mut messages = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT guid,workspace_id,user_id,text,attachments_json,ledger_hash,created_at
         FROM chat_messages WHERE ledger_hash IS NOT NULL ORDER BY created_at,guid",
    ) {
        for row in stmt
            .query_map([], |row| {
                let workspace: i64 = row.get(1)?;
                let user: i64 = row.get(2)?;
                Ok(json!({
                    "guid": row.get::<_, String>(0)?,
                    "workspaceGuid": guid_of(conn, "workspaces", workspace),
                    "userGuid": guid_of(conn, "users", user),
                    "text": row.get::<_, String>(3)?,
                    "attachments":serde_json::from_str::<Value>(&row.get::<_,String>(4)?)
                        .unwrap_or_else(|_|json!([])),
                    "ledgerHash": row.get::<_, String>(5)?,
                    "createdAt": row.get::<_, String>(6)?,
                }))
            })
            .into_iter()
            .flatten()
            .flatten()
        {
            messages.push(row);
        }
    }
    let mut photos = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT p.guid,p.item_id,p.url,p.thumb_url,p.sha256,p.is_title,
                (SELECT h.hash FROM history_entries h
                 WHERE h.item_id=p.item_id AND h.type='photo_add' AND h.from_label=p.guid
                   AND h.event_version=3 AND h.request_body IS NOT NULL
                 ORDER BY h.id DESC LIMIT 1)
         FROM item_photos p ORDER BY p.id",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            let item_id: i64 = row.get(1)?;
            Ok(json!({
                "guid": row.get::<_, Option<String>>(0)?,
                "itemGuid": guid_of(conn,"items",item_id),
                "url": row.get::<_, String>(2)?,
                "thumbUrl": row.get::<_, Option<String>>(3)?,
                "sha256": row.get::<_, Option<String>>(4)?,
                "isTitle": row.get::<_, i64>(5)? != 0,
                "ledgerHash": row.get::<_, Option<String>>(6)?,
            }))
        }) {
            photos.extend(rows.flatten());
        }
    }
    let mut documents = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT d.guid,d.item_id,d.name,d.url,d.mime,d.sha256,d.author_id,d.access_level,
                (SELECT h.hash FROM history_entries h
                 WHERE h.item_id=d.item_id AND h.type='document_add' AND h.from_label=d.guid
                   AND h.event_version=3 AND h.request_body IS NOT NULL
                 ORDER BY h.id DESC LIMIT 1)
         FROM item_documents d ORDER BY d.id",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            let item_id: i64 = row.get(1)?;
            let author_id: Option<i64> = row.get(6)?;
            Ok(json!({
                "guid": row.get::<_, Option<String>>(0)?,
                "itemGuid": guid_of(conn,"items",item_id),
                "name": row.get::<_, String>(2)?,
                "url": row.get::<_, String>(3)?,
                "mime": row.get::<_, Option<String>>(4)?,
                "sha256": row.get::<_, Option<String>>(5)?,
                "authorGuid": author_id.map(|id| guid_of(conn,"users",id)),
                "accessLevel": row.get::<_, String>(7)?,
                "ledgerHash": row.get::<_, Option<String>>(8)?,
            }))
        }) {
            documents.extend(rows.flatten());
        }
    }
    let mut journal = json!({
        "v": 2,
        "nodeId": node_id,
        "nodeName": name,
        "nodeUrl": guess_lan_base(),
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "journalSequence": next_journal_sequence(conn),
        "journalScope": "*",
        "membershipMode": "versioned-tombstones/v1",
        "custodyMode": "ledger-delta/v1",
        "itemTombstoneMode": "monotonic/v1",
        "itemCommentMode": "ledger-records/v1",
        "photoMode": "intent-bound-with-explicit-legacy/v1",
        "faultMode": "append-only-branches/v1",
        "changeRequestMode":"portable-branches/v1",
        "configMode":"portable-branches/v1",
        "itemStateMode":"portable-branches/v1",
        "historyMode": if recipient_frontier.is_some() { "delta" } else { "full" },
        "frontier": frontier(conn),
        "workspaces": workspaces,
        "users": users,
        "devices": devices,
        "organizationNodes": organization_nodes,
        "items": items,
        "history": history,
        "invites": invites,
        "memberships": memberships,
        "custody": custody,
        "itemTombstones": item_tombstones,
        "itemComments": item_comments,
        "faults": faults,
        "changeRequests":change_requests,
        "configVersions":config_versions,
        "itemStateVersions":item_state_versions,
        "messages": messages,
        "photos": photos,
        "documents": documents,
        "blobs": crate::content::manifests(conn),
        "contentCatalog": crate::content::catalog(conn),
        "contentProviders": crate::content::provider_manifest(conn),
        "accounting": crate::accounting::export(conn),
        "knowledge": crate::knowledge::export(conn),
    });
    if let Some(object) = journal.as_object_mut() {
        object.insert(
            "chatMode".into(),
            json!("device-signed-with-explicit-legacy/v1"),
        );
        object.insert(
            "documentMode".into(),
            json!("intent-bound-with-explicit-legacy/v1"),
        );
        object.insert("inventoryMode".into(), json!("append-only-records/v1"));
        object.insert("inventoryRecords".into(), Value::Array(inventory_records));
        object.insert("saleOfferMode".into(), json!("device-signed-intent/v1"));
        object.insert("saleOffers".into(), Value::Array(sale_offers));
        object.insert("organizationNodeMode".into(), json!("portable-branches/v1"));
        object.insert(
            "organizationNodeVersions".into(),
            Value::Array(organization_node_versions),
        );
    }
    if let Err(error) = ledger::sign_journal(conn, &mut journal) {
        return json!({"ok": false, "error": format!("Не удалось подписать журнал: {error}")});
    }
    journal
}

pub fn export_journal(conn: &Connection) -> Value {
    export_journal_since(conn, None)
}

fn retain_workspace(entries: &mut Value, allowed: &HashSet<String>) {
    if let Some(rows) = entries.as_array_mut() {
        rows.retain(|row| {
            row.get("workspaceGuid")
                .and_then(Value::as_str)
                .is_some_and(|guid| allowed.contains(guid))
        });
    }
}

fn cas_hashes(value: &Value, out: &mut HashSet<String>) {
    match value {
        Value::String(text) => {
            if let Some(hash) = text.strip_prefix("cas:") {
                if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    out.insert(hash.to_ascii_lowercase());
                }
            }
        }
        Value::Array(values) => values.iter().for_each(|value| cas_hashes(value, out)),
        Value::Object(values) => values.values().for_each(|value| cas_hashes(value, out)),
        _ => {}
    }
}

/// Redacts a signed snapshot to the exact set of organizations authorized for
/// one mesh capability. The result must be signed again by the exporting node.
fn filter_journal_scope(journal: &mut Value, allowed: &HashSet<String>) {
    let Some(object) = journal.as_object_mut() else {
        return;
    };
    if let Some(rows) = object.get_mut("workspaces").and_then(Value::as_array_mut) {
        rows.retain(|row| {
            row.get("guid")
                .and_then(Value::as_str)
                .is_some_and(|guid| allowed.contains(guid))
        });
    }
    for key in [
        "organizationNodes",
        "organizationNodeVersions",
        "items",
        "history",
        "invites",
        "memberships",
        "custody",
        "itemTombstones",
        "itemComments",
        "faults",
        "changeRequests",
        "configVersions",
        "itemStateVersions",
        "inventoryRecords",
        "saleOffers",
        "messages",
        "frontier",
    ] {
        if let Some(value) = object.get_mut(key) {
            retain_workspace(value, allowed);
        }
    }

    let item_guids: HashSet<String> = object
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("guid").and_then(Value::as_str).map(str::to_owned))
        .collect();
    for key in ["photos", "documents"] {
        if let Some(rows) = object.get_mut(key).and_then(Value::as_array_mut) {
            rows.retain(|row| {
                row.get("itemGuid")
                    .and_then(Value::as_str)
                    .is_some_and(|guid| item_guids.contains(guid))
            });
        }
    }

    let mut account_guids = HashSet::new();
    let mut transaction_guids = HashSet::new();
    if let Some(accounting) = object.get_mut("accounting").and_then(Value::as_object_mut) {
        if let Some(accounts) = accounting.get_mut("accounts") {
            retain_workspace(accounts, allowed);
            account_guids.extend(
                accounts
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|row| row.get("guid").and_then(Value::as_str).map(str::to_owned)),
            );
        }
        if let Some(transactions) = accounting.get_mut("transactions") {
            retain_workspace(transactions, allowed);
            transaction_guids.extend(
                transactions
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|row| row.get("guid").and_then(Value::as_str).map(str::to_owned)),
            );
        }
        if let Some(lines) = accounting.get_mut("lines").and_then(Value::as_array_mut) {
            lines.retain(|row| {
                row.get("transactionGuid")
                    .and_then(Value::as_str)
                    .is_some_and(|guid| transaction_guids.contains(guid))
                    && row
                        .get("accountGuid")
                        .and_then(Value::as_str)
                        .is_some_and(|guid| account_guids.contains(guid))
            });
        }
    }

    let mut page_guids = HashSet::new();
    if let Some(knowledge) = object.get_mut("knowledge").and_then(Value::as_object_mut) {
        if let Some(pages) = knowledge.get_mut("pages") {
            retain_workspace(pages, allowed);
            page_guids.extend(
                pages
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|row| row.get("guid").and_then(Value::as_str).map(str::to_owned)),
            );
        }
        if let Some(revisions) = knowledge.get_mut("revisions").and_then(Value::as_array_mut) {
            revisions.retain(|row| {
                row.get("pageGuid")
                    .and_then(Value::as_str)
                    .is_some_and(|guid| page_guids.contains(guid))
            });
        }
    }

    let mut user_guids = HashSet::new();
    for key in ["memberships", "messages", "custody"] {
        if let Some(rows) = object.get(key).and_then(Value::as_array) {
            user_guids.extend(rows.iter().filter_map(|row| {
                row.get("userGuid")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }));
        }
    }
    for (key, field) in [
        ("items", "responsibleGuid"),
        ("history", "actorGuid"),
        ("organizationNodes", "responsibleGuid"),
    ] {
        if let Some(rows) = object.get(key).and_then(Value::as_array) {
            user_guids.extend(
                rows.iter()
                    .filter_map(|row| row.get(field).and_then(Value::as_str).map(str::to_owned)),
            );
        }
    }
    if let Some(accounting) = object.get("accounting") {
        for key in ["accounts", "transactions"] {
            if let Some(rows) = accounting.get(key).and_then(Value::as_array) {
                for row in rows {
                    for field in ["ownerGuid", "actorGuid"] {
                        if let Some(guid) = row.get(field).and_then(Value::as_str) {
                            user_guids.insert(guid.to_owned());
                        }
                    }
                }
            }
        }
    }
    if let Some(revisions) = object
        .get("knowledge")
        .and_then(|v| v.get("revisions"))
        .and_then(Value::as_array)
    {
        user_guids.extend(revisions.iter().filter_map(|row| {
            row.get("authorGuid")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }));
    }
    if let Some(users) = object.get_mut("users").and_then(Value::as_array_mut) {
        users.retain(|row| {
            row.get("guid")
                .and_then(Value::as_str)
                .is_some_and(|guid| user_guids.contains(guid))
        });
    }
    if let Some(devices) = object.get_mut("devices").and_then(Value::as_array_mut) {
        devices.retain(|row| {
            row.get("userGuid")
                .and_then(Value::as_str)
                .is_some_and(|guid| user_guids.contains(guid))
        });
    }

    let mut hashes = HashSet::new();
    for key in ["photos", "documents", "knowledge", "custody"] {
        if let Some(value) = object.get(key) {
            cas_hashes(value, &mut hashes);
        }
    }
    for key in ["blobs", "contentCatalog", "contentProviders"] {
        if let Some(rows) = object.get_mut(key).and_then(Value::as_array_mut) {
            rows.retain(|row| {
                row.get("hash")
                    .and_then(Value::as_str)
                    .is_some_and(|hash| hashes.contains(&hash.to_ascii_lowercase()))
            });
        }
    }
}

pub fn export_journal_scoped(
    conn: &Connection,
    recipient_frontier: Option<&Value>,
    allowed: Option<&HashSet<String>>,
) -> Value {
    let mut journal = export_journal_since(conn, recipient_frontier);
    if let Some(allowed) = allowed {
        filter_journal_scope(&mut journal, allowed);
        let mut scope = allowed.iter().cloned().collect::<Vec<_>>();
        scope.sort();
        journal["journalScope"] =
            Value::String(hex::encode(Sha256::digest(scope.join("\n").as_bytes())));
        if let Err(error) = ledger::sign_journal(conn, &mut journal) {
            return json!({"ok":false,"error":format!("Не удалось подписать scoped-журнал: {error}")});
        }
    }
    journal
}

pub fn journal_within_scope(journal: &Value, allowed: &HashSet<String>) -> bool {
    let mut filtered = journal.clone();
    filter_journal_scope(&mut filtered, allowed);
    for value in [&mut filtered] {
        if let Some(object) = value.as_object_mut() {
            object.remove("journalHash");
            object.remove("journalSignature");
            object.remove("journalPublicKey");
        }
    }
    let mut original = journal.clone();
    if let Some(object) = original.as_object_mut() {
        object.remove("journalHash");
        object.remove("journalSignature");
        object.remove("journalPublicKey");
    }
    filtered == original
}

/// Blob endpoint authorization is checked independently of journal filtering:
/// knowing a hash from another organization must not turn the shared transport
/// token into a cross-organization file oracle.
pub fn content_hash_allowed(conn: &Connection, allowed: &HashSet<String>, hash: &str) -> bool {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let cas = format!("cas:{}", hash.to_ascii_lowercase());
    let direct = conn.query_row(
        "SELECT 1 FROM item_photos p JOIN items i ON i.id=p.item_id JOIN workspaces w ON w.id=i.workspace_id
         WHERE w.guid IN (SELECT value FROM json_each(?1)) AND (lower(p.url)=?2 OR lower(p.thumb_url)=?2)
         UNION ALL
         SELECT 1 FROM item_documents d JOIN items i ON i.id=d.item_id JOIN workspaces w ON w.id=i.workspace_id
         WHERE w.guid IN (SELECT value FROM json_each(?1)) AND lower(d.url)=?2
         UNION ALL
         SELECT 1 FROM custody_entries c JOIN workspaces w ON w.guid=c.workspace_guid
         WHERE w.guid IN (SELECT value FROM json_each(?1)) AND lower(c.photo_url)=?2 LIMIT 1",
        params![serde_json::to_string(&allowed.iter().collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into()), cas],
        |_| Ok(()),
    ).is_ok();
    if direct {
        return true;
    }
    let Ok(mut statement) = conn.prepare(
        "SELECT r.attachments_json FROM knowledge_revisions r
         JOIN knowledge_pages p ON p.guid=r.page_guid JOIN workspaces w ON w.id=p.workspace_id
         WHERE w.guid IN (SELECT value FROM json_each(?1))",
    ) else {
        return false;
    };
    let scope =
        serde_json::to_string(&allowed.iter().collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
    let found = statement
        .query_map([scope], |row| row.get::<_, String>(0))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|raw| {
            serde_json::from_str::<Value>(&raw)
                .ok()
                .is_some_and(|value| {
                    let mut hashes = HashSet::new();
                    cas_hashes(&value, &mut hashes);
                    hashes.contains(&hash.to_ascii_lowercase())
                })
        });
    found
}

/// Транспортно-независимый пакет: его можно передать файлом, Bluetooth Share,
/// Wi-Fi Direct, USB или любым store-and-forward каналом. Бинарные CAS-объекты
/// сюда намеренно не входят; journal содержит только manifests и ссылки.
fn transport_bundle_key(secret: &str) -> anyhow::Result<[u8; 32]> {
    if secret.chars().count() < 32 {
        anyhow::bail!("mesh-токен transport bundle должен содержать не менее 32 символов");
    }
    // HKDF-Extract + single-block HKDF-Expand (RFC 5869), domain-separated
    // from HTTP bearer and per-blob capabilities.
    let mut extract = <Hmac<Sha256> as Mac>::new_from_slice(b"everyday/transport-bundle/salt/v2")?;
    extract.update(secret.as_bytes());
    let prk = extract.finalize().into_bytes();
    let mut expand = <Hmac<Sha256> as Mac>::new_from_slice(&prk)?;
    expand.update(b"everyday/transport-bundle/key/v2");
    expand.update(&[1]);
    Ok(expand.finalize().into_bytes().into())
}

fn encrypt_transport_journal(journal: &Value, secret: &str) -> anyhow::Result<Value> {
    let plaintext = serde_json::to_vec(journal)?;
    if plaintext.len() > TRANSPORT_BUNDLE_LIMIT {
        anyhow::bail!("Transport bundle превышает лимит 30 МБ");
    }
    let mut nonce = [0_u8; 24];
    rand::thread_rng().fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new_from_slice(&transport_bundle_key(secret)?)?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: TRANSPORT_BUNDLE_AAD,
            },
        )
        .map_err(|_| anyhow::anyhow!("Не удалось зашифровать transport bundle"))?;
    Ok(json!({
        "format": "everyday-sync-bundle",
        "version": 2,
        "createdAt": chrono::Utc::now().to_rfc3339(),
        "cipher": "XChaCha20-Poly1305",
        "kdf": "HKDF-SHA256",
        "nonce": STANDARD_NO_PAD.encode(nonce),
        "ciphertext": STANDARD_NO_PAD.encode(ciphertext),
    }))
}

fn decrypt_transport_journal(bundle: &Value, secret: &str) -> anyhow::Result<Value> {
    if bundle.get("cipher").and_then(Value::as_str) != Some("XChaCha20-Poly1305")
        || bundle.get("kdf").and_then(Value::as_str) != Some("HKDF-SHA256")
    {
        anyhow::bail!("Неподдерживаемое шифрование transport bundle");
    }
    let nonce =
        STANDARD_NO_PAD.decode(bundle.get("nonce").and_then(Value::as_str).unwrap_or(""))?;
    let ciphertext = STANDARD_NO_PAD.decode(
        bundle
            .get("ciphertext")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    if nonce.len() != 24 || ciphertext.len() > TRANSPORT_BUNDLE_LIMIT + 16 {
        anyhow::bail!("Некорректный размер transport bundle");
    }
    let cipher = XChaCha20Poly1305::new_from_slice(&transport_bundle_key(secret)?)?;
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: TRANSPORT_BUNDLE_AAD,
            },
        )
        .map_err(|_| anyhow::anyhow!("Неверный mesh-токен или transport bundle повреждён"))?;
    Ok(serde_json::from_slice(&plaintext)?)
}

#[cfg(test)]
pub fn export_transport_bundle(conn: &Connection, secret: Option<&str>) -> Value {
    export_transport_bundle_scoped(conn, secret, None)
}

pub fn export_transport_bundle_scoped(
    conn: &Connection,
    secret: Option<&str>,
    allowed: Option<&HashSet<String>>,
) -> Value {
    if let Ok(public_key) = ledger::node_public_key(conn) {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO trusted_node_keys(public_key,label,source,created_at) VALUES(?1,'Этот узел','local',?2)",
            params![public_key, chrono::Utc::now().to_rfc3339()],
        );
    }
    let Some(secret) = secret else {
        return json!({"ok":false,"error":"Для защищённого offline bundle задайте MESHKEEPER_SYNC_TOKEN"});
    };
    encrypt_transport_journal(&export_journal_scoped(conn, None, allowed), secret)
        .unwrap_or_else(|error| json!({"ok":false,"error":error.to_string()}))
}

#[cfg(test)]
pub fn import_transport_bundle(conn: &Connection, bundle: &Value, secret: Option<&str>) -> Value {
    import_transport_bundle_scoped(conn, bundle, secret, None)
}

pub fn import_transport_bundle_scoped(
    conn: &Connection,
    bundle: &Value,
    secret: Option<&str>,
    allowed: Option<&HashSet<String>>,
) -> Value {
    if bundle.get("format").and_then(Value::as_str) != Some("everyday-sync-bundle") {
        return json!({"ok":false,"error":"Неподдерживаемый формат transport bundle"});
    }
    let version = bundle.get("version").and_then(Value::as_u64);
    let decrypted;
    let journal = if version == Some(2) {
        let Some(secret) = secret else {
            return json!({"ok":false,"error":"Для расшифровки bundle нужен MESHKEEPER_SYNC_TOKEN"});
        };
        decrypted = match decrypt_transport_journal(bundle, secret) {
            Ok(value) => value,
            Err(error) => return json!({"ok":false,"error":error.to_string()}),
        };
        &decrypted
    } else if version == Some(1) {
        let Some(journal) = bundle.get("journal") else {
            return json!({"ok":false,"error":"В transport bundle отсутствует journal"});
        };
        journal
    } else {
        return json!({"ok":false,"error":"Неподдерживаемая версия transport bundle"});
    };
    if serde_json::to_vec(journal).map_or(true, |bytes| bytes.len() > TRANSPORT_BUNDLE_LIMIT) {
        return json!({"ok":false,"error":"Transport bundle превышает лимит 30 МБ"});
    }
    if allowed.is_some_and(|scope| !journal_within_scope(journal, scope)) {
        return json!({"ok":false,"error":"Transport bundle содержит организацию вне capability scope"});
    }
    // Файл не доказывает, что объявленный HTTP-адрес сейчас принадлежит
    // непосредственному отправителю: не добавляем его автоматически в peers.
    // Адреса CAS-провайдеров всё равно приходят в подписанном gossip-каталоге.
    apply_remote_journal(conn, journal, "")
}

fn upsert_workspace(conn: &Connection, w: &Value) -> i64 {
    let guid = w.get("guid").and_then(|v| v.as_str()).unwrap_or("");
    if !guid.is_empty() {
        if let Ok(id) = conn.query_row(
            "SELECT id FROM workspaces WHERE guid=?1",
            params![guid],
            |r| r.get::<_, i64>(0),
        ) {
            let _ = db::ensure_workspace_statuses(conn, id);
            return id;
        }
    }
    let name = w.get("name").and_then(|v| v.as_str()).unwrap_or("Группа");
    let _ = conn.execute(
        "INSERT INTO workspaces (name, timezone, internal_id_prefix, comment, created_at, guid) VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            name,
            w.get("timezone").and_then(|v| v.as_str()).unwrap_or("Europe/Moscow"),
            w.get("internalIdPrefix").and_then(|v| v.as_str()).unwrap_or("ВН-"),
            w.get("comment").and_then(|v| v.as_str()),
            chrono::Utc::now().to_rfc3339(),
            if guid.is_empty() { uuid::Uuid::new_v4().to_string().replace('-', "") } else { guid.to_string() }
        ],
    );
    let id = conn.last_insert_rowid();
    let _ = db::ensure_workspace_statuses(conn, id);
    id
}

fn upsert_user(conn: &Connection, u: &Value) -> i64 {
    let guid = u.get("guid").and_then(|v| v.as_str()).unwrap_or("");
    let phone = u.get("phone").and_then(|v| v.as_str()).unwrap_or("");
    let password_hash = u.get("passwordHash").and_then(|v| v.as_str());
    if !guid.is_empty() {
        if let Ok(id) = conn.query_row("SELECT id FROM users WHERE guid=?1", params![guid], |r| {
            r.get::<_, i64>(0)
        }) {
            fill_missing_password(conn, id, password_hash);
            return id;
        }
    }
    if !phone.is_empty() {
        let want = db::digits_only(phone);
        if let Ok(mut stmt) = conn.prepare("SELECT id, phone FROM users") {
            if let Ok(rows) =
                stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            {
                let found: Vec<(i64, String)> = rows.filter_map(|x| x.ok()).collect();
                if let Some((id, _)) = found.into_iter().find(|(_, p)| db::digits_only(p) == want) {
                    fill_missing_password(conn, id, password_hash);
                    return id;
                }
            }
        }
    }
    let name = u
        .get("fullName")
        .and_then(|v| v.as_str())
        .unwrap_or("Участник");
    let _ = conn.execute(
        "INSERT INTO users (full_name, position, phone, status, role_rights, checkout_policy, guid, password_hash, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            name,
            u.get("position").and_then(|v| v.as_str()),
            if phone.is_empty() { format!("sync-{}", &guid[..8.min(guid.len())]) } else { phone.to_string() },
            u.get("status").and_then(|v| v.as_str()).unwrap_or("active"),
            u.get("roleRights").and_then(|v| v.as_str()).unwrap_or(""),
            u.get("checkoutPolicy").and_then(|v| v.as_str()),
            if guid.is_empty() { uuid::Uuid::new_v4().to_string().replace('-', "") } else { guid.to_string() },
            password_hash,
            chrono::Utc::now().to_rfc3339()
        ],
    );
    conn.last_insert_rowid()
}

/// Finds both locally-created raw capabilities and replicated one-way forms
/// without ever materialising the original token in a journal.
fn find_invite_by_digest(conn: &Connection, digest: &str) -> Option<i64> {
    let mut stmt = conn.prepare("SELECT id,token FROM invites").ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    let wanted = digest.to_ascii_lowercase();
    for row in rows.flatten() {
        let candidate = row
            .1
            .strip_prefix("sha256:")
            .map(str::to_owned)
            .unwrap_or_else(|| hex::encode(Sha256::digest(row.1.as_bytes())));
        if candidate == wanted {
            return Some(row.0);
        }
    }
    None
}

/// Проставляет хеш пароля, если локально его ещё нет. Существующий хеш
/// не трогаем: входящая копия может быть старее локальной.
fn fill_missing_password(conn: &Connection, user_id: i64, incoming: Option<&str>) {
    let Some(hash) = incoming.filter(|h| !h.is_empty()) else {
        return;
    };
    let _ = conn.execute(
        "UPDATE users SET password_hash=?1
         WHERE id=?2 AND (password_hash IS NULL OR password_hash='')",
        params![hash, user_id],
    );
}

fn id_by_guid(conn: &Connection, table: &str, guid: &str) -> Option<i64> {
    if guid.is_empty() {
        return None;
    }
    let sql = format!("SELECT id FROM {table} WHERE guid=?1");
    conn.query_row(&sql, params![guid], |r| r.get(0)).ok()
}

fn status_id(conn: &Connection, ws: i64, slug: &str) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM statuses WHERE workspace_id=?1 AND slug=?2",
        params![ws, slug],
        |r| r.get(0),
    )
    .ok()
}

#[derive(Clone)]
struct CustodyLot {
    quantity: f64,
    due_at: Option<String>,
    comment: Option<String>,
    photo_url: Option<String>,
    created_at: String,
}

fn rebuild_quantitative_holdings(conn: &Connection) -> anyhow::Result<()> {
    let mut statement = conn.prepare(
        "SELECT i.id,u.id,c.quantity_delta,c.due_at,c.comment,c.photo_url,c.created_at
         FROM custody_entries c JOIN items i ON i.guid=c.item_guid
         JOIN users u ON u.guid=c.user_guid WHERE i.quantitative=1
         ORDER BY c.created_at,c.entry_hash",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut balances: HashMap<(i64, i64), Vec<CustodyLot>> = HashMap::new();
    for row in rows {
        let (item, user, delta, due_at, comment, photo_url, created_at) = row?;
        let lots = balances.entry((item, user)).or_default();
        if delta > 0.0 {
            lots.push(CustodyLot {
                quantity: delta,
                due_at,
                comment,
                photo_url,
                created_at,
            });
            continue;
        }
        let mut returned = -delta;
        for lot in lots.iter_mut() {
            if returned <= 1e-9 {
                break;
            }
            let consumed = lot.quantity.min(returned);
            lot.quantity -= consumed;
            returned -= consumed;
        }
        lots.retain(|lot| lot.quantity > 1e-9);
        if returned > 1e-9 {
            anyhow::bail!("custody return exceeds verified holdings for item {item}, user {user}");
        }
    }
    for ((item, user), lots) in balances {
        let (active, rebuilt): (i64, i64) = conn.query_row(
            "SELECT COUNT(*),COALESCE(SUM(sync_rebuilt),0) FROM item_holdings
             WHERE item_id=?1 AND user_id=?2 AND returned_at IS NULL",
            params![item, user],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if active > 0 && active != rebuilt {
            // Локальная pre-upgrade запись содержит metadata, которой нет в старой
            // custody-летописи. Не уничтожаем её автоматически.
            continue;
        }
        conn.execute(
            "DELETE FROM item_holdings WHERE item_id=?1 AND user_id=?2
             AND returned_at IS NULL AND sync_rebuilt=1",
            params![item, user],
        )?;
        for lot in lots {
            conn.execute(
                "INSERT INTO item_holdings(item_id,user_id,quantity,due_at,comment,photo_url,created_at,sync_rebuilt)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,1)",
                params![item,user,lot.quantity,lot.due_at,lot.comment,lot.photo_url,lot.created_at],
            )?;
        }
    }
    Ok(())
}

struct FaultStateRow {
    guid: String,
    item: String,
    workspace: String,
    reporter: String,
    actor: String,
    severity: String,
    description: String,
    photo: Option<String>,
    status: String,
    resolution: Option<String>,
    updated: String,
    created: String,
}

fn rebuild_fault_state(conn: &Connection) -> anyhow::Result<()> {
    let mut statement=conn.prepare("SELECT r.fault_guid,r.item_guid,r.workspace_guid,r.reporter_guid,r.actor_guid,r.severity,r.description,r.photo_url,r.status,r.resolution,r.created_at,
        (SELECT created_at FROM fault_records root WHERE root.fault_guid=r.fault_guid AND root.depth=0 ORDER BY root.record_hash LIMIT 1)
        FROM fault_records r WHERE r.record_hash=(SELECT record_hash FROM fault_records w WHERE w.fault_guid=r.fault_guid ORDER BY depth DESC,record_hash DESC LIMIT 1)")?;
    let rows: Vec<FaultStateRow> = statement
        .query_map([], |r| {
            Ok(FaultStateRow {
                guid: r.get(0)?,
                item: r.get(1)?,
                workspace: r.get(2)?,
                reporter: r.get(3)?,
                actor: r.get(4)?,
                severity: r.get(5)?,
                description: r.get(6)?,
                photo: r.get(7)?,
                status: r.get(8)?,
                resolution: r.get(9)?,
                updated: r.get(10)?,
                created: r.get(11)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    for row in rows {
        let item = id_by_guid(conn, "items", &row.item)
            .ok_or_else(|| anyhow::anyhow!("fault item unavailable"))?;
        let ws = id_by_guid(conn, "workspaces", &row.workspace)
            .ok_or_else(|| anyhow::anyhow!("fault workspace unavailable"))?;
        let reporter = id_by_guid(conn, "users", &row.reporter)
            .ok_or_else(|| anyhow::anyhow!("fault reporter unavailable"))?;
        let actor = id_by_guid(conn, "users", &row.actor)
            .ok_or_else(|| anyhow::anyhow!("fault actor unavailable"))?;
        conn.execute("INSERT OR IGNORE INTO faults(item_id,workspace_id,author_id,severity,description,photo_url,status,resolution,resolver_id,created_at,resolved_at,guid)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,CASE WHEN ?7='open' THEN NULL ELSE ?9 END,?10,CASE WHEN ?7='open' THEN NULL ELSE ?11 END,?12)",
            params![item,ws,reporter,row.severity,row.description,row.photo,row.status,row.resolution,actor,row.created,row.updated,row.guid])?;
        conn.execute("UPDATE faults SET item_id=?1,workspace_id=?2,author_id=?3,severity=?4,description=?5,photo_url=?6,status=?7,resolution=?8,resolver_id=CASE WHEN ?7='open' THEN NULL ELSE ?9 END,resolved_at=CASE WHEN ?7='open' THEN NULL ELSE ?10 END WHERE guid=?11",
            params![item,ws,reporter,row.severity,row.description,row.photo,row.status,row.resolution,actor,row.updated,row.guid])?;
    }
    Ok(())
}

fn local_change_fields(conn: &Connection, ws: i64, portable: &Value) -> anyhow::Result<Value> {
    let mut local = serde_json::Map::new();
    for (key, value) in portable
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("portable patch is not object"))?
    {
        if value.is_null() || !value.is_object() {
            local.insert(key.clone(), value.clone());
            continue;
        }
        let kind = value
            .get("$ref")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("portable reference has no type"))?;
        let id = match kind {
            "user" => value
                .get("guid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "users", g))
                .ok_or_else(|| anyhow::anyhow!("change user unavailable"))?,
            "status" => {
                let slug = value
                    .get("slug")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("change status has no slug"))?;
                if let Some(id) = status_id(conn, ws, slug) {
                    id
                } else {
                    let name = value.get("name").and_then(Value::as_str).unwrap_or(slug);
                    conn.execute("INSERT INTO statuses(name,workspace_id,type,slug,color,bg) VALUES(?1,?2,'status',?3,'#5E629B','#EDEDF7')",params![name,ws,slug])?;
                    conn.last_insert_rowid()
                }
            }
            table @ ("categories" | "brands" | "building_sites" | "storages") => {
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("change dictionary reference has no name"))?;
                let sql = format!(
                    "SELECT id FROM {table} WHERE workspace_id=?1 AND name=?2 ORDER BY id LIMIT 1"
                );
                if let Some(id) = conn
                    .query_row(&sql, params![ws, name], |r| r.get(0))
                    .optional()?
                {
                    id
                } else {
                    let insert = format!("INSERT INTO {table}(name,workspace_id) VALUES(?1,?2)");
                    conn.execute(&insert, params![name, ws])?;
                    conn.last_insert_rowid()
                }
            }
            _ => anyhow::bail!("unsupported portable reference"),
        };
        local.insert(key.clone(), json!(id));
    }
    Ok(Value::Object(local))
}

fn apply_change_fields(conn: &Connection, item: i64, patch: &Value) -> anyhow::Result<()> {
    conn.execute("UPDATE items SET title=CASE WHEN ?2 THEN ?3 ELSE title END,category_id=CASE WHEN ?4 THEN ?5 ELSE category_id END,brand_id=CASE WHEN ?6 THEN ?7 ELSE brand_id END,status_id=CASE WHEN ?8 THEN ?9 ELSE status_id END,responsible_user_id=CASE WHEN ?10 THEN ?11 ELSE responsible_user_id END,building_site_id=CASE WHEN ?12 THEN ?13 ELSE building_site_id END,storage_id=CASE WHEN ?14 THEN ?15 ELSE storage_id END,serial_number=CASE WHEN ?16 THEN ?17 ELSE serial_number END,cost=CASE WHEN ?18 THEN ?19 ELSE cost END,comment=CASE WHEN ?20 THEN ?21 ELSE comment END,qr_code=CASE WHEN ?22 THEN ?23 ELSE qr_code END,calibrated_until=CASE WHEN ?24 THEN ?25 ELSE calibrated_until END,min_quantity=CASE WHEN ?26 THEN ?27 ELSE min_quantity END WHERE id=?1",params![
        item,patch.get("title").is_some(),patch.get("title").and_then(Value::as_str),patch.get("categoryId").is_some(),patch.get("categoryId").and_then(Value::as_i64),
        patch.get("brandId").is_some(),patch.get("brandId").and_then(Value::as_i64),patch.get("statusId").is_some(),patch.get("statusId").and_then(Value::as_i64),
        patch.get("responsibleUserId").is_some(),patch.get("responsibleUserId").and_then(Value::as_i64),patch.get("buildingSiteId").is_some(),patch.get("buildingSiteId").and_then(Value::as_i64),
        patch.get("storageId").is_some(),patch.get("storageId").and_then(Value::as_i64),patch.get("serialNumber").is_some(),patch.get("serialNumber").and_then(Value::as_str),
        patch.get("cost").is_some(),patch.get("cost").and_then(Value::as_f64),patch.get("comment").is_some(),patch.get("comment").and_then(Value::as_str),
        patch.get("qrCode").is_some(),patch.get("qrCode").and_then(Value::as_str),patch.get("calibratedUntil").is_some(),patch.get("calibratedUntil").and_then(Value::as_str),
        patch.get("minQuantity").is_some(),patch.get("minQuantity").and_then(Value::as_f64)])?;
    Ok(())
}

fn rebuild_change_requests(conn: &Connection) -> anyhow::Result<()> {
    let mut statement=conn.prepare("SELECT r.request_guid,r.item_guid,r.workspace_guid,r.requester_guid,r.actor_guid,r.patch_json,r.before_json,r.comment,r.status,r.reason,r.created_at,r.depth,(SELECT created_at FROM change_request_records root WHERE root.request_guid=r.request_guid AND root.depth=0 ORDER BY root.record_hash LIMIT 1) FROM change_request_records r WHERE r.record_hash=(SELECT record_hash FROM change_request_records w WHERE w.request_guid=r.request_guid ORDER BY depth DESC,record_hash DESC LIMIT 1) ORDER BY r.created_at,r.record_hash")?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, String>(10)?,
                r.get::<_, i64>(11)?,
                r.get::<_, String>(12)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (
        guid,
        item_g,
        ws_g,
        requester_g,
        actor_g,
        patch_json,
        before_json,
        comment,
        status,
        reason,
        updated,
        depth,
        created,
    ) in rows
    {
        let item = id_by_guid(conn, "items", &item_g)
            .ok_or_else(|| anyhow::anyhow!("change item unavailable"))?;
        let ws = id_by_guid(conn, "workspaces", &ws_g)
            .ok_or_else(|| anyhow::anyhow!("change workspace unavailable"))?;
        let requester = id_by_guid(conn, "users", &requester_g)
            .ok_or_else(|| anyhow::anyhow!("change requester unavailable"))?;
        let actor = id_by_guid(conn, "users", &actor_g)
            .ok_or_else(|| anyhow::anyhow!("change actor unavailable"))?;
        let portable_patch: Value = serde_json::from_str(&patch_json)?;
        let portable_before: Value = serde_json::from_str(&before_json)?;
        let local_patch = local_change_fields(conn, ws, &portable_patch)?;
        conn.execute("INSERT OR IGNORE INTO change_requests(item_id,workspace_id,author_id,payload,comment,status,reason,decided_by,created_at,decided_at,guid) VALUES(?1,?2,?3,?4,?5,?6,?7,CASE WHEN ?6='pending' THEN NULL ELSE ?8 END,?9,CASE WHEN ?6='pending' THEN NULL ELSE ?10 END,?11)",params![item,ws,requester,local_patch.to_string(),comment,status,reason,actor,created,updated,guid])?;
        conn.execute("UPDATE change_requests SET item_id=?1,workspace_id=?2,author_id=?3,payload=?4,comment=?5,status=?6,reason=?7,decided_by=CASE WHEN ?6='pending' THEN NULL ELSE ?8 END,decided_at=CASE WHEN ?6='pending' THEN NULL ELSE ?9 END WHERE guid=?10",params![item,ws,requester,local_patch.to_string(),comment,status,reason,actor,updated,guid])?;
        if depth > 0 {
            let newer_direct:i64=conn.query_row("SELECT count(*) FROM history_entries WHERE item_id=?1 AND type IN ('update','item_state_update') AND created_at>=?2",params![item,updated],|r|r.get(0))?;
            if newer_direct == 0 {
                let chosen = if status == "accepted" {
                    local_patch
                } else {
                    local_change_fields(conn, ws, &portable_before)?
                };
                apply_change_fields(conn, item, &chosen)?;
            }
        }
    }
    Ok(())
}

fn rebuild_config_state(conn: &Connection) -> anyhow::Result<()> {
    let mut statement=conn.prepare("SELECT kind,entity_guid,workspace_guid,active,fields_json FROM config_versions v WHERE version_hash=(SELECT version_hash FROM config_versions w WHERE w.kind=v.kind AND w.entity_guid=v.entity_guid ORDER BY depth DESC,version_hash DESC LIMIT 1) ORDER BY kind,entity_guid")?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)? != 0,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (kind, guid, workspace_guid, active, raw) in rows {
        let ws = id_by_guid(conn, "workspaces", &workspace_guid)
            .ok_or_else(|| anyhow::anyhow!("config workspace unavailable"))?;
        let fields: Value = serde_json::from_str(&raw)?;
        let name = fields
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("config name unavailable"))?;
        let table = match kind.as_str() {
            "storage" => "storages",
            "site" => "building_sites",
            "category" => "categories",
            "brand" => "brands",
            "status" => "statuses",
            _ => anyhow::bail!("unsupported config kind"),
        };
        let mut id = id_by_guid(conn, table, &guid);
        if id.is_none() {
            let semantic = if kind == "status" {
                fields.get("slug").and_then(Value::as_str).unwrap_or(name)
            } else {
                name
            };
            let column = if kind == "status" { "slug" } else { "name" };
            let sql = format!(
                "SELECT id FROM {table} WHERE workspace_id=?1 AND {column}=?2 ORDER BY id LIMIT 1"
            );
            id = conn
                .query_row(&sql, params![ws, semantic], |r| r.get(0))
                .optional()?;
            if let Some(existing) = id {
                conn.execute(
                    &format!("UPDATE {table} SET guid=?1 WHERE id=?2"),
                    params![guid, existing],
                )?;
            }
        }
        let id = if let Some(id) = id {
            id
        } else {
            match kind.as_str() {
                "storage" => {
                    conn.execute(
                        "INSERT INTO storages(name,workspace_id,guid) VALUES(?1,?2,?3)",
                        params![name, ws, guid],
                    )?;
                }
                "site" => {
                    conn.execute(
                        "INSERT INTO building_sites(name,workspace_id,guid) VALUES(?1,?2,?3)",
                        params![name, ws, guid],
                    )?;
                }
                "category" => {
                    conn.execute("INSERT INTO categories(name,workspace_id,type,guid) VALUES(?1,?2,'category',?3)",params![name,ws,guid])?;
                }
                "brand" => {
                    conn.execute(
                        "INSERT INTO brands(name,workspace_id,type,guid) VALUES(?1,?2,'brand',?3)",
                        params![name, ws, guid],
                    )?;
                }
                "status" => {
                    conn.execute("INSERT INTO statuses(name,workspace_id,type,slug,color,bg,guid) VALUES(?1,?2,'status',?3,?4,?5,?6)",params![name,ws,fields.get("slug").and_then(Value::as_str).unwrap_or("custom"),fields.get("color").and_then(Value::as_str),fields.get("bg").and_then(Value::as_str),guid])?;
                }
                _ => unreachable!(),
            };
            conn.last_insert_rowid()
        };
        let archived = (!active) as i64;
        match kind.as_str() {
            "storage" => {
                let responsible = fields
                    .get("responsibleGuid")
                    .and_then(Value::as_str)
                    .and_then(|g| id_by_guid(conn, "users", g));
                conn.execute("UPDATE storages SET name=?1,address=?2,responsible_user_id=?3,archived=?4 WHERE id=?5",params![name,fields.get("address").and_then(Value::as_str),responsible,archived,id])?;
            }
            "site" => {
                let responsible = fields
                    .get("responsibleGuid")
                    .and_then(Value::as_str)
                    .and_then(|g| id_by_guid(conn, "users", g));
                conn.execute("UPDATE building_sites SET name=?1,responsible_user_id=?2,archived=?3 WHERE id=?4",params![name,responsible,archived,id])?;
            }
            "category" | "brand" => {
                conn.execute(
                    &format!("UPDATE {table} SET name=?1,description=?2,archived=?3 WHERE id=?4"),
                    params![
                        name,
                        fields.get("description").and_then(Value::as_str),
                        archived,
                        id
                    ],
                )?;
            }
            "status" => {
                conn.execute("UPDATE statuses SET name=?1,description=?2,slug=?3,color=?4,bg=?5,archived=?6 WHERE id=?7",params![name,fields.get("description").and_then(Value::as_str),fields.get("slug").and_then(Value::as_str),fields.get("color").and_then(Value::as_str),fields.get("bg").and_then(Value::as_str),archived,id])?;
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

fn rebuild_item_state(conn: &Connection) -> anyhow::Result<()> {
    let mut statement=conn.prepare("SELECT item_guid,fields_json,updated_at FROM item_state_versions v WHERE version_hash=(SELECT version_hash FROM item_state_versions w WHERE w.item_guid=v.item_guid ORDER BY depth DESC,version_hash DESC LIMIT 1) ORDER BY item_guid")?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (guid, raw, updated_at) in rows {
        let Some(id) = id_by_guid(conn, "items", &guid) else {
            continue;
        };
        let fields: Value = serde_json::from_str(&raw)?;
        let reference = |key: &str, table: &str| {
            fields
                .get(key)
                .and_then(Value::as_str)
                .and_then(|guid| id_by_guid(conn, table, guid))
        };
        conn.execute("UPDATE items SET internal_id=?1,title=?2,category_id=?3,brand_id=?4,serial_number=?5,qr_code=?6,calibrated_until=?7,min_quantity=?8,quantitative=?9,unit=?10,cost=?11,comment=?12,source_system=?13,external_id=?14,metadata_json=?15,organization_node_id=?16 WHERE id=?17",params![fields.get("internalId").and_then(Value::as_str),fields.get("title").and_then(Value::as_str),reference("categoryGuid","categories"),reference("brandGuid","brands"),fields.get("serialNumber").and_then(Value::as_str),fields.get("qrCode").and_then(Value::as_str),fields.get("calibratedUntil").and_then(Value::as_str),fields.get("minQuantity").and_then(Value::as_f64),fields.get("quantitative").and_then(Value::as_bool).unwrap_or(false) as i64,fields.get("unit").and_then(Value::as_str),fields.get("cost").and_then(Value::as_f64),fields.get("comment").and_then(Value::as_str),fields.get("sourceSystem").and_then(Value::as_str),fields.get("externalId").and_then(Value::as_str),fields.get("metadata").map(Value::to_string),reference("organizationNodeGuid","organization_nodes"),id])?;
        let later:i64=conn.query_row("SELECT count(*) FROM history_entries WHERE item_id=?1 AND created_at>=?2 AND type IN ('take','return','move','inventory','replenish','write_off','transfer_send','transfer_receive','fault_report','fault_update')",params![id,updated_at],|r|r.get(0))?;
        if later == 0 {
            let workspace: i64 =
                conn.query_row("SELECT workspace_id FROM items WHERE id=?1", [id], |r| {
                    r.get(0)
                })?;
            let status = fields
                .get("statusSlug")
                .and_then(Value::as_str)
                .and_then(|slug| status_id(conn, workspace, slug));
            conn.execute("UPDATE items SET status_id=?1,responsible_user_id=?2,building_site_id=?3,storage_id=?4,quantity=?5 WHERE id=?6",params![status,reference("responsibleGuid","users"),reference("buildingSiteGuid","building_sites"),reference("storageGuid","storages"),fields.get("quantity").and_then(Value::as_f64),id])?;
        }
    }
    Ok(())
}

fn rebuild_organization_nodes(conn: &Connection) -> anyhow::Result<()> {
    let mut statement=conn.prepare("SELECT node_guid,workspace_guid,active,fields_json FROM organization_node_versions v WHERE version_hash=(SELECT version_hash FROM organization_node_versions w WHERE w.node_guid=v.node_guid ORDER BY depth DESC,version_hash DESC LIMIT 1) ORDER BY node_guid")?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? != 0,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let parsed: Vec<(String, String, bool, Value)> = rows
        .into_iter()
        .map(|(guid, ws, active, raw)| Ok((guid, ws, active, serde_json::from_str(&raw)?)))
        .collect::<anyhow::Result<_>>()?;
    let parents: HashMap<&str, Option<&str>> = parsed
        .iter()
        .map(|(guid, _, _, fields)| {
            (
                guid.as_str(),
                fields.get("parentGuid").and_then(Value::as_str),
            )
        })
        .collect();
    for start in parents.keys() {
        let mut seen = HashSet::new();
        let mut cursor = Some(*start);
        while let Some(node) = cursor {
            if !seen.insert(node) {
                anyhow::bail!("organization structure cycle detected");
            }
            cursor = parents.get(node).copied().flatten();
        }
    }
    for (guid, workspace, active, fields) in &parsed {
        let ws = id_by_guid(conn, "workspaces", workspace)
            .ok_or_else(|| anyhow::anyhow!("organization workspace unavailable"))?;
        let responsible = fields
            .get("responsibleGuid")
            .and_then(Value::as_str)
            .and_then(|guid| id_by_guid(conn, "users", guid));
        if id_by_guid(conn, "organization_nodes", guid).is_none() {
            conn.execute("INSERT INTO organization_nodes(guid,workspace_id,kind,name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)",params![guid,ws,fields.get("kind").and_then(Value::as_str).unwrap_or("section"),fields.get("name").and_then(Value::as_str).unwrap_or("Раздел"),chrono::Utc::now().to_rfc3339()])?;
        }
        conn.execute("UPDATE organization_nodes SET workspace_id=?1,kind=?2,name=?3,tab_label=?4,responsible_user_id=?5,display_order=?6,color=?7,icon=?8,archived=?9,updated_at=?10 WHERE guid=?11",params![ws,fields.get("kind").and_then(Value::as_str),fields.get("name").and_then(Value::as_str),fields.get("tabLabel").and_then(Value::as_str),responsible,fields.get("displayOrder").and_then(Value::as_i64).unwrap_or(0),fields.get("color").and_then(Value::as_str),fields.get("icon").and_then(Value::as_str),(!*active) as i64,chrono::Utc::now().to_rfc3339(),guid])?;
    }
    for (guid, _, _, fields) in &parsed {
        let parent = fields
            .get("parentGuid")
            .and_then(Value::as_str)
            .and_then(|guid| id_by_guid(conn, "organization_nodes", guid));
        conn.execute(
            "UPDATE organization_nodes SET parent_id=?1 WHERE guid=?2",
            params![parent, guid],
        )?;
    }
    Ok(())
}

pub fn import_journal(conn: &Connection, journal: &Value) -> Value {
    let mut workspaces = 0u32;
    let mut users = 0u32;
    let mut items_n = 0u32;
    let mut ops = 0u32;
    let mut skipped = 0u32;
    let mut conflicts = 0u32;
    let mut messages = 0u32;

    if let Some(arr) = journal.get("workspaces").and_then(|v| v.as_array()) {
        for w in arr {
            upsert_workspace(conn, w);
            workspaces += 1;
        }
    }
    if let Some(arr) = journal.get("users").and_then(|v| v.as_array()) {
        for u in arr {
            upsert_user(conn, u);
            users += 1;
        }
    }
    if let Ok(devices) = incoming_devices(journal) {
        for (device_id, device) in devices {
            let Some(user_id) = id_by_guid(conn, "users", &device.user_guid) else {
                skipped += 1;
                continue;
            };
            let existing: Option<(i64, String, Option<String>)> = conn
                .query_row(
                    "SELECT user_id,public_key,revoked_at FROM user_devices WHERE device_id=?1",
                    [&device_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .ok()
                .flatten();
            match existing {
                Some((owner, key, local_revoked))
                    if owner == user_id && key == device.public_key =>
                {
                    let merged_revoked =
                        match (local_revoked.as_deref(), device.revoked_at.as_deref()) {
                            (Some(local), Some(remote)) => Some(local.min(remote)),
                            (Some(local), None) => Some(local),
                            (None, Some(remote)) => Some(remote),
                            (None, None) => None,
                        };
                    let _ = conn.execute(
                        "UPDATE user_devices SET revoked_at=?1 WHERE device_id=?2",
                        params![merged_revoked, device_id],
                    );
                }
                None => {
                    let _ = conn.execute(
                        "INSERT INTO user_devices(device_id,user_id,name,public_key,created_at,revoked_at)
                         VALUES(?1,?2,'Синхронизированное устройство',?3,?4,?5)",
                        params![
                            device_id,
                            user_id,
                            device.public_key,
                            device.created_at,
                            device.revoked_at
                        ],
                    );
                }
                _ => skipped += 1,
            }
        }
    }
    if let Some(records) = journal.get("configVersions").and_then(Value::as_array) {
        for record in records {
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO config_versions(version_hash,entity_guid,kind,parent_hash,depth,workspace_guid,actor_guid,active,fields_json,payload_hash,ledger_hash,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![record.get("versionHash").and_then(Value::as_str),record.get("entityGuid").and_then(Value::as_str),record.get("kind").and_then(Value::as_str),record.get("parentHash").and_then(Value::as_str),record.get("depth").and_then(Value::as_i64),record.get("workspaceGuid").and_then(Value::as_str),record.get("actorGuid").and_then(Value::as_str),record.get("active").and_then(Value::as_bool).map(i64::from),record.get("fields").map(Value::to_string),record.get("payloadHash").and_then(Value::as_str),record.get("ledgerHash").and_then(Value::as_str),record.get("updatedAt").and_then(Value::as_str)],
            ).unwrap_or(0);
            if inserted > 0 {
                ops += 1;
            } else {
                skipped += 1;
            }
        }
        if let Err(error) = rebuild_config_state(conn) {
            return json!({"ok":false,"error":format!("Не удалось восстановить справочники: {error}")});
        }
    }
    if let Some(records) = journal.get("itemStateVersions").and_then(Value::as_array) {
        for record in records {
            let inserted=conn.execute("INSERT OR IGNORE INTO item_state_versions(version_hash,item_guid,parent_hash,depth,workspace_guid,actor_guid,fields_json,payload_hash,ledger_hash,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![record.get("versionHash").and_then(Value::as_str),record.get("itemGuid").and_then(Value::as_str),record.get("parentHash").and_then(Value::as_str),record.get("depth").and_then(Value::as_i64),record.get("workspaceGuid").and_then(Value::as_str),record.get("actorGuid").and_then(Value::as_str),record.get("fields").map(Value::to_string),record.get("payloadHash").and_then(Value::as_str),record.get("ledgerHash").and_then(Value::as_str),record.get("updatedAt").and_then(Value::as_str)]).unwrap_or(0);
            if inserted > 0 {
                ops += 1
            } else {
                skipped += 1
            }
        }
    }
    if let Some(records) = journal
        .get("organizationNodeVersions")
        .and_then(Value::as_array)
    {
        for record in records {
            let inserted=conn.execute("INSERT OR IGNORE INTO organization_node_versions(version_hash,node_guid,parent_hash,depth,workspace_guid,actor_guid,active,fields_json,payload_hash,ledger_hash,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![record.get("versionHash").and_then(Value::as_str),record.get("nodeGuid").and_then(Value::as_str),record.get("parentHash").and_then(Value::as_str),record.get("depth").and_then(Value::as_i64),record.get("workspaceGuid").and_then(Value::as_str),record.get("actorGuid").and_then(Value::as_str),record.get("active").and_then(Value::as_bool).map(i64::from),record.get("fields").map(Value::to_string),record.get("payloadHash").and_then(Value::as_str),record.get("ledgerHash").and_then(Value::as_str),record.get("updatedAt").and_then(Value::as_str)]).unwrap_or(0);
            if inserted > 0 {
                ops += 1
            } else {
                skipped += 1
            }
        }
    }
    if let Some(arr) = journal.get("organizationNodes").and_then(|v| v.as_array()) {
        // Первый проход создаёт узлы без родителей, чтобы порядок входящего
        // массива не имел значения. Второй восстанавливает связи по GUID.
        for node in arr {
            let guid = node.get("guid").and_then(Value::as_str).unwrap_or("");
            let ws_guid = node
                .get("workspaceGuid")
                .and_then(Value::as_str)
                .unwrap_or("");
            let Some(ws) = id_by_guid(conn, "workspaces", ws_guid) else {
                continue;
            };
            let responsible = node
                .get("responsibleGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "users", g));
            let _ = conn.execute(
                "INSERT INTO organization_nodes(guid,workspace_id,parent_id,kind,name,tab_label,responsible_user_id,display_order,color,icon,archived,created_at,updated_at)
                 VALUES(?1,?2,NULL,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
                 ON CONFLICT(guid) DO UPDATE SET kind=excluded.kind,name=excluded.name,tab_label=excluded.tab_label,responsible_user_id=excluded.responsible_user_id,display_order=excluded.display_order,color=excluded.color,icon=excluded.icon,archived=excluded.archived,updated_at=excluded.updated_at",
                params![guid,ws,node.get("kind").and_then(Value::as_str).unwrap_or("section"),node.get("name").and_then(Value::as_str).unwrap_or("Раздел"),node.get("tabLabel").and_then(Value::as_str),responsible,node.get("displayOrder").and_then(Value::as_i64).unwrap_or(0),node.get("color").and_then(Value::as_str),node.get("icon").and_then(Value::as_str),node.get("archived").and_then(Value::as_bool).unwrap_or(false),node.get("createdAt").and_then(Value::as_str).unwrap_or(""),node.get("updatedAt").and_then(Value::as_str).unwrap_or("")],
            );
        }
        for node in arr {
            let guid = node.get("guid").and_then(Value::as_str).unwrap_or("");
            let parent = node
                .get("parentGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "organization_nodes", g));
            let _ = conn.execute(
                "UPDATE organization_nodes SET parent_id=?1 WHERE guid=?2",
                params![parent, guid],
            );
        }
    }
    if let Err(error) = rebuild_organization_nodes(conn) {
        return json!({"ok":false,"error":format!("Не удалось восстановить структуру организации: {error}")});
    }
    if let Some(arr) = journal.get("items").and_then(|v| v.as_array()) {
        for it in arr {
            let guid = it.get("guid").and_then(|v| v.as_str()).unwrap_or("");
            let ws_g = it
                .get("workspaceGuid")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Some(ws) = id_by_guid(conn, "workspaces", ws_g) else {
                continue;
            };
            let resp_g = it.get("responsibleGuid").and_then(|v| v.as_str());
            let resp = resp_g.and_then(|g| id_by_guid(conn, "users", g));
            let organization_node = it
                .get("organizationNodeGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "organization_nodes", g));
            let category = it
                .get("categoryGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "categories", g));
            let brand = it
                .get("brandGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "brands", g));
            let building_site = it
                .get("buildingSiteGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "building_sites", g));
            let mut storage = it
                .get("storageGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "storages", g));
            if storage.is_none() {
                if let (Some(guid), Some(name)) = (
                    it.get("storageGuid").and_then(Value::as_str),
                    it.get("storageName").and_then(Value::as_str),
                ) {
                    let existing:Option<i64>=conn.query_row("SELECT id FROM storages WHERE workspace_id=?1 AND name=?2 ORDER BY id LIMIT 1",params![ws,name],|r|r.get(0)).optional().ok().flatten();
                    if let Some(id) = existing {
                        if conn
                            .execute("UPDATE storages SET guid=?1 WHERE id=?2", params![guid, id])
                            .is_ok()
                        {
                            storage = Some(id);
                        }
                    }
                }
            }
            let slug = it
                .get("statusSlug")
                .and_then(|v| v.as_str())
                .unwrap_or("in-stock");
            let st = status_id(conn, ws, slug);
            if !guid.is_empty() {
                if let Some(local_id) = id_by_guid(conn, "items", guid) {
                    let local_clock: Option<String> = conn
                        .query_row(
                            "SELECT MAX(created_at) FROM history_entries WHERE item_id=?1",
                            params![local_id],
                            |row| row.get(0),
                        )
                        .ok()
                        .flatten();
                    let incoming_clock = journal
                        .get("history")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|event| event.get("itemGuid").and_then(Value::as_str) == Some(guid))
                        .filter_map(|event| event.get("createdAt").and_then(Value::as_str))
                        .max();
                    let incoming_is_newer = match (incoming_clock, local_clock.as_deref()) {
                        (Some(incoming), Some(local)) => incoming > local,
                        (Some(_), None) => true,
                        _ => false,
                    };
                    let local_resp: Option<i64> = conn
                        .query_row(
                            "SELECT responsible_user_id FROM items WHERE id=?1",
                            params![local_id],
                            |r| r.get(0),
                        )
                        .ok()
                        .flatten();
                    if local_resp.is_some() && resp.is_some() && local_resp != resp {
                        let desc = format!(
                            "Двое взяли один предмет офлайн: локально {:?} / входящий {:?}",
                            local_resp, resp
                        );
                        let _ = conn.execute(
                            "INSERT INTO conflicts (workspace_id, item_id, item_guid, description, left_label, right_label, created_at)
                             VALUES (?1,?2,?3,?4,?5,?6,?7)",
                            params![
                                ws,
                                local_id,
                                guid,
                                desc,
                                format!("user:{:?}", local_resp),
                                format!("user:{:?}", resp),
                                chrono::Utc::now().to_rfc3339()
                            ],
                        );
                        if let Some(st_id) = status_id(conn, ws, "needs-check") {
                            let _ = conn.execute(
                                "UPDATE items SET responsible_user_id=NULL, status_id=?1 WHERE id=?2",
                                params![st_id, local_id],
                            );
                        }
                        // Конфликт — самостоятельный факт истории, а не только
                        // локальная строка в UI. За счёт нового подписанного
                        // события карантинное состояние становится новее обеих
                        // конкурирующих выдач и распространяется всем peers.
                        if let Ok(conflict_actor) = conn.query_row(
                            "SELECT user_id FROM user_workspaces WHERE workspace_id=?1 ORDER BY id LIMIT 1",
                            params![ws],
                            |row| row.get::<_, i64>(0),
                        ) {
                            let _ = ledger::append(
                                conn,
                                ws,
                                conflict_actor,
                                Some(local_id),
                                "conflict_detected",
                                Some(&format!("user:{:?}", local_resp)),
                                Some(&format!("user:{:?}", resp)),
                                None,
                                Some(&desc),
                            );
                        }
                        notify_conflict(conn, ws, local_id, &desc);
                        conflicts += 1;
                    } else if incoming_is_newer {
                        let incoming_title = it.get("title").and_then(|v| v.as_str()).unwrap_or("");
                        let title_ok = !incoming_title.is_empty()
                            && !incoming_title.contains('Ã')
                            && !incoming_title.contains('\u{FFFD}');
                        let _ = conn.execute(
                            "UPDATE items SET title=CASE WHEN ?5 THEN COALESCE(?2,title) ELSE title END,
                             due_at=?3, responsible_user_id=?4, status_id=COALESCE(?10,status_id),
                             source_system=COALESCE(?6,source_system), external_id=COALESCE(?7,external_id),
                             metadata_json=COALESCE(?8,metadata_json),
                             organization_node_id=?9,
                             calibrated_until=?11, min_quantity=?12, quantity=?13,
                             unit=?14, cost=?15, comment=?16, category_id=?17, brand_id=?18,
                             building_site_id=?19, storage_id=?20 WHERE id=?1",
                            params![local_id, incoming_title, it.get("dueAt").and_then(|v| v.as_str()), resp, title_ok as i64,
                                it.get("sourceSystem").and_then(|v| v.as_str()),
                                it.get("externalId").and_then(|v| v.as_str()),
                                it.get("metadata").filter(|v| v.is_object()).map(Value::to_string),
                                organization_node, st,
                                it.get("calibratedUntil").and_then(Value::as_str),
                                it.get("minQuantity").and_then(Value::as_f64),
                                it.get("quantity").and_then(Value::as_f64),
                                it.get("unit").and_then(Value::as_str),
                                it.get("cost").and_then(Value::as_f64),
                                it.get("comment").and_then(Value::as_str), category, brand,
                                building_site, storage],
                        );
                    }
                    items_n += 1;
                    continue;
                }
            }
            let title = it
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Инструмент");
            let internal = it
                .get("internalId")
                .and_then(|v| v.as_str())
                .unwrap_or("ВН-0000");
            let _ = conn.execute(
                "INSERT INTO items (internal_id, title, status_id, responsible_user_id, workspace_id, serial_number, qr_code, due_at, guid, calibrated_until, min_quantity, quantitative, quantity, unit, cost, comment, source_system, external_id, metadata_json, created_at, organization_node_id,category_id,brand_id,building_site_id,storage_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
                params![
                    internal, title, st, resp, ws,
                    it.get("serialNumber").and_then(|v| v.as_str()),
                    it.get("qrCode").and_then(|v| v.as_str()),
                    it.get("dueAt").and_then(|v| v.as_str()),
                    if guid.is_empty() { uuid::Uuid::new_v4().to_string().replace('-', "") } else { guid.to_string() },
                    it.get("calibratedUntil").and_then(|v| v.as_str()),
                    it.get("minQuantity").and_then(|v| v.as_f64()),
                    it.get("quantitative").and_then(|v| v.as_bool()).unwrap_or(false) as i64,
                    it.get("quantity").and_then(|v| v.as_f64()),
                    it.get("unit").and_then(|v| v.as_str()),
                    it.get("cost").and_then(|v| v.as_f64()),
                    it.get("comment").and_then(|v| v.as_str()),
                    it.get("sourceSystem").and_then(|v| v.as_str()),
                    it.get("externalId").and_then(|v| v.as_str()),
                    it.get("metadata").filter(|v| v.is_object()).map(Value::to_string),
                    chrono::Utc::now().to_rfc3339(),
                    organization_node, category, brand, building_site, storage
                ],
            );
            items_n += 1;
        }
    }
    if let Some(records) = journal.get("itemTombstones").and_then(Value::as_array) {
        for record in records {
            let Some(item_guid) = record.get("itemGuid").and_then(Value::as_str) else {
                skipped += 1;
                continue;
            };
            let Some(workspace_guid) = record.get("workspaceGuid").and_then(Value::as_str) else {
                skipped += 1;
                continue;
            };
            let Some(actor_guid) = record.get("actorGuid").and_then(Value::as_str) else {
                skipped += 1;
                continue;
            };
            let Some(ledger_hash) = record.get("ledgerHash").and_then(Value::as_str) else {
                skipped += 1;
                continue;
            };
            let Some(deleted_at) = record.get("deletedAt").and_then(Value::as_str) else {
                skipped += 1;
                continue;
            };
            let expected = item_tombstone_hash(
                workspace_guid,
                item_guid,
                actor_guid,
                ledger_hash,
                deleted_at,
            );
            if record.get("tombstoneHash").and_then(Value::as_str) != Some(expected.as_str()) {
                skipped += 1;
                continue;
            }
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO item_tombstones(item_guid,workspace_guid,actor_guid,ledger_hash,deleted_at,tombstone_hash)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![item_guid,workspace_guid,actor_guid,ledger_hash,deleted_at,expected],
            ).unwrap_or(0);
            let _ = conn.execute(
                "UPDATE items SET archived=1,archived_at=COALESCE(archived_at,?1) WHERE guid=?2",
                params![deleted_at, item_guid],
            );
            if inserted == 0 {
                skipped += 1;
            }
        }
    }
    if let Some(arr) = journal.get("photos").and_then(Value::as_array) {
        for photo in arr {
            let guid = photo.get("guid").and_then(Value::as_str).unwrap_or("");
            let Some(item_id) = photo
                .get("itemGuid")
                .and_then(Value::as_str)
                .and_then(|guid| id_by_guid(conn, "items", guid))
            else {
                skipped += 1;
                continue;
            };
            let url = photo.get("url").and_then(Value::as_str).unwrap_or("");
            if guid.is_empty() || url.is_empty() {
                skipped += 1;
                continue;
            }
            let inserted = conn
                .execute(
                    "INSERT OR IGNORE INTO item_photos(guid,item_id,url,thumb_url,sha256,is_title)
                     VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        guid,
                        item_id,
                        url,
                        photo.get("thumbUrl").and_then(Value::as_str),
                        photo.get("sha256").and_then(Value::as_str),
                        photo
                            .get("isTitle")
                            .and_then(Value::as_bool)
                            .unwrap_or(false) as i64
                    ],
                )
                .unwrap_or(0);
            if inserted == 0 {
                skipped += 1;
            }
        }
    }
    if let Some(arr) = journal.get("documents").and_then(Value::as_array) {
        for document in arr {
            let guid = document.get("guid").and_then(Value::as_str).unwrap_or("");
            let Some(item_id) = document
                .get("itemGuid")
                .and_then(Value::as_str)
                .and_then(|guid| id_by_guid(conn, "items", guid))
            else {
                skipped += 1;
                continue;
            };
            let name = document.get("name").and_then(Value::as_str).unwrap_or("");
            let url = document.get("url").and_then(Value::as_str).unwrap_or("");
            let access = document
                .get("accessLevel")
                .and_then(Value::as_str)
                .unwrap_or("members");
            if guid.is_empty()
                || name.is_empty()
                || url.is_empty()
                || !matches!(access, "members" | "accounting" | "managers")
            {
                skipped += 1;
                continue;
            }
            let author_id = document
                .get("authorGuid")
                .and_then(Value::as_str)
                .and_then(|guid| id_by_guid(conn, "users", guid));
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO item_documents(guid,item_id,name,url,mime,sha256,author_id,access_level)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![guid,item_id,name,url,document.get("mime").and_then(Value::as_str),
                    document.get("sha256").and_then(Value::as_str),author_id,access],
            ).unwrap_or(0);
            if inserted == 0 {
                skipped += 1;
            }
        }
    }
    if let Some(arr) = journal.get("history").and_then(|v| v.as_array()) {
        for h in arr {
            // opId — текущее имя поля, hash — совместимость со старыми архивами.
            let hash = h
                .get("opId")
                .or_else(|| h.get("hash"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if hash.is_empty() {
                skipped += 1;
                continue;
            }
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM history_entries WHERE hash=?1",
                    params![hash],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if exists > 0 {
                skipped += 1;
                continue;
            }
            let ws_g = h
                .get("workspaceGuid")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Some(ws) = id_by_guid(conn, "workspaces", ws_g) else {
                skipped += 1;
                continue;
            };
            let item = h
                .get("itemGuid")
                .and_then(|v| v.as_str())
                .and_then(|g| id_by_guid(conn, "items", g));
            let actor = h
                .get("actorGuid")
                .and_then(|v| v.as_str())
                .and_then(|g| id_by_guid(conn, "users", g))
                .unwrap_or(1);
            let _ = conn.execute(
                "INSERT OR IGNORE INTO history_entries (workspace_id,item_id,type,actor_user_id,from_label,to_label,quantity_delta,comment,hash,created_at,guid,prev_hash,signature,pubkey,event_version,request_device_id,request_public_key,request_nonce,request_signature,request_hash,request_timestamp,request_path,request_body)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
                params![
                    ws, item,
                    h.get("type").and_then(|v| v.as_str()).unwrap_or("update"),
                    actor,
                    h.get("fromLabel").and_then(|v| v.as_str()),
                    h.get("toLabel").and_then(|v| v.as_str()),
                    h.get("quantityDelta").and_then(|v| v.as_f64()),
                    h.get("comment").and_then(|v| v.as_str()),
                    hash,
                    h.get("createdAt").and_then(|v| v.as_str()).unwrap_or(""),
                    h.get("guid").and_then(|v| v.as_str()),
                    h.get("prevHash").and_then(|v| v.as_str()),
                    h.get("signature").and_then(|v| v.as_str()),
                    h.get("pubkey").and_then(|v| v.as_str()),
                    h.get("eventVersion").and_then(Value::as_i64).unwrap_or(1),
                    h.get("requestDeviceId").and_then(Value::as_str),
                    h.get("requestPublicKey").and_then(Value::as_str),
                    h.get("requestNonce").and_then(Value::as_str),
                    h.get("requestSignature").and_then(Value::as_str),
                    h.get("requestHash").and_then(Value::as_str),
                    h.get("requestTimestamp").and_then(Value::as_str),
                    h.get("requestPath").and_then(Value::as_str),
                    h.get("requestBody").and_then(Value::as_str)
                ],
            );
            ops += 1;
        }
    }
    if let Err(error) = rebuild_item_state(conn) {
        return json!({"ok":false,"error":format!("Не удалось восстановить master-состояние ТМЦ: {error}")});
    }
    if let Some(offers) = journal.get("saleOffers").and_then(Value::as_array) {
        for offer in offers {
            let Some(workspace) = offer
                .get("workspaceGuid")
                .and_then(Value::as_str)
                .and_then(|guid| id_by_guid(conn, "workspaces", guid))
            else {
                skipped += 1;
                continue;
            };
            let Some(item) = offer
                .get("itemGuid")
                .and_then(Value::as_str)
                .and_then(|guid| id_by_guid(conn, "items", guid))
            else {
                skipped += 1;
                continue;
            };
            let Some(seller) = offer
                .get("sellerGuid")
                .and_then(Value::as_str)
                .and_then(|guid| id_by_guid(conn, "users", guid))
            else {
                skipped += 1;
                continue;
            };
            let Some(buyer) = offer
                .get("buyerGuid")
                .and_then(Value::as_str)
                .and_then(|guid| id_by_guid(conn, "users", guid))
            else {
                skipped += 1;
                continue;
            };
            let guid = offer.get("offerGuid").and_then(Value::as_str).unwrap_or("");
            let amount = offer.get("bitAmount").and_then(Value::as_i64).unwrap_or(0);
            let quantity = offer.get("quantity").and_then(Value::as_f64);
            let immutable: Option<(i64,i64,i64,i64,i64,Option<f64>)> = conn.query_row(
                "SELECT workspace_id,item_id,from_user_id,to_user_id,bit_amount,quantity FROM transfers WHERE guid=?1",
                [guid], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
            ).optional().ok().flatten();
            if immutable.is_some_and(|existing| {
                existing != (workspace, item, seller, buyer, amount, quantity)
            }) {
                return json!({"ok":false,"error":"GUID предложения Bit уже связан с другими неизменяемыми полями"});
            }
            let code = format!("SALE-{}", guid.chars().take(8).collect::<String>());
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO transfers(code,item_id,from_user_id,to_user_id,workspace_id,status,comment,no_confirmation,source_custody,bit_amount,guid,prepare_ledger_hash,created_at,quantity)
                 VALUES(?1,?2,?3,?4,?5,'pending',?6,0,1,?7,?8,?9,?10,?11)",
                params![code,item,seller,buyer,workspace,offer.get("comment").and_then(Value::as_str),
                    amount,guid,
                    offer.get("ledgerHash").and_then(Value::as_str),offer.get("createdAt").and_then(Value::as_str),quantity],
            ).unwrap_or(0);
            if inserted > 0 {
                ops += 1
            } else {
                skipped += 1
            }
            let status = offer
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            if status == "accepted" {
                conn.execute(
                    "UPDATE transfers SET status='accepted',completed_at=COALESCE(completed_at,?1),accept_ledger_hash=?2,bit_transaction_guid=?3 WHERE guid=?4 AND status!='accepted'",
                    params![chrono::Utc::now().to_rfc3339(),offer.get("decisionLedgerHash").and_then(Value::as_str),offer.get("bitTransactionGuid").and_then(Value::as_str),guid],
                ).unwrap_or(0);
                let quantitative = conn
                    .query_row(
                        "SELECT quantitative!=0 FROM items WHERE id=?1",
                        [item],
                        |row| row.get::<_, bool>(0),
                    )
                    .unwrap_or(false);
                if !quantitative {
                    conn.execute(
                        "UPDATE items SET responsible_user_id=?1 WHERE id=?2",
                        params![buyer, item],
                    )
                    .unwrap_or(0);
                }
            } else if status == "rejected" {
                conn.execute(
                    "UPDATE transfers SET status='rejected',completed_at=COALESCE(completed_at,?1),accept_ledger_hash=?2 WHERE guid=?3 AND status='pending'",
                    params![chrono::Utc::now().to_rfc3339(),offer.get("decisionLedgerHash").and_then(Value::as_str),guid],
                ).unwrap_or(0);
            }
        }
    }
    if let Some(entries) = journal.get("custody").and_then(Value::as_array) {
        for entry in entries {
            let inserted = conn
                .execute(
                    "INSERT OR IGNORE INTO custody_entries(entry_hash,workspace_guid,item_guid,user_guid,quantity_delta,due_at,comment,photo_url,ledger_hash,created_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        entry.get("entryHash").and_then(Value::as_str),
                        entry.get("workspaceGuid").and_then(Value::as_str),
                        entry.get("itemGuid").and_then(Value::as_str),
                        entry.get("userGuid").and_then(Value::as_str),
                        entry.get("quantityDelta").and_then(Value::as_f64),
                        entry.get("dueAt").and_then(Value::as_str),
                        entry.get("comment").and_then(Value::as_str),
                        entry.get("photoUrl").and_then(Value::as_str),
                        entry.get("ledgerHash").and_then(Value::as_str),
                        entry.get("createdAt").and_then(Value::as_str),
                    ],
                )
                .unwrap_or(0);
            if inserted == 0 {
                skipped += 1;
            }
        }
        if let Err(error) = rebuild_quantitative_holdings(conn) {
            return json!({"ok":false,"error":format!("Не удалось восстановить custody-состояние: {error}")});
        }
    }
    if let Some(records) = journal.get("itemComments").and_then(Value::as_array) {
        for record in records {
            let Some(item_id) = record
                .get("itemGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "items", g))
            else {
                skipped += 1;
                continue;
            };
            let Some(author_id) = record
                .get("authorGuid")
                .and_then(Value::as_str)
                .and_then(|g| id_by_guid(conn, "users", g))
            else {
                skipped += 1;
                continue;
            };
            let Some(text) = record.get("text").and_then(Value::as_str) else {
                skipped += 1;
                continue;
            };
            let Some(created_at) = record.get("createdAt").and_then(Value::as_str) else {
                skipped += 1;
                continue;
            };
            let inserted=conn.execute(
                "INSERT OR IGNORE INTO item_comment_records(record_hash,guid,workspace_guid,item_guid,author_guid,text,payload_hash,ledger_hash,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![
                    record.get("recordHash").and_then(Value::as_str),record.get("guid").and_then(Value::as_str),
                    record.get("workspaceGuid").and_then(Value::as_str),record.get("itemGuid").and_then(Value::as_str),
                    record.get("authorGuid").and_then(Value::as_str),text,record.get("payloadHash").and_then(Value::as_str),
                    record.get("ledgerHash").and_then(Value::as_str),created_at]).unwrap_or(0);
            if inserted > 0 {
                let _=conn.execute("INSERT INTO item_comments(item_id,user_id,text,created_at) VALUES(?1,?2,?3,?4)",params![item_id,author_id,text,created_at]);
                ops += 1;
            } else {
                skipped += 1;
            }
        }
    }
    if let Some(records) = journal.get("faults").and_then(Value::as_array) {
        for record in records {
            let inserted=conn.execute("INSERT OR IGNORE INTO fault_records(record_hash,fault_guid,parent_hash,depth,workspace_guid,item_guid,reporter_guid,actor_guid,severity,description,photo_url,status,resolution,payload_hash,ledger_hash,created_at)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![
                    record.get("recordHash").and_then(Value::as_str),record.get("faultGuid").and_then(Value::as_str),record.get("parentHash").and_then(Value::as_str),record.get("depth").and_then(Value::as_i64),
                    record.get("workspaceGuid").and_then(Value::as_str),record.get("itemGuid").and_then(Value::as_str),record.get("reporterGuid").and_then(Value::as_str),record.get("actorGuid").and_then(Value::as_str),
                    record.get("severity").and_then(Value::as_str),record.get("description").and_then(Value::as_str),record.get("photoUrl").and_then(Value::as_str),record.get("status").and_then(Value::as_str),
                    record.get("resolution").and_then(Value::as_str),record.get("payloadHash").and_then(Value::as_str),record.get("ledgerHash").and_then(Value::as_str),record.get("createdAt").and_then(Value::as_str)]).unwrap_or(0);
            if inserted > 0 {
                ops += 1;
            } else {
                skipped += 1;
            }
        }
        if let Err(error) = rebuild_fault_state(conn) {
            return json!({"ok":false,"error":format!("Не удалось восстановить неисправности: {error}")});
        }
    }
    if let Some(records) = journal.get("changeRequests").and_then(Value::as_array) {
        for record in records {
            let inserted=conn.execute("INSERT OR IGNORE INTO change_request_records(record_hash,request_guid,parent_hash,depth,workspace_guid,item_guid,requester_guid,actor_guid,patch_json,before_json,comment,status,reason,payload_hash,ledger_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![
                record.get("recordHash").and_then(Value::as_str),record.get("requestGuid").and_then(Value::as_str),record.get("parentHash").and_then(Value::as_str),record.get("depth").and_then(Value::as_i64),
                record.get("workspaceGuid").and_then(Value::as_str),record.get("itemGuid").and_then(Value::as_str),record.get("requesterGuid").and_then(Value::as_str),record.get("actorGuid").and_then(Value::as_str),
                record.get("patch").map(Value::to_string),record.get("before").map(Value::to_string),record.get("comment").and_then(Value::as_str),record.get("status").and_then(Value::as_str),
                record.get("reason").and_then(Value::as_str),record.get("payloadHash").and_then(Value::as_str),record.get("ledgerHash").and_then(Value::as_str),record.get("createdAt").and_then(Value::as_str)]).unwrap_or(0);
            if inserted > 0 {
                ops += 1
            } else {
                skipped += 1
            }
        }
        if let Err(error) = rebuild_change_requests(conn) {
            return json!({"ok":false,"error":format!("Не удалось восстановить заявки: {error}")});
        }
    }
    if let Some(records) = journal.get("inventoryRecords").and_then(Value::as_array) {
        for record in records {
            let fields = record.get("fields").cloned().unwrap_or(Value::Null);
            let inserted=conn.execute("INSERT OR IGNORE INTO inventory_records(record_hash,session_guid,workspace_guid,actor_guid,kind,item_guid,number,expected_qty,actual_qty,checked,fields_json,payload_hash,ledger_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![record.get("recordHash").and_then(Value::as_str),record.get("sessionGuid").and_then(Value::as_str),record.get("workspaceGuid").and_then(Value::as_str),record.get("actorGuid").and_then(Value::as_str),record.get("kind").and_then(Value::as_str),record.get("itemGuid").and_then(Value::as_str),fields.get("number").and_then(Value::as_str),fields.get("expectedQty").and_then(Value::as_f64),fields.get("actualQty").and_then(Value::as_f64),fields.get("checked").and_then(Value::as_bool).map(i64::from),fields.to_string(),record.get("payloadHash").and_then(Value::as_str),record.get("ledgerHash").and_then(Value::as_str),record.get("createdAt").and_then(Value::as_str)]).unwrap_or(0);
            if inserted > 0 {
                ops += 1
            } else {
                skipped += 1
            }
        }
        if let Err(error) = rebuild_inventory_state(conn) {
            return json!({"ok":false,"error":format!("Не удалось восстановить инвентаризацию: {error}")});
        }
    }
    if let Some(arr) = journal.get("messages").and_then(Value::as_array) {
        for message in arr {
            let guid = message.get("guid").and_then(Value::as_str).unwrap_or("");
            let ledger_hash = message
                .get("ledgerHash")
                .and_then(Value::as_str)
                .unwrap_or("");
            let text = message.get("text").and_then(Value::as_str).unwrap_or("");
            let attachments = message
                .get("attachments")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if guid.is_empty()
                || ledger_hash.is_empty()
                || (text.is_empty() && attachments.as_array().is_none_or(|items| items.is_empty()))
                || text.chars().count() > 4000
            {
                skipped += 1;
                continue;
            }
            let Some(workspace) = message
                .get("workspaceGuid")
                .and_then(Value::as_str)
                .and_then(|value| id_by_guid(conn, "workspaces", value))
            else {
                skipped += 1;
                continue;
            };
            let Some(user) = message
                .get("userGuid")
                .and_then(Value::as_str)
                .and_then(|value| id_by_guid(conn, "users", value))
            else {
                skipped += 1;
                continue;
            };
            let inserted = conn
                .execute(
                    "INSERT OR IGNORE INTO chat_messages(guid,workspace_id,user_id,text,attachments_json,ledger_hash,created_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        guid,
                        workspace,
                        user,
                        text,
                        attachments.to_string(),
                        ledger_hash,
                        message.get("createdAt").and_then(Value::as_str).unwrap_or("")
                    ],
                )
                .unwrap_or(0);
            messages += inserted as u32;
            if inserted == 0 {
                skipped += 1;
            }
        }
    }
    if let Some(arr) = journal.get("invites").and_then(|v| v.as_array()) {
        for inv in arr {
            // v1 peers could explicitly export raw tokens. Accept those only
            // when the operator enabled the old compatibility switch, and
            // immediately convert them to a one-way digest at rest.
            let digest = inv
                .get("tokenDigest")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    let raw = inv.get("token").and_then(Value::as_str)?;
                    (std::env::var("MESHKEEPER_SYNC_INVITES").as_deref() == Ok("1"))
                        .then(|| hex::encode(Sha256::digest(raw.as_bytes())))
                })
                .unwrap_or_default();
            let ws_g = inv
                .get("workspaceGuid")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Some(ws) = id_by_guid(conn, "workspaces", ws_g) else {
                continue;
            };
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let encoded = format!("sha256:{}", digest.to_ascii_lowercase());
            let existing = find_invite_by_digest(conn, &digest);
            if let Some(id) = existing {
                // Состояние возможности только ужесточается при merge:
                // отзыв и больший счётчик использований нельзя откатить.
                let _ = conn.execute(
                        "UPDATE invites SET used_count=MAX(used_count,?2), revoked=MAX(revoked,?3) WHERE id=?1",
                        params![id, inv.get("usedCount").and_then(Value::as_i64).unwrap_or(0), if inv.get("revoked").and_then(Value::as_bool).unwrap_or(false) {1} else {0}],
                    );
            } else {
                let _ = conn.execute(
                    "INSERT INTO invites (workspace_id, token, role, max_uses, used_count, revoked, created_at, expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        ws,
                        encoded,
                        inv.get("role").and_then(|v| v.as_str()).unwrap_or("member"),
                        inv.get("maxUses").and_then(|v| v.as_i64()).unwrap_or(20),
                        inv.get("usedCount").and_then(|v| v.as_i64()).unwrap_or(0),
                        if inv.get("revoked").and_then(|v| v.as_bool()).unwrap_or(false) { 1 } else { 0 },
                        inv.get("createdAt").and_then(|v| v.as_str()).unwrap_or(""),
                        inv.get("expiresAt").and_then(Value::as_str),
                    ],
                );
            }
        }
    }
    if let Some(arr) = journal.get("memberships").and_then(|v| v.as_array()) {
        for m in arr {
            merge_membership_record(conn, m);
        }
    }
    json!({
        "ok": true,
        "workspaces": workspaces,
        "users": users,
        "items": items_n,
        "ops": ops,
        "skipped": skipped,
        "conflicts": conflicts
        ,"messages": messages
    })
}

fn merge_membership_record(conn: &Connection, record: &Value) {
    let Some(workspace_guid) = record.get("workspaceGuid").and_then(Value::as_str) else {
        return;
    };
    let Some(user_guid) = record.get("userGuid").and_then(Value::as_str) else {
        return;
    };
    let Some(revision) = record.get("revision").and_then(Value::as_i64) else {
        return;
    };
    let Some(active) = record.get("active").and_then(Value::as_bool) else {
        return;
    };
    let Some(incoming_hash) = record.get("versionHash").and_then(Value::as_str) else {
        return;
    };
    let local: Option<(i64, bool, String)> = conn
        .query_row(
            "SELECT revision,active,version_hash FROM membership_versions
             WHERE workspace_guid=?1 AND user_guid=?2",
            params![workspace_guid, user_guid],
            |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0, row.get(2)?)),
        )
        .optional()
        .ok()
        .flatten();
    let incoming_wins = match local {
        None => true,
        Some((local_revision, _, _)) if revision > local_revision => true,
        Some((local_revision, _, _)) if revision < local_revision => false,
        Some((_, true, _)) if !active => true,
        Some((_, false, _)) if active => false,
        Some((_, _, local_hash)) => incoming_hash > local_hash.as_str(),
    };
    if !incoming_wins {
        return;
    }
    let _ = conn.execute(
        "INSERT INTO membership_versions(workspace_guid,user_guid,revision,active,rights_json,position,role_name,personnel_number,ledger_hash,version_hash,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(workspace_guid,user_guid) DO UPDATE SET revision=excluded.revision,
           active=excluded.active,rights_json=excluded.rights_json,position=excluded.position,
           role_name=excluded.role_name,personnel_number=excluded.personnel_number,
           ledger_hash=excluded.ledger_hash,version_hash=excluded.version_hash,updated_at=excluded.updated_at",
        params![workspace_guid,user_guid,revision,i64::from(active),record.get("rights").and_then(Value::as_str),
            record.get("position").and_then(Value::as_str),record.get("roleName").and_then(Value::as_str),
            record.get("personnelNumber").and_then(Value::as_str),record.get("ledgerHash").and_then(Value::as_str),
            incoming_hash,record.get("updatedAt").and_then(Value::as_str).unwrap_or("")],
    );
    let Some(workspace) = id_by_guid(conn, "workspaces", workspace_guid) else {
        return;
    };
    let user = id_by_guid(conn, "users", user_guid);
    if !active {
        if let Some(user) = user {
            let _ = conn.execute(
                "DELETE FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
                params![user, workspace],
            );
            let other: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM user_workspaces WHERE user_id=?1",
                    [user],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if other == 0 {
                let _ = conn.execute("UPDATE users SET status='disabled' WHERE id=?1", [user]);
                let _ = conn.execute(
                    "UPDATE sessions SET revoked_at=?1 WHERE user_id=?2 AND revoked_at IS NULL",
                    params![chrono::Utc::now().to_rfc3339(), user],
                );
            }
        }
        return;
    }
    let Some(user) = user else { return };
    let rights = record.get("rights").and_then(Value::as_str);
    let position = record.get("position").and_then(Value::as_str);
    let role_name = record.get("roleName").and_then(Value::as_str);
    let personnel_number = record.get("personnelNumber").and_then(Value::as_str);
    let changed = conn
        .execute(
            "UPDATE user_workspaces SET rights_json=?1,position=?2,role_name=?3,personnel_number=?4
             WHERE user_id=?5 AND workspace_id=?6",
            params![
                rights,
                position,
                role_name,
                personnel_number,
                user,
                workspace
            ],
        )
        .unwrap_or(0);
    if changed == 0 {
        let _ = conn.execute(
            "INSERT INTO user_workspaces(user_id,workspace_id,rights_json,position,role_name,personnel_number)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![user,workspace,rights,position,role_name,personnel_number],
        );
    }
    let _ = conn.execute("UPDATE users SET status=CASE WHEN status='disabled' THEN 'active' ELSE status END WHERE id=?1", [user]);
}

fn notify_conflict(conn: &Connection, ws: i64, item_id: i64, text: &str) {
    if let Ok(mut stmt) = conn.prepare("SELECT user_id FROM user_workspaces WHERE workspace_id=?1")
    {
        let ids: Vec<i64> = stmt
            .query_map(params![ws], |r| r.get(0))
            .ok()
            .map(|r| r.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        for uid in ids {
            let _ = conn.execute(
                "INSERT INTO notifications (user_id, item_id, type, title, text, created_at) VALUES (?1,?2,'system','Конфликт выдачи',?3,?4)",
                params![uid, item_id, text, chrono::Utc::now().to_rfc3339()],
            );
        }
    }
}

pub fn list_peers(conn: &Connection) -> Value {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id, node_id, url, name, last_seen, last_sync, last_error FROM peers ORDER BY id DESC") {
        for row in stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "nodeId": r.get::<_, Option<String>>(1)?,
                "url": r.get::<_, String>(2)?,
                "name": r.get::<_, Option<String>>(3)?,
                "lastSeen": r.get::<_, Option<String>>(4)?,
                "lastSync": r.get::<_, Option<String>>(5)?,
                "lastError": r.get::<_, Option<String>>(6)?,
            }))
        }).into_iter().flatten().flatten() {
            out.push(row);
        }
    }
    Value::Array(out)
}

pub fn peer_urls(conn: &Connection) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare("SELECT url FROM peers ORDER BY id") else {
        return Vec::new();
    };
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

pub fn remove_peer(conn: &Connection, url: &str) -> Value {
    let url = url.trim().trim_end_matches('/');
    let removed = conn
        .execute("DELETE FROM peers WHERE url=?1", params![url])
        .unwrap_or(0);
    json!({"ok": true, "removed": removed})
}

pub fn add_peer(conn: &Connection, url: &str, name: Option<&str>, node_id: Option<&str>) -> Value {
    let url = url.trim().trim_end_matches('/').to_string();
    let exists: bool = conn
        .query_row("SELECT 1 FROM peers WHERE url=?1", params![url], |_| {
            Ok(true)
        })
        .optional()
        .ok()
        .flatten()
        .unwrap_or(false);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM peers", [], |row| row.get(0))
        .unwrap_or(0);
    if !exists && count >= MAX_PEERS {
        return json!({"ok": false, "error": format!("Достигнут лимит {MAX_PEERS} прямых peers")});
    }
    let _ = conn.execute(
        "INSERT INTO peers (url, name, node_id, last_seen) VALUES (?1,?2,?3,?4)
         ON CONFLICT(url) DO UPDATE SET name=COALESCE(excluded.name, peers.name), node_id=COALESCE(excluded.node_id, peers.node_id), last_seen=excluded.last_seen",
        params![url, name, node_id, chrono::Utc::now().to_rfc3339()],
    );
    json!({"ok": true, "url": url, "added": !exists})
}

pub fn list_conflicts(conn: &Connection, workspace_id: i64) -> Value {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id, workspace_id, item_id, item_guid, status, description, left_label, right_label, created_at FROM conflicts WHERE workspace_id=?1 ORDER BY id DESC LIMIT 200") {
        for row in stmt.query_map([workspace_id], |r| {
            let item_id: Option<i64> = r.get(2)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "workspaceId": r.get::<_, Option<i64>>(1)?,
                "itemId": item_id,
                "itemGuid": r.get::<_, Option<String>>(3)?,
                "status": r.get::<_, String>(4)?,
                "description": r.get::<_, String>(5)?,
                "leftLabel": r.get::<_, Option<String>>(6)?,
                "rightLabel": r.get::<_, Option<String>>(7)?,
                "createdAt": r.get::<_, String>(8)?,
                "item": item_id.and_then(|i| jsn::item_json(conn, i, false)),
            }))
        }).into_iter().flatten().flatten() {
            out.push(row);
        }
    }
    Value::Array(out)
}

pub fn resolve_conflict(
    conn: &Connection,
    id: i64,
    responsible: Option<i64>,
    uid: i64,
) -> anyhow::Result<Value> {
    let (item_id, ws): (i64, i64) = conn.query_row(
        "SELECT item_id, workspace_id FROM conflicts WHERE id=?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    conn.execute(
        "UPDATE conflicts SET status='resolved', resolved_at=?1, resolver_id=?2 WHERE id=?3",
        params![chrono::Utc::now().to_rfc3339(), uid, id],
    )?;
    let slug = if responsible.is_some() {
        "in-work"
    } else {
        "in-stock"
    };
    if let Some(st) = status_id(conn, ws, slug) {
        conn.execute(
            "UPDATE items SET responsible_user_id=?1, status_id=?2 WHERE id=?3",
            params![responsible, st, item_id],
        )?;
    }
    let _ = ledger::append(
        conn,
        ws,
        uid,
        Some(item_id),
        "update",
        None,
        None,
        None,
        Some("Конфликт выдачи разрешён администратором"),
    );
    Ok(json!({"ok": true}))
}

pub fn local_http_base() -> String {
    let bind = std::env::var("MESHKEEPER_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let port = bind.rsplit(':').next().unwrap_or("8080");
    format!("http://127.0.0.1:{port}")
}

pub fn guess_lan_base() -> String {
    if let Ok(url) = std::env::var("MESHKEEPER_ADVERTISE_URL") {
        let normalized = url.trim().trim_end_matches('/');
        if normalized.starts_with("http://") || normalized.starts_with("https://") {
            return normalized.to_string();
        }
    }
    let bind = std::env::var("MESHKEEPER_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    // Явная bind-точка уже является лучшим обратным маршрутом. Это особенно
    // важно для loopback/LAN-only узлов: нельзя рекламировать адрес другого
    // интерфейса, на котором процесс фактически не слушает.
    if let Ok(address) = bind.parse::<std::net::SocketAddr>() {
        if !address.ip().is_unspecified() {
            return format!("http://{address}");
        }
    }
    let port = bind.rsplit(':').next().unwrap_or("8080");
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        let _ = sock.connect("8.8.8.8:80");
        if let Ok(addr) = sock.local_addr() {
            return format!("http://{}:{port}", addr.ip());
        }
    }
    format!("http://127.0.0.1:{port}")
}

pub fn apply_remote_journal(conn: &Connection, journal: &Value, peer_url: &str) -> Value {
    if let Err(error) = ledger::verify_journal(journal) {
        return json!({"ok":false,"error":format!("Криптографическая проверка снимка: {error}")});
    }
    if let Err(error) = verify_device_registry(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка устройств: {error}")});
    }
    if let Err(error) = verify_custody_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка custody-летописи: {error}")});
    }
    if let Err(error) = verify_membership_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка членства: {error}")});
    }
    if let Err(error) = verify_item_tombstones(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка tombstone ТМЦ: {error}")});
    }
    if let Err(error) = verify_item_comments(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка комментариев ТМЦ: {error}")});
    }
    if let Err(error) = verify_fault_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка летописи неисправностей: {error}")});
    }
    if let Err(error) = verify_change_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка заявок на изменение: {error}")});
    }
    if let Err(error) = verify_config_versions(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка летописи справочников: {error}")});
    }
    if let Err(error) = verify_organization_node_versions(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка летописи структуры: {error}")});
    }
    if let Err(error) = verify_item_state_versions(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка master-летописи ТМЦ: {error}")});
    }
    if let Err(error) = verify_inventory_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка летописи инвентаризации: {error}")});
    }
    if let Err(error) = verify_photo_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка фото-летописи: {error}")});
    }
    if let Err(error) = verify_document_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка летописи документов: {error}")});
    }
    if let Err(error) = verify_knowledge_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка intent базы знаний: {error}")});
    }
    if let Err(error) = verify_chat_records(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка intent чата: {error}")});
    }
    if let Err(error) = verify_sale_offers(journal) {
        return json!({"ok":false,"error":format!("Проверка предложений Bit: {error}")});
    }
    if let Err(error) = crate::accounting::verify_journal_links(conn, journal) {
        return json!({"ok":false,"error":format!("Проверка Bit-летописи: {error}")});
    }
    if let Err(error) = enforce_node_trust(conn, journal, peer_url) {
        return json!({"ok":false,"error":format!("Ключ mesh-ноды не разрешён: {error}")});
    }
    let receipt = match journal_receipt(conn, journal) {
        Ok(receipt) => receipt,
        Err(error) => {
            return json!({"ok":false,"error":format!("Защита от rollback/replay: {error}")});
        }
    };
    if receipt.duplicate {
        return json!({"ok":true,"duplicate":true,"ops":0,"skipped":0});
    }
    if let Err(error) = conn.execute_batch("SAVEPOINT verified_sync") {
        return json!({"ok":false,"error":error.to_string()});
    }
    let result = import_journal(conn, journal);
    if result.get("ok").and_then(Value::as_bool) != Some(true) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return result;
    }
    if let Some(knowledge) = journal.get("knowledge") {
        if let Err(error) = crate::knowledge::import(conn, knowledge) {
            let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
            return json!({"ok":false,"error":format!("База знаний отклонена: {error}")});
        }
    }
    if let Some(accounting) = journal.get("accounting") {
        if let Err(error) = crate::accounting::import(conn, accounting) {
            let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
            return json!({"ok":false,"error":format!("Бухгалтерская летопись отклонена: {error}")});
        }
    }
    if let Err(error) = crate::content::observe_journal(conn, journal, peer_url) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("CAS-каталог отклонён: {error}")});
    }
    if let Err(error) = ledger::verify_all(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Криптографическая проверка входящего журнала: {error}")});
    }
    if let Err(error) = verify_stored_device_bindings(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка привязки устройств: {error}")});
    }
    if let Err(error) = verify_stored_custody(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённой custody-летописи: {error}")});
    }
    if let Err(error) = verify_stored_item_tombstones(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённых tombstone ТМЦ: {error}")});
    }
    if let Err(error) = verify_stored_item_comments(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённых комментариев ТМЦ: {error}")});
    }
    if let Err(error) = verify_stored_fault_records(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённой летописи неисправностей: {error}")});
    }
    if let Err(error) = verify_stored_change_records(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённых заявок на изменение: {error}")});
    }
    if let Err(error) = verify_stored_config_versions(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённой летописи справочников: {error}")});
    }
    if let Err(error) = verify_stored_organization_node_versions(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённой структуры: {error}")});
    }
    if let Err(error) = verify_stored_item_state_versions(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённой master-летописи ТМЦ: {error}")});
    }
    if let Err(error) = verify_stored_inventory_records(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённой инвентаризации: {error}")});
    }
    if let Err(error) = verify_photo_records(conn, &export_journal(conn)) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённой фото-летописи: {error}")});
    }
    if let Err(error) = verify_document_records(conn, &export_journal(conn)) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённой летописи документов: {error}")});
    }
    if let Err(error) = verify_knowledge_records(conn, &export_journal(conn)) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённого intent базы знаний: {error}")});
    }
    if let Err(error) = verify_chat_records(conn, &export_journal(conn)) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённого intent чата: {error}")});
    }
    if let Err(error) = verify_sale_offers(&export_journal(conn)) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка сохранённых предложений Bit: {error}")});
    }
    if let Err(error) = ledger::verify_chat_links(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Криптографическая проверка сообщений: {error}")});
    }
    if let Err(error) = crate::accounting::verify(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка бухгалтерской летописи: {error}")});
    }
    if let Err(error) = crate::knowledge::verify(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка базы знаний: {error}")});
    }
    if let Err(error) = store_journal_receipt(conn, &receipt) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Не удалось сохранить anti-rollback квитанцию: {error}")});
    }
    if let Err(error) = conn.execute_batch("RELEASE verified_sync") {
        return json!({"ok":false,"error":error.to_string()});
    }
    let name = journal.get("nodeName").and_then(|v| v.as_str());
    let nid = journal.get("nodeId").and_then(|v| v.as_str());
    if crate::validate_peer_url(peer_url).is_ok() {
        add_peer(conn, peer_url, name, nid);
        let _ = conn.execute(
            "UPDATE peers SET last_sync=?1, last_error=NULL WHERE url=?2",
            params![
                chrono::Utc::now().to_rfc3339(),
                peer_url.trim().trim_end_matches('/')
            ],
        );
        resolve_peer_error(conn, peer_url);
    }
    result
}

#[derive(Clone)]
struct PortableDevice {
    user_guid: String,
    public_key: String,
    created_at: String,
    revoked_at: Option<String>,
}

fn incoming_devices(journal: &Value) -> anyhow::Result<HashMap<String, PortableDevice>> {
    let users: HashSet<&str> = journal
        .get("users")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("guid").and_then(Value::as_str))
        .collect();
    let mut devices = HashMap::new();
    for row in journal
        .get("devices")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal does not provide portable device registry"))?
    {
        let required = |field: &str| {
            row.get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("device has no {field}"))
        };
        let device_id = required("deviceId")?;
        let user_guid = required("userGuid")?;
        let public_key = required("publicKey")?;
        let created_at = required("createdAt")?;
        if device_id.len() < 16 || device_id.len() > 100 || !users.contains(user_guid) {
            anyhow::bail!("device identity is outside signed user set");
        }
        let raw = URL_SAFE_NO_PAD
            .decode(public_key)
            .map_err(|_| anyhow::anyhow!("invalid device public key encoding"))?;
        if raw.len() != 32 {
            anyhow::bail!("invalid device public key length");
        }
        chrono::DateTime::parse_from_rfc3339(created_at)
            .map_err(|_| anyhow::anyhow!("invalid device creation timestamp"))?;
        let revoked_at = row
            .get("revokedAt")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if let Some(value) = revoked_at.as_deref() {
            chrono::DateTime::parse_from_rfc3339(value)
                .map_err(|_| anyhow::anyhow!("invalid device revocation timestamp"))?;
            if value < created_at {
                anyhow::bail!("device was revoked before it was created");
            }
        }
        let record = PortableDevice {
            user_guid: user_guid.to_owned(),
            public_key: public_key.to_owned(),
            created_at: created_at.to_owned(),
            revoked_at,
        };
        if let Some(previous) = devices.insert(device_id.to_owned(), record.clone()) {
            if previous.user_guid != record.user_guid
                || previous.public_key != record.public_key
                || previous.created_at != record.created_at
                || previous.revoked_at != record.revoked_at
            {
                anyhow::bail!("conflicting duplicate device identity");
            }
            anyhow::bail!("duplicate device identity");
        }
    }
    Ok(devices)
}

fn local_device(conn: &Connection, device_id: &str) -> anyhow::Result<Option<PortableDevice>> {
    conn.query_row(
        "SELECT u.guid,d.public_key,d.created_at,d.revoked_at
         FROM user_devices d JOIN users u ON u.id=d.user_id WHERE d.device_id=?1",
        [device_id],
        |row| {
            Ok(PortableDevice {
                user_guid: row.get(0)?,
                public_key: row.get(1)?,
                created_at: row.get(2)?,
                revoked_at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn verify_device_registry(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    let devices = incoming_devices(journal)?;
    for (device_id, incoming) in &devices {
        if let Some(local) = local_device(conn, device_id)? {
            if local.user_guid != incoming.user_guid || local.public_key != incoming.public_key {
                anyhow::bail!("device identity conflicts with local registry");
            }
            // Отзыв является monotonic tombstone: старый offline snapshot с
            // active-записью допустим, но importer не имеет права воскресить ключ.
        }
    }
    for event in journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let proof_fields = [
            "requestDeviceId",
            "requestPublicKey",
            "requestNonce",
            "requestSignature",
            "requestHash",
            "requestTimestamp",
            "requestPath",
        ];
        let present = proof_fields
            .iter()
            .filter(|field| event.get(**field).and_then(Value::as_str).is_some())
            .count();
        if present == 0 {
            continue;
        }
        if present != proof_fields.len() {
            anyhow::bail!("incomplete device proof in journal event");
        }
        let device_id = event["requestDeviceId"].as_str().unwrap_or_default();
        let proof_key = event["requestPublicKey"].as_str().unwrap_or_default();
        let actor = event["actorGuid"].as_str().unwrap_or_default();
        let created_at = event["createdAt"].as_str().unwrap_or_default();
        let device = devices
            .get(device_id)
            .cloned()
            .or(local_device(conn, device_id)?)
            .ok_or_else(|| anyhow::anyhow!("device proof has no registered identity"))?;
        if device.user_guid != actor || device.public_key != proof_key {
            anyhow::bail!("device proof is not bound to event actor");
        }
        let event_time = chrono::DateTime::parse_from_rfc3339(created_at)
            .map_err(|_| anyhow::anyhow!("invalid device-proof event timestamp"))?;
        let registered_time = chrono::DateTime::parse_from_rfc3339(&device.created_at)
            .map_err(|_| anyhow::anyhow!("invalid registered device timestamp"))?;
        let revoked_before_event = device
            .revoked_at
            .as_deref()
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid registered revocation timestamp"))?
            .is_some_and(|revoked| event_time >= revoked);
        if event_time < registered_time || revoked_before_event {
            anyhow::bail!("device proof was produced outside device validity interval");
        }
    }
    Ok(())
}

fn verify_stored_device_bindings(conn: &Connection) -> anyhow::Result<usize> {
    let mut statement = conn.prepare(
        "SELECT h.request_device_id,h.request_public_key,h.created_at,
                u.guid,du.public_key,du.created_at,du.revoked_at,du.user_id,h.actor_user_id
         FROM history_entries h
         JOIN users u ON u.id=h.actor_user_id
         LEFT JOIN user_devices du ON du.device_id=h.request_device_id
         WHERE h.request_device_id IS NOT NULL",
    )?;
    let mut rows = statement.query([])?;
    let mut verified = 0;
    while let Some(row) = rows.next()? {
        let device_id: String = row.get(0)?;
        let proof_key: Option<String> = row.get(1)?;
        let event_created: String = row.get(2)?;
        let actor_guid: String = row.get(3)?;
        let registered_key: Option<String> = row.get(4)?;
        let registered_at: Option<String> = row.get(5)?;
        let revoked_at: Option<String> = row.get(6)?;
        let device_user: Option<i64> = row.get(7)?;
        let actor_user: i64 = row.get(8)?;
        let (Some(proof_key), Some(registered_key), Some(registered_at), Some(device_user)) =
            (proof_key, registered_key, registered_at, device_user)
        else {
            anyhow::bail!("device proof {device_id} has no stored identity");
        };
        if proof_key != registered_key || device_user != actor_user {
            anyhow::bail!("stored device proof {device_id} is not bound to actor {actor_guid}");
        }
        let event_time = chrono::DateTime::parse_from_rfc3339(&event_created)
            .map_err(|_| anyhow::anyhow!("invalid stored event timestamp"))?;
        let registered_time = chrono::DateTime::parse_from_rfc3339(&registered_at)
            .map_err(|_| anyhow::anyhow!("invalid stored device timestamp"))?;
        let revoked_before_event = revoked_at
            .as_deref()
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid stored revocation timestamp"))?
            .is_some_and(|revoked| event_time >= revoked);
        if event_time < registered_time || revoked_before_event {
            anyhow::bail!("stored device proof {device_id} is outside validity interval");
        }
        verified += 1;
    }
    Ok(verified)
}

struct CustodyLedgerEvidence {
    workspace_guid: String,
    item_guid: String,
    actor_guid: String,
    to_label: Option<String>,
    operation: String,
    quantity: Option<f64>,
    created_at: String,
    has_device_proof: bool,
}

fn custody_ledger_evidence(
    conn: &Connection,
    incoming: &HashMap<&str, &Value>,
    hash: &str,
) -> anyhow::Result<CustodyLedgerEvidence> {
    if let Some(event) = incoming.get(hash) {
        let present = |field: &str| {
            event
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        };
        return Ok(CustodyLedgerEvidence {
            workspace_guid: event
                .get("workspaceGuid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            item_guid: event
                .get("itemGuid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            actor_guid: event
                .get("actorGuid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            to_label: event
                .get("toLabel")
                .and_then(Value::as_str)
                .map(str::to_owned),
            operation: event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            quantity: event.get("quantityDelta").and_then(Value::as_f64),
            created_at: event
                .get("createdAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            has_device_proof: [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ]
            .into_iter()
            .all(present),
        });
    }
    conn.query_row(
        "SELECT w.guid,i.guid,u.guid,h.to_label,h.type,h.quantity_delta,h.created_at,
                h.request_device_id IS NOT NULL AND h.request_device_id!='' AND
                h.request_public_key IS NOT NULL AND h.request_public_key!='' AND
                h.request_nonce IS NOT NULL AND h.request_nonce!='' AND
                h.request_signature IS NOT NULL AND h.request_signature!='' AND
                h.request_hash IS NOT NULL AND h.request_hash!='' AND
                h.request_timestamp IS NOT NULL AND h.request_timestamp!='' AND
                h.request_path IS NOT NULL AND h.request_path!=''
         FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id
         JOIN items i ON i.id=h.item_id JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1",
        [hash],
        |row| {
            Ok(CustodyLedgerEvidence {
                workspace_guid: row.get(0)?,
                item_guid: row.get(1)?,
                actor_guid: row.get(2)?,
                to_label: row.get(3)?,
                operation: row.get(4)?,
                quantity: row.get(5)?,
                created_at: row.get(6)?,
                has_device_proof: row.get::<_, i64>(7)? != 0,
            })
        },
    )
    .map_err(Into::into)
}

fn verify_custody_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("custodyMode").and_then(Value::as_str) != Some("ledger-delta/v1") {
        anyhow::bail!("journal does not provide ledger custody entries");
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    let mut hashes = HashSet::new();
    let mut ledger_hashes = HashSet::new();
    for record in journal
        .get("custody")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no custody array"))?
    {
        let required = |field: &str| {
            record
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("custody record has no {field}"))
        };
        let entry_hash = required("entryHash")?;
        let workspace = required("workspaceGuid")?;
        let item = required("itemGuid")?;
        let user = required("userGuid")?;
        let ledger_hash = required("ledgerHash")?;
        let created_at = required("createdAt")?;
        let delta = record
            .get("quantityDelta")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && value.abs() >= 1e-9)
            .ok_or_else(|| anyhow::anyhow!("invalid custody quantity"))?;
        let due_at = record.get("dueAt").and_then(Value::as_str);
        let comment = record.get("comment").and_then(Value::as_str);
        let photo_url = record.get("photoUrl").and_then(Value::as_str);
        if comment.is_some_and(|value| value.chars().count() > 2000)
            || photo_url.is_some_and(|value| value.len() > 512)
            || !hashes.insert(entry_hash)
            || !ledger_hashes.insert(ledger_hash)
        {
            anyhow::bail!("duplicate or oversized custody record");
        }
        if custody_entry_hash(
            workspace,
            item,
            user,
            delta,
            due_at,
            comment,
            photo_url,
            ledger_hash,
            created_at,
        ) != entry_hash
        {
            anyhow::bail!("custody entry hash mismatch");
        }
        let evidence = custody_ledger_evidence(conn, &history, ledger_hash)
            .map_err(|_| anyhow::anyhow!("custody ledger event is unavailable"))?;
        let expected_type = if delta > 0.0 {
            "transfer_receive"
        } else {
            "transfer_send"
        };
        let quantity_matches = evidence
            .quantity
            .map(|quantity| (quantity.abs() - delta.abs()).abs() < 1e-9)
            .unwrap_or_else(|| (delta.abs() - 1.0).abs() < 1e-9);
        let actor_matches = evidence.actor_guid == user
            || (delta > 0.0 && evidence.to_label.as_deref() == Some(user));
        if evidence.workspace_guid != workspace
            || evidence.item_guid != item
            || !actor_matches
            || evidence.operation != expected_type
            || evidence.created_at != created_at
            || !quantity_matches
            || !evidence.has_device_proof
        {
            anyhow::bail!("custody record is not semantically bound to ledger event");
        }
    }
    Ok(())
}

fn verify_stored_custody(conn: &Connection) -> anyhow::Result<usize> {
    let mut statement = conn.prepare(
        "SELECT entry_hash,workspace_guid,item_guid,user_guid,quantity_delta,due_at,comment,
                photo_url,ledger_hash,created_at FROM custody_entries",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
        ))
    })?;
    let empty = HashMap::new();
    let mut verified = 0;
    for row in rows {
        let (hash, workspace, item, user, delta, due, comment, photo, ledger_hash, created) = row?;
        if custody_entry_hash(
            &workspace,
            &item,
            &user,
            delta,
            due.as_deref(),
            comment.as_deref(),
            photo.as_deref(),
            &ledger_hash,
            &created,
        ) != hash
        {
            anyhow::bail!("stored custody entry hash mismatch");
        }
        let evidence = custody_ledger_evidence(conn, &empty, &ledger_hash)?;
        let expected = if delta > 0.0 {
            "transfer_receive"
        } else {
            "transfer_send"
        };
        let quantity_matches = evidence
            .quantity
            .map(|quantity| (quantity.abs() - delta.abs()).abs() < 1e-9)
            .unwrap_or_else(|| (delta.abs() - 1.0).abs() < 1e-9);
        let actor_matches = evidence.actor_guid == user
            || (delta > 0.0 && evidence.to_label.as_deref() == Some(user.as_str()));
        if evidence.workspace_guid != workspace
            || evidence.item_guid != item
            || !actor_matches
            || evidence.operation != expected
            || evidence.created_at != created
            || !quantity_matches
            || !evidence.has_device_proof
        {
            anyhow::bail!("stored custody entry is not bound to ledger event");
        }
        verified += 1;
    }
    Ok(verified)
}

struct MembershipLedgerEvidence {
    operation: String,
    actor_guid: String,
    from_label: Option<String>,
    to_label: Option<String>,
    has_device_proof: bool,
}

fn membership_ledger_evidence(
    conn: &Connection,
    incoming: &HashMap<&str, &Value>,
    hash: &str,
) -> anyhow::Result<MembershipLedgerEvidence> {
    if let Some(event) = incoming.get(hash) {
        let present = |field: &str| {
            event
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        };
        return Ok(MembershipLedgerEvidence {
            operation: event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            actor_guid: event
                .get("actorGuid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            from_label: event
                .get("fromLabel")
                .and_then(Value::as_str)
                .map(str::to_owned),
            to_label: event
                .get("toLabel")
                .and_then(Value::as_str)
                .map(str::to_owned),
            has_device_proof: [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ]
            .into_iter()
            .all(present),
        });
    }
    conn.query_row(
        "SELECT h.type,u.guid,h.from_label,h.to_label,
                h.request_device_id IS NOT NULL AND h.request_device_id!='' AND
                h.request_public_key IS NOT NULL AND h.request_public_key!='' AND
                h.request_nonce IS NOT NULL AND h.request_nonce!='' AND
                h.request_signature IS NOT NULL AND h.request_signature!='' AND
                h.request_hash IS NOT NULL AND h.request_hash!='' AND
                h.request_timestamp IS NOT NULL AND h.request_timestamp!='' AND
                h.request_path IS NOT NULL AND h.request_path!=''
         FROM history_entries h JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1",
        [hash],
        |row| {
            Ok(MembershipLedgerEvidence {
                operation: row.get(0)?,
                actor_guid: row.get(1)?,
                from_label: row.get(2)?,
                to_label: row.get(3)?,
                has_device_proof: row.get::<_, i64>(4)? != 0,
            })
        },
    )
    .map_err(Into::into)
}

fn verify_membership_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("membershipMode").and_then(Value::as_str) != Some("versioned-tombstones/v1") {
        anyhow::bail!("journal does not provide membership tombstones");
    }
    let incoming_history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    let mut membership_keys = HashSet::new();
    for record in journal
        .get("memberships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let workspace = record
            .get("workspaceGuid")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("membership has no workspace GUID"))?;
        let user = record
            .get("userGuid")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("membership has no user GUID"))?;
        if !membership_keys.insert((workspace.to_owned(), user.to_owned())) {
            anyhow::bail!("duplicate membership version");
        }
        let revision = record
            .get("revision")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow::anyhow!("membership has invalid revision"))?;
        let active = record
            .get("active")
            .and_then(Value::as_bool)
            .ok_or_else(|| anyhow::anyhow!("membership has no active flag"))?;
        let ledger_hash = record.get("ledgerHash").and_then(Value::as_str);
        let fields = MembershipFields::from_json(record);
        let expected =
            membership_version_hash(workspace, user, revision, active, &fields, ledger_hash);
        if record.get("versionHash").and_then(Value::as_str) != Some(expected.as_str()) {
            anyhow::bail!("membership version hash mismatch");
        }
        if revision > 1 && ledger_hash.is_none() {
            anyhow::bail!("non-legacy membership has no ledger event");
        }
        if let Some(hash) = ledger_hash {
            let evidence = membership_ledger_evidence(conn, &incoming_history, hash)
                .map_err(|_| anyhow::anyhow!("membership ledger event is unavailable"))?;
            let expected = if active {
                if revision == 1 {
                    matches!(
                        evidence.operation.as_str(),
                        "membership_create" | "membership_join" | "workspace_create"
                    )
                } else {
                    matches!(
                        evidence.operation.as_str(),
                        "membership_update" | "membership_join"
                    )
                }
            } else {
                evidence.operation == "membership_remove"
            };
            if !expected {
                anyhow::bail!("membership references incompatible ledger operation");
            }
            match evidence.operation.as_str() {
                "membership_create" => {
                    if evidence.from_label.is_some() || evidence.to_label.as_deref() != Some(user) {
                        anyhow::bail!("membership creation target does not match version user");
                    }
                }
                "membership_update" => {
                    if evidence.from_label.as_deref() != Some(user)
                        || evidence.to_label.as_deref() != Some(user)
                    {
                        anyhow::bail!("membership event target does not match version user");
                    }
                }
                "membership_remove" => {
                    if evidence.from_label.as_deref() != Some(user) || evidence.to_label.is_some() {
                        anyhow::bail!("membership removal target does not match version user");
                    }
                }
                "membership_join" => {
                    if evidence.actor_guid != user {
                        anyhow::bail!("membership join actor does not match user");
                    }
                }
                "workspace_create" => {
                    if evidence.actor_guid != user
                        || evidence.to_label.as_deref() != Some(workspace)
                    {
                        anyhow::bail!("workspace creation does not bind owner membership");
                    }
                }
                _ => unreachable!("operation was checked above"),
            }
            if evidence.operation != "membership_join" && !evidence.has_device_proof {
                anyhow::bail!("administrative membership event has no device proof");
            }
        }
    }
    Ok(())
}

fn valid_portable_change(value: &Value) -> bool {
    let allowed = [
        "title",
        "categoryId",
        "brandId",
        "statusId",
        "responsibleUserId",
        "buildingSiteId",
        "storageId",
        "serialNumber",
        "cost",
        "comment",
        "qrCode",
        "calibratedUntil",
        "minQuantity",
    ];
    value.as_object().is_some_and(|object| {
        !object.is_empty()
            && object.iter().all(|(key, value)| {
                allowed.contains(&key.as_str())
                    && (value.is_null()
                        || !value.is_object()
                        || value
                            .get("$ref")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| {
                                matches!(
                                    kind,
                                    "user"
                                        | "status"
                                        | "categories"
                                        | "brands"
                                        | "building_sites"
                                        | "storages"
                                )
                            }))
            })
    })
}

fn verify_change_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("changeRequestMode").and_then(Value::as_str) != Some("portable-branches/v1") {
        anyhow::bail!("journal does not provide portable change requests");
    }
    let records = journal
        .get("changeRequests")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no change request array"))?;
    let by_hash: HashMap<&str, &Value> = records
        .iter()
        .filter_map(|r| r.get("recordHash").and_then(Value::as_str).map(|h| (h, r)))
        .collect();
    if by_hash.len() != records.len() {
        anyhow::bail!("duplicate change request record");
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("opId").and_then(Value::as_str).map(|h| (h, e)))
        .collect();
    for record in records {
        let get = |name: &str| {
            record
                .get(name)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow::anyhow!("change record has no {name}"))
        };
        let hash = get("recordHash")?;
        let ledger_hash = get("ledgerHash")?;
        let payload_hash = get("payloadHash")?;
        let request = get("requestGuid")?;
        let workspace = get("workspaceGuid")?;
        let item = get("itemGuid")?;
        let actor = get("actorGuid")?;
        let requester = get("requesterGuid")?;
        let depth = record
            .get("depth")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("change depth missing"))?;
        let status = get("status")?;
        if !matches!(status, "pending" | "accepted" | "rejected")
            || !valid_portable_change(&record["patch"])
            || !valid_portable_change(&record["before"])
        {
            anyhow::bail!("invalid portable change fields");
        }
        if change_payload_hash(record)? != payload_hash
            || change_record_hash(payload_hash, ledger_hash) != hash
        {
            anyhow::bail!("change request hash mismatch");
        }
        let parent = record.get("parentHash").and_then(Value::as_str);
        if depth == 0 {
            if parent.is_some() {
                anyhow::bail!("change root has parent");
            }
        } else {
            let p = parent
                .and_then(|h| by_hash.get(h).copied())
                .ok_or_else(|| anyhow::anyhow!("change parent unavailable"))?;
            if p.get("depth").and_then(Value::as_i64) != Some(depth - 1) {
                anyhow::bail!("change depth does not follow parent");
            }
            for field in [
                "requestGuid",
                "workspaceGuid",
                "itemGuid",
                "requesterGuid",
                "patch",
                "before",
                "comment",
            ] {
                if p.get(field) != record.get(field) {
                    anyhow::bail!("change immutable field changed");
                }
            }
        }
        let event_type = if depth == 0 && status == "pending" && actor == requester {
            "change_request"
        } else if depth == 0 {
            "change_adopt"
        } else {
            "change_decision"
        };
        if let Some(event) = history.get(ledger_hash) {
            let proof = [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ];
            if event.get("type").and_then(Value::as_str) != Some(event_type)
                || event.get("workspaceGuid").and_then(Value::as_str) != Some(workspace)
                || event.get("itemGuid").and_then(Value::as_str) != Some(item)
                || event.get("actorGuid").and_then(Value::as_str) != Some(actor)
                || event.get("fromLabel").and_then(Value::as_str) != Some(request)
                || event.get("toLabel").and_then(Value::as_str) != Some(payload_hash)
                || !proof.iter().all(|name| {
                    event
                        .get(*name)
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty())
                })
            {
                anyhow::bail!("change request ledger evidence mismatch");
            }
        } else {
            let exists:i64=conn.query_row("SELECT count(*) FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN items i ON i.id=h.item_id JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1 AND h.type=?2 AND w.guid=?3 AND i.guid=?4 AND u.guid=?5 AND h.from_label=?6 AND h.to_label=?7 AND h.request_device_id IS NOT NULL AND h.request_signature IS NOT NULL",params![ledger_hash,event_type,workspace,item,actor,request,payload_hash],|r|r.get(0))?;
            if exists == 0 {
                anyhow::bail!("change request ledger event unavailable");
            }
        }
    }
    Ok(())
}

fn verify_stored_change_records(conn: &Connection) -> anyhow::Result<usize> {
    let snapshot = export_journal(conn);
    verify_change_records(conn, &snapshot)?;
    Ok(snapshot["changeRequests"].as_array().map_or(0, Vec::len))
}

fn verify_organization_node_versions(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("organizationNodeMode").and_then(Value::as_str) != Some("portable-branches/v1") {
        anyhow::bail!("journal does not provide portable organization structure");
    }
    let records = journal
        .get("organizationNodeVersions")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no organization node versions"))?;
    let by_hash: HashMap<&str, &Value> = records
        .iter()
        .filter_map(|record| {
            record
                .get("versionHash")
                .and_then(Value::as_str)
                .map(|hash| (hash, record))
        })
        .collect();
    if by_hash.len() != records.len() {
        anyhow::bail!("duplicate organization node version");
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    for record in records {
        let get = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("organization node version has no {key}"))
        };
        let version = get("versionHash")?;
        let node = get("nodeGuid")?;
        let workspace = get("workspaceGuid")?;
        let actor = get("actorGuid")?;
        let payload = get("payloadHash")?;
        let ledger_hash = get("ledgerHash")?;
        let depth = record
            .get("depth")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("organization node depth missing"))?;
        let active = record
            .get("active")
            .and_then(Value::as_bool)
            .ok_or_else(|| anyhow::anyhow!("organization node active missing"))?;
        let fields = record
            .get("fields")
            .ok_or_else(|| anyhow::anyhow!("organization node fields missing"))?;
        if fields
            .get("name")
            .and_then(Value::as_str)
            .is_none_or(|name| name.is_empty() || name.chars().count() > 120)
            || fields
                .get("kind")
                .and_then(Value::as_str)
                .is_none_or(|kind| kind.is_empty() || kind.chars().count() > 40)
        {
            anyhow::bail!("invalid organization node fields");
        }
        if organization_node_payload_hash(record)? != payload
            || organization_node_version_hash(payload, ledger_hash) != version
        {
            anyhow::bail!("organization node version hash mismatch");
        }
        let parent = record.get("parentHash").and_then(Value::as_str);
        if depth == 0 {
            if parent.is_some() {
                anyhow::bail!("organization node root has parent");
            }
        } else {
            let valid = if let Some(previous) = parent.and_then(|hash| by_hash.get(hash).copied()) {
                previous.get("depth").and_then(Value::as_i64) == Some(depth - 1)
                    && previous.get("nodeGuid") == record.get("nodeGuid")
                    && previous.get("workspaceGuid") == record.get("workspaceGuid")
            } else if let Some(parent) = parent {
                conn.query_row("SELECT count(*) FROM organization_node_versions WHERE version_hash=?1 AND depth=?2 AND node_guid=?3 AND workspace_guid=?4",params![parent,depth-1,node,workspace],|r|r.get::<_,i64>(0))?==1
            } else {
                false
            };
            if !valid {
                anyhow::bail!("organization node parent unavailable or mismatched");
            }
        }
        let validate = |event: &&Value| {
            let ty = event.get("type").and_then(Value::as_str);
            let type_ok = if depth == 0 {
                matches!(
                    ty,
                    Some("organization_node_create" | "organization_node_adopt")
                )
            } else if !active {
                ty == Some("organization_node_archive")
            } else {
                ty == Some("organization_node_update")
            };
            let proof = [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ];
            type_ok
                && event.get("workspaceGuid").and_then(Value::as_str) == Some(workspace)
                && event.get("actorGuid").and_then(Value::as_str) == Some(actor)
                && event.get("fromLabel").and_then(Value::as_str) == Some(node)
                && event.get("toLabel").and_then(Value::as_str) == Some(payload)
                && proof.iter().all(|key| {
                    event
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty())
                })
        };
        if let Some(event) = history.get(ledger_hash) {
            if !validate(event) {
                anyhow::bail!("organization node ledger evidence mismatch");
            }
        } else {
            let exists:i64=conn.query_row("SELECT count(*) FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1 AND w.guid=?2 AND u.guid=?3 AND h.from_label=?4 AND h.to_label=?5 AND h.type IN ('organization_node_create','organization_node_adopt','organization_node_update','organization_node_archive') AND h.request_device_id IS NOT NULL AND h.request_signature IS NOT NULL",params![ledger_hash,workspace,actor,node,payload],|r|r.get(0))?;
            if exists == 0 {
                anyhow::bail!("organization node ledger event unavailable");
            }
        }
    }
    let snapshots: HashMap<&str, &Value> = journal
        .get("organizationNodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|node| {
            node.get("guid")
                .and_then(Value::as_str)
                .map(|guid| (guid, node))
        })
        .collect();
    let mut winners: HashMap<&str, &Value> = HashMap::new();
    for record in records {
        let node = record["nodeGuid"].as_str().unwrap_or_default();
        let candidate = (
            record["depth"].as_i64().unwrap_or(-1),
            record["versionHash"].as_str().unwrap_or_default(),
        );
        if winners.get(node).is_none_or(|current| {
            candidate
                > (
                    current["depth"].as_i64().unwrap_or(-1),
                    current["versionHash"].as_str().unwrap_or_default(),
                )
        }) {
            winners.insert(node, record);
        }
    }
    let mut parents = HashMap::new();
    for (guid, winner) in &winners {
        let snapshot = snapshots
            .get(guid)
            .ok_or_else(|| anyhow::anyhow!("organization node snapshot unavailable"))?;
        let fields = &winner["fields"];
        for (snapshot_key, field_key) in [
            ("parentGuid", "parentGuid"),
            ("kind", "kind"),
            ("name", "name"),
            ("tabLabel", "tabLabel"),
            ("responsibleGuid", "responsibleGuid"),
            ("displayOrder", "displayOrder"),
            ("color", "color"),
            ("icon", "icon"),
        ] {
            if snapshot.get(snapshot_key).unwrap_or(&Value::Null)
                != fields.get(field_key).unwrap_or(&Value::Null)
            {
                anyhow::bail!("organization node snapshot mismatch for {field_key}");
            }
        }
        if snapshot.get("archived").and_then(Value::as_bool)
            != winner
                .get("active")
                .and_then(Value::as_bool)
                .map(|active| !active)
        {
            anyhow::bail!("organization node archive mismatch");
        }
        parents.insert(*guid, fields.get("parentGuid").and_then(Value::as_str));
    }
    for start in parents.keys() {
        let mut seen = HashSet::new();
        let mut cursor = Some(*start);
        while let Some(node) = cursor {
            if !seen.insert(node) {
                anyhow::bail!("organization node cycle");
            }
            cursor = parents.get(node).copied().flatten();
        }
    }
    Ok(())
}
fn verify_stored_organization_node_versions(conn: &Connection) -> anyhow::Result<usize> {
    let snapshot = export_journal(conn);
    verify_organization_node_versions(conn, &snapshot)?;
    Ok(snapshot["organizationNodeVersions"]
        .as_array()
        .map_or(0, Vec::len))
}

fn inventory_payload_hash(record: &Value) -> anyhow::Result<String> {
    let payload = json!({"domain":"everyday/inventory/v1","sessionGuid":record.get("sessionGuid"),"workspaceGuid":record.get("workspaceGuid"),"actorGuid":record.get("actorGuid"),"kind":record.get("kind"),"itemGuid":record.get("itemGuid").unwrap_or(&Value::Null),"fields":record.get("fields"),"createdAt":record.get("createdAt")});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload)?)
    ))
}
fn inventory_record_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/inventory-ledger/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}

fn trpc_request_input(envelope: &Value) -> anyhow::Result<&Value> {
    envelope
        .get("0")
        .and_then(|value| value.get("json"))
        .or_else(|| envelope.get("json"))
        .ok_or_else(|| anyhow::anyhow!("signed tRPC request input unavailable"))
}

fn verify_sale_offers(journal: &Value) -> anyhow::Result<()> {
    let history = journal
        .get("history")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for offer in journal
        .get("saleOffers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        if offer.get("recordHash").and_then(Value::as_str) != Some(&sale_offer_hash(offer)?) {
            anyhow::bail!("sale offer commitment mismatch");
        }
        let ledger_hash = offer["ledgerHash"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("sale offer ledger hash unavailable"))?;
        let event = history
            .iter()
            .find(|event| event.get("opId").and_then(Value::as_str) == Some(ledger_hash))
            .ok_or_else(|| anyhow::anyhow!("sale offer has no signed ledger event"))?;
        if event.get("eventVersion").and_then(Value::as_i64) != Some(3)
            || event.get("type").and_then(Value::as_str) != Some("transfer_send")
            || event.get("workspaceGuid") != offer.get("workspaceGuid")
            || event.get("itemGuid") != offer.get("itemGuid")
            || event.get("actorGuid") != offer.get("sellerGuid")
            || event.get("fromLabel") != offer.get("sellerGuid")
            || event.get("toLabel") != offer.get("buyerGuid")
            || event.get("requestPath").and_then(Value::as_str) != Some("/api/trpc/bit.offer")
        {
            anyhow::bail!("sale offer differs from signed ledger intent");
        }
        let body = event["requestBody"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("sale offer signed body unavailable"))?;
        if event.get("requestHash").and_then(Value::as_str)
            != Some(hex::encode(Sha256::digest(body.as_bytes())).as_str())
        {
            anyhow::bail!("sale offer request body hash mismatch");
        }
        let envelope: Value = serde_json::from_str(body)?;
        let input = trpc_request_input(&envelope)?;
        for field in ["offerGuid", "workspaceGuid", "itemGuid", "buyerGuid"] {
            if input.get(field) != offer.get(field) {
                anyhow::bail!("sale offer {field} differs from signed request");
            }
        }
        let signed_quantity = input.get("quantity").and_then(Value::as_f64);
        let portable_quantity = offer.get("quantity").and_then(Value::as_f64);
        let quantity_matches = match (signed_quantity, portable_quantity) {
            (None, None) => true,
            (Some(signed), Some(portable)) => {
                signed.is_finite() && signed > 0.0 && signed.to_bits() == portable.to_bits()
            }
            _ => false,
        };
        if input.get("bitAmount") != offer.get("bitAmount")
            || !quantity_matches
            || input.get("comment").unwrap_or(&Value::Null)
                != offer.get("comment").unwrap_or(&Value::Null)
        {
            anyhow::bail!("sale offer price or comment differs from signed request");
        }
        let status = offer
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        if !matches!(status, "pending" | "accepted" | "rejected") {
            anyhow::bail!("unsupported sale offer status");
        }
        if status != "pending" {
            let decision_hash = offer
                .get("decisionLedgerHash")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("decided sale offer has no buyer event"))?;
            let decision = history
                .iter()
                .find(|event| event.get("opId").and_then(Value::as_str) == Some(decision_hash))
                .ok_or_else(|| anyhow::anyhow!("sale offer buyer event unavailable"))?;
            let expected_type = if status == "accepted" {
                "transfer_receive"
            } else {
                "transfer_reject"
            };
            if decision.get("type").and_then(Value::as_str) != Some(expected_type)
                || decision.get("workspaceGuid") != offer.get("workspaceGuid")
                || decision.get("itemGuid") != offer.get("itemGuid")
                || decision.get("actorGuid") != offer.get("buyerGuid")
                || decision.get("fromLabel") != offer.get("sellerGuid")
                || decision.get("toLabel") != offer.get("buyerGuid")
                || decision.get("requestPath").and_then(Value::as_str)
                    != Some(if status == "accepted" {
                        "/api/trpc/bit.acceptSale"
                    } else {
                        "/api/trpc/bit.rejectSale"
                    })
            {
                anyhow::bail!("sale offer decision differs from buyer signature");
            }
        }
        if status == "accepted" {
            let transaction_guid = offer
                .get("bitTransactionGuid")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("accepted sale offer has no Bit transaction"))?;
            let transaction = journal
                .get("accounting")
                .and_then(|value| value.get("transactions"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|transaction| {
                    transaction.get("guid").and_then(Value::as_str) == Some(transaction_guid)
                })
                .ok_or_else(|| anyhow::anyhow!("accepted sale Bit transaction unavailable"))?;
            if transaction.get("kind").and_then(Value::as_str) != Some("sale")
                || transaction.get("workspaceGuid") != offer.get("workspaceGuid")
                || transaction.get("actorGuid") != offer.get("buyerGuid")
                || transaction.get("reference") != offer.get("itemGuid")
                || transaction.get("amount") != offer.get("bitAmount")
            {
                anyhow::bail!("accepted sale differs from portable accounting");
            }
        } else if offer
            .get("bitTransactionGuid")
            .is_some_and(|value| !value.is_null())
        {
            anyhow::bail!("non-accepted sale offer contains Bit transaction");
        }
    }
    Ok(())
}

fn verify_inventory_intent(record: &Value, event: &Value) -> anyhow::Result<()> {
    if event.get("eventVersion").and_then(Value::as_i64) != Some(3) {
        anyhow::bail!("inventory event has no signed request body")
    }
    let body = event
        .get("requestBody")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("inventory request body unavailable"))?;
    let hash = event
        .get("requestHash")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("inventory request hash unavailable"))?;
    if hex::encode(Sha256::digest(body.as_bytes())) != hash {
        anyhow::bail!("inventory request body hash mismatch")
    }
    let kind = record
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let procedure = kind.strip_prefix("adopt_").unwrap_or(kind);
    let expected_path = match procedure {
        "create" => "/api/trpc/inventory.create",
        "check" => "/api/trpc/inventory.checkItem",
        "complete" => "/api/trpc/inventory.complete",
        _ => anyhow::bail!("unsupported inventory intent"),
    };
    if event.get("requestPath").and_then(Value::as_str) != Some(expected_path) {
        anyhow::bail!("inventory request path mismatch")
    }
    let envelope: Value = serde_json::from_str(body)?;
    let input = trpc_request_input(&envelope)?;
    if procedure == "check" {
        let fields = &record["fields"];
        let requested_checked = input
            .get("checked")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if fields.get("checked").and_then(Value::as_bool) != Some(requested_checked)
            || fields.get("actualQty").and_then(Value::as_f64)
                != input.get("actualQty").and_then(Value::as_f64)
        {
            anyhow::bail!("inventory result differs from signed user intent")
        }
    }
    Ok(())
}

fn photo_commitment(photo: &Value) -> anyhow::Result<String> {
    let required = |field: &str| {
        photo
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("photo has no {field}"))
    };
    let guid = required("guid")?;
    let item = required("itemGuid")?;
    let url = required("url")?;
    let thumb = required("thumbUrl")?;
    let checksum = required("sha256")?;
    if url.strip_prefix("cas:") != Some(checksum) {
        anyhow::bail!("photo CAS hash mismatch")
    }
    let payload = json!({
        "domain":"everyday/item-photo/v1","guid":guid,"itemGuid":item,
        "url":url,"thumbUrl":thumb,"sha256":checksum,
        "isTitle":photo.get("isTitle").and_then(Value::as_bool).unwrap_or(false),
    });
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&payload)?)))
}

fn verify_photo_intent(photo: &Value, event: &Value) -> anyhow::Result<()> {
    let payload_hash = photo_commitment(photo)?;
    let guid = photo["guid"].as_str().unwrap_or_default();
    let item = photo["itemGuid"].as_str().unwrap_or_default();
    if event.get("eventVersion").and_then(Value::as_i64) != Some(3)
        || event.get("type").and_then(Value::as_str) != Some("photo_add")
        || event.get("itemGuid").and_then(Value::as_str) != Some(item)
        || event.get("fromLabel").and_then(Value::as_str) != Some(guid)
        || event.get("toLabel").and_then(Value::as_str) != Some(payload_hash.as_str())
        || event.get("requestPath").and_then(Value::as_str) != Some("/api/trpc/items.addPhoto")
    {
        anyhow::bail!("photo ledger evidence mismatch")
    }
    let body = event
        .get("requestBody")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("photo event has no signed intent"))?;
    if event.get("requestHash").and_then(Value::as_str)
        != Some(hex::encode(Sha256::digest(body.as_bytes())).as_str())
    {
        anyhow::bail!("photo request body hash mismatch")
    }
    let envelope: Value = serde_json::from_str(body)?;
    let input = trpc_request_input(&envelope)?;
    let requested_url = input.get("url").and_then(Value::as_str);
    let requested_thumb = input
        .get("thumbUrl")
        .and_then(Value::as_str)
        .or(requested_url);
    if input.get("itemGuid").and_then(Value::as_str) != Some(item)
        || input.get("photoGuid").and_then(Value::as_str) != Some(guid)
        || requested_url != photo.get("url").and_then(Value::as_str)
        || requested_thumb != photo.get("thumbUrl").and_then(Value::as_str)
        || input
            .get("isTitle")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            != photo
                .get("isTitle")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    {
        anyhow::bail!("photo differs from signed user intent")
    }
    Ok(())
}

fn verify_photo_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("photoMode").and_then(Value::as_str)
        != Some("intent-bound-with-explicit-legacy/v1")
    {
        anyhow::bail!("journal does not declare photo verification mode")
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    for photo in journal
        .get("photos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(ledger_hash) = photo.get("ledgerHash").and_then(Value::as_str) else {
            // Снимки, созданные до photo_add/v1, остаются явно legacy и не
            // получают несуществовавшую пользовательскую подпись задним числом.
            continue;
        };
        if let Some(event) = history.get(ledger_hash) {
            verify_photo_intent(photo, event)?;
            continue;
        }
        let stored: Option<Value> = conn
            .query_row(
                "SELECT h.event_version,i.guid,h.type,h.from_label,h.to_label,
                        h.request_hash,h.request_path,h.request_body
                 FROM history_entries h JOIN items i ON i.id=h.item_id WHERE h.hash=?1",
                [ledger_hash],
                |row| Ok(json!({
                    "eventVersion":row.get::<_,i64>(0)?,"itemGuid":row.get::<_,String>(1)?,
                    "type":row.get::<_,String>(2)?,"fromLabel":row.get::<_,Option<String>>(3)?,
                    "toLabel":row.get::<_,Option<String>>(4)?,"requestHash":row.get::<_,Option<String>>(5)?,
                    "requestPath":row.get::<_,Option<String>>(6)?,"requestBody":row.get::<_,Option<String>>(7)?,
                })),
            )
            .optional()?;
        verify_photo_intent(
            photo,
            stored
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("photo ledger event unavailable"))?,
        )?;
    }
    Ok(())
}

fn document_commitment(document: &Value) -> anyhow::Result<String> {
    let required = |field: &str| {
        document
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("document has no {field}"))
    };
    let guid = required("guid")?;
    let item = required("itemGuid")?;
    let author = required("authorGuid")?;
    let name = required("name")?;
    let url = required("url")?;
    let checksum = required("sha256")?;
    let access = required("accessLevel")?;
    if url.strip_prefix("cas:") != Some(checksum)
        || !matches!(access, "members" | "accounting" | "managers")
    {
        anyhow::bail!("document CAS or ACL mismatch")
    }
    let payload = json!({
        "domain":"everyday/item-document/v1","guid":guid,"itemGuid":item,
        "authorGuid":author,"name":name,"url":url,"mime":document.get("mime"),
        "sha256":checksum,"accessLevel":access,
    });
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&payload)?)))
}

fn verify_document_intent(document: &Value, event: &Value) -> anyhow::Result<()> {
    let payload_hash = document_commitment(document)?;
    let guid = document["guid"].as_str().unwrap_or_default();
    let item = document["itemGuid"].as_str().unwrap_or_default();
    let author = document["authorGuid"].as_str().unwrap_or_default();
    if event.get("eventVersion").and_then(Value::as_i64) != Some(3)
        || event.get("type").and_then(Value::as_str) != Some("document_add")
        || event.get("itemGuid").and_then(Value::as_str) != Some(item)
        || event.get("actorGuid").and_then(Value::as_str) != Some(author)
        || event.get("fromLabel").and_then(Value::as_str) != Some(guid)
        || event.get("toLabel").and_then(Value::as_str) != Some(payload_hash.as_str())
        || event.get("requestPath").and_then(Value::as_str) != Some("/api/trpc/items.addDocument")
    {
        anyhow::bail!("document ledger evidence mismatch")
    }
    let body = event
        .get("requestBody")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("document event has no signed intent"))?;
    let request_hash = hex::encode(Sha256::digest(body.as_bytes()));
    if event.get("requestHash").and_then(Value::as_str) != Some(request_hash.as_str()) {
        anyhow::bail!("document request body hash mismatch")
    }
    let envelope: Value = serde_json::from_str(body)?;
    let input = trpc_request_input(&envelope)?;
    for (input_field, record_field) in [
        ("itemGuid", "itemGuid"),
        ("documentGuid", "guid"),
        ("name", "name"),
        ("url", "url"),
        ("mime", "mime"),
        ("accessLevel", "accessLevel"),
    ] {
        let default_access = Value::String("members".into());
        let input_value = if input_field == "accessLevel" {
            input.get(input_field).unwrap_or(&default_access)
        } else {
            input.get(input_field).unwrap_or(&Value::Null)
        };
        if input_value != document.get(record_field).unwrap_or(&Value::Null) {
            anyhow::bail!("document differs from signed user intent")
        }
    }
    Ok(())
}

fn verify_document_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("documentMode").and_then(Value::as_str)
        != Some("intent-bound-with-explicit-legacy/v1")
    {
        anyhow::bail!("journal does not declare document verification mode")
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    for document in journal
        .get("documents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(ledger_hash) = document.get("ledgerHash").and_then(Value::as_str) else {
            continue;
        };
        if let Some(event) = history.get(ledger_hash) {
            verify_document_intent(document, event)?;
            continue;
        }
        let stored: Option<Value> = conn
            .query_row(
                "SELECT h.event_version,i.guid,u.guid,h.type,h.from_label,h.to_label,
                        h.request_hash,h.request_path,h.request_body
                 FROM history_entries h JOIN items i ON i.id=h.item_id
                 JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1",
                [ledger_hash],
                |row| Ok(json!({
                    "eventVersion":row.get::<_,i64>(0)?,"itemGuid":row.get::<_,String>(1)?,
                    "actorGuid":row.get::<_,String>(2)?,"type":row.get::<_,String>(3)?,
                    "fromLabel":row.get::<_,Option<String>>(4)?,"toLabel":row.get::<_,Option<String>>(5)?,
                    "requestHash":row.get::<_,Option<String>>(6)?,"requestPath":row.get::<_,Option<String>>(7)?,
                    "requestBody":row.get::<_,Option<String>>(8)?,
                })),
            )
            .optional()?;
        verify_document_intent(
            document,
            stored
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("document ledger event unavailable"))?,
        )?;
    }
    Ok(())
}

fn verify_knowledge_intent(revision: &Value, event: &Value) -> anyhow::Result<()> {
    let required = |field: &str| {
        revision
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("knowledge revision has no {field}"))
    };
    let revision_guid = required("guid")?;
    let page_guid = required("pageGuid")?;
    let workspace_guid = required("workspaceGuid")?;
    let author_guid = required("authorGuid")?;
    let revision_hash = required("revisionHash")?;
    if event.get("eventVersion").and_then(Value::as_i64) != Some(3)
        || event.get("type").and_then(Value::as_str) != Some("knowledge_revision")
        || event.get("actorGuid").and_then(Value::as_str) != Some(author_guid)
        || event.get("fromLabel").and_then(Value::as_str) != Some(page_guid)
        || event.get("toLabel").and_then(Value::as_str) != Some(revision_hash)
        || event.get("requestPath").and_then(Value::as_str) != Some("/api/trpc/knowledge.save")
    {
        anyhow::bail!("knowledge ledger evidence mismatch")
    }
    let body = event
        .get("requestBody")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("knowledge event has no signed intent"))?;
    let request_hash = hex::encode(Sha256::digest(body.as_bytes()));
    if event.get("requestHash").and_then(Value::as_str) != Some(request_hash.as_str()) {
        anyhow::bail!("knowledge request body hash mismatch")
    }
    let envelope: Value = serde_json::from_str(body)?;
    let input = trpc_request_input(&envelope)?;
    for (input_field, expected) in [
        ("workspaceGuid", Value::String(workspace_guid.into())),
        ("pageGuid", Value::String(page_guid.into())),
        ("revisionGuid", Value::String(revision_guid.into())),
        ("slug", revision.get("slug").cloned().unwrap_or(Value::Null)),
        (
            "title",
            revision.get("title").cloned().unwrap_or(Value::Null),
        ),
        (
            "content",
            revision.get("content").cloned().unwrap_or(Value::Null),
        ),
        (
            "visibility",
            revision
                .get("visibility")
                .cloned()
                .unwrap_or_else(|| json!("members")),
        ),
        (
            "parentRevisionGuid",
            revision.get("parentGuid").cloned().unwrap_or(Value::Null),
        ),
        (
            "attachments",
            revision
                .get("attachments")
                .cloned()
                .unwrap_or_else(|| json!([])),
        ),
    ] {
        let actual = match input_field {
            "visibility" => input
                .get(input_field)
                .cloned()
                .unwrap_or_else(|| json!("members")),
            "parentRevisionGuid" => input.get(input_field).cloned().unwrap_or(Value::Null),
            "attachments" => input.get(input_field).cloned().unwrap_or_else(|| json!([])),
            _ => input.get(input_field).cloned().unwrap_or(Value::Null),
        };
        if actual != expected {
            anyhow::bail!("knowledge revision differs from signed user intent")
        }
    }
    Ok(())
}

fn verify_knowledge_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    let knowledge = journal
        .get("knowledge")
        .ok_or_else(|| anyhow::anyhow!("journal has no knowledge section"))?;
    if knowledge.get("intentMode").and_then(Value::as_str)
        != Some("device-signed-with-explicit-legacy/v1")
    {
        anyhow::bail!("knowledge section does not declare intent verification mode")
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    for revision in knowledge
        .get("revisions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(ledger_hash) = revision.get("ledgerHash").and_then(Value::as_str) else {
            continue;
        };
        if let Some(event) = history.get(ledger_hash) {
            verify_knowledge_intent(revision, event)?;
            continue;
        }
        let stored: Option<Value> = conn
            .query_row(
                "SELECT h.event_version,u.guid,h.type,h.from_label,h.to_label,
                        h.request_hash,h.request_path,h.request_body
                 FROM history_entries h JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1",
                [ledger_hash],
                |row| Ok(json!({
                    "eventVersion":row.get::<_,i64>(0)?,"actorGuid":row.get::<_,String>(1)?,
                    "type":row.get::<_,String>(2)?,"fromLabel":row.get::<_,Option<String>>(3)?,
                    "toLabel":row.get::<_,Option<String>>(4)?,"requestHash":row.get::<_,Option<String>>(5)?,
                    "requestPath":row.get::<_,Option<String>>(6)?,"requestBody":row.get::<_,Option<String>>(7)?,
                })),
            )
            .optional()?;
        verify_knowledge_intent(
            revision,
            stored
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("knowledge ledger event unavailable"))?,
        )?;
    }
    Ok(())
}

fn verify_chat_intent(message: &Value, event: &Value) -> anyhow::Result<()> {
    let required = |field: &str| {
        message
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("chat message has no {field}"))
    };
    let guid = required("guid")?;
    let workspace_guid = required("workspaceGuid")?;
    let user_guid = required("userGuid")?;
    let text_value = message.get("text").and_then(Value::as_str).unwrap_or("");
    let attachments = message
        .get("attachments")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let commitment =
        ledger::chat_commitment(guid, workspace_guid, user_guid, text_value, &attachments);
    if event.get("eventVersion").and_then(Value::as_i64) != Some(3)
        || event.get("type").and_then(Value::as_str) != Some("chat_message")
        || event.get("actorGuid").and_then(Value::as_str) != Some(user_guid)
        || event.get("fromLabel").and_then(Value::as_str) != Some(guid)
        || event.get("toLabel").and_then(Value::as_str) != Some(commitment.as_str())
        || event.get("requestPath").and_then(Value::as_str) != Some("/api/trpc/chat.send")
    {
        anyhow::bail!("chat ledger evidence mismatch")
    }
    let body = event
        .get("requestBody")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("chat event has no signed intent"))?;
    let request_hash = hex::encode(Sha256::digest(body.as_bytes()));
    if event.get("requestHash").and_then(Value::as_str) != Some(request_hash.as_str()) {
        anyhow::bail!("chat request body hash mismatch")
    }
    let envelope: Value = serde_json::from_str(body)?;
    let input = trpc_request_input(&envelope)?;
    let mut input_attachments = input
        .get("attachments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for attachment in &mut input_attachments {
        let url = attachment
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("chat attachment has no CAS URL"))?;
        let checksum = url
            .strip_prefix("cas:")
            .filter(|hash| hash.len() == 64 && hash.chars().all(|value| value.is_ascii_hexdigit()))
            .ok_or_else(|| anyhow::anyhow!("chat attachment is not CAS-backed"))?;
        attachment["sha256"] = json!(checksum);
    }
    for (field, expected) in [
        ("workspaceGuid", Value::String(workspace_guid.into())),
        ("messageGuid", Value::String(guid.into())),
        ("text", Value::String(text_value.into())),
    ] {
        let actual = input.get(field).cloned().unwrap_or(Value::Null);
        if actual != expected {
            anyhow::bail!("chat message differs from signed user intent")
        }
    }
    if Value::Array(input_attachments) != attachments {
        anyhow::bail!("chat attachments differ from signed user intent")
    }
    Ok(())
}

fn verify_chat_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("chatMode").and_then(Value::as_str)
        != Some("device-signed-with-explicit-legacy/v1")
    {
        anyhow::bail!("journal does not declare chat verification mode")
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    for message in journal
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ledger_hash = message
            .get("ledgerHash")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("chat message has no ledgerHash"))?;
        let stored;
        let event = if let Some(event) = history.get(ledger_hash) {
            *event
        } else {
            stored = conn
                .query_row(
                    "SELECT h.event_version,u.guid,h.type,h.from_label,h.to_label,
                            h.request_hash,h.request_path,h.request_body
                     FROM history_entries h JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1",
                    [ledger_hash],
                    |row| Ok(json!({
                        "eventVersion":row.get::<_,i64>(0)?,"actorGuid":row.get::<_,String>(1)?,
                        "type":row.get::<_,String>(2)?,"fromLabel":row.get::<_,Option<String>>(3)?,
                        "toLabel":row.get::<_,Option<String>>(4)?,"requestHash":row.get::<_,Option<String>>(5)?,
                        "requestPath":row.get::<_,Option<String>>(6)?,"requestBody":row.get::<_,Option<String>>(7)?,
                    })),
                )
                .optional()?
                .ok_or_else(|| anyhow::anyhow!("chat ledger event unavailable"))?;
            &stored
        };
        if event.get("eventVersion").and_then(Value::as_i64) == Some(3) {
            verify_chat_intent(message, event)?;
        }
    }
    Ok(())
}

fn verify_inventory_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("inventoryMode").and_then(Value::as_str) != Some("append-only-records/v1") {
        anyhow::bail!("journal does not provide portable inventory")
    }
    let records = journal
        .get("inventoryRecords")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no inventory records"))?;
    let unique: HashSet<&str> = records
        .iter()
        .filter_map(|r| r.get("recordHash").and_then(Value::as_str))
        .collect();
    if unique.len() != records.len() {
        anyhow::bail!("duplicate inventory record")
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("opId").and_then(Value::as_str).map(|h| (h, e)))
        .collect();
    let incoming_creates: HashSet<&str> = records
        .iter()
        .filter(|r| {
            matches!(
                r.get("kind").and_then(Value::as_str),
                Some("create" | "adopt_check" | "adopt_complete")
            )
        })
        .filter_map(|r| r.get("sessionGuid").and_then(Value::as_str))
        .collect();
    let incoming_create_count = records
        .iter()
        .filter(|r| {
            matches!(
                r.get("kind").and_then(Value::as_str),
                Some("create" | "adopt_check" | "adopt_complete")
            )
        })
        .count();
    if incoming_creates.len() != incoming_create_count {
        anyhow::bail!("duplicate inventory session root")
    }
    for record in records {
        let get = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow::anyhow!("inventory record has no {key}"))
        };
        let hash = get("recordHash")?;
        let session = get("sessionGuid")?;
        let ws = get("workspaceGuid")?;
        let actor = get("actorGuid")?;
        let kind = get("kind")?;
        let payload = get("payloadHash")?;
        let ledger_hash = get("ledgerHash")?;
        if !matches!(
            kind,
            "create" | "check" | "complete" | "adopt_check" | "adopt_complete"
        ) || !record.get("fields").is_some_and(Value::is_object)
        {
            anyhow::bail!("invalid inventory record")
        }
        let fields = &record["fields"];
        match kind {
            "create" | "adopt_check" | "adopt_complete" => {
                if fields
                    .get("number")
                    .and_then(Value::as_str)
                    .is_none_or(|v| v.is_empty() || v.chars().count() > 80)
                {
                    anyhow::bail!("invalid inventory number")
                }
                let results = fields
                    .get("results")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow::anyhow!("inventory root has no results"))?;
                if results.len() > 100_000
                    || results.iter().any(|result| {
                        result.get("itemGuid").and_then(Value::as_str).is_none()
                            || result
                                .get("expectedQty")
                                .and_then(Value::as_f64)
                                .is_none_or(|v| !v.is_finite() || v < 0.0)
                    })
                {
                    anyhow::bail!("invalid inventory root results")
                }
            }
            "check" => {
                if fields.get("checked").and_then(Value::as_bool).is_none()
                    || fields
                        .get("actualQty")
                        .and_then(Value::as_f64)
                        .is_some_and(|v| !v.is_finite() || v < 0.0)
                {
                    anyhow::bail!("invalid inventory check")
                }
            }
            "complete" => {
                if fields
                    .get("totalItems")
                    .and_then(Value::as_i64)
                    .is_none_or(|v| v < 0)
                    || fields
                        .get("checkedItems")
                        .and_then(Value::as_i64)
                        .is_none_or(|v| v < 0)
                {
                    anyhow::bail!("invalid inventory completion")
                }
            }
            _ => unreachable!(),
        }
        if matches!(kind, "check" | "adopt_check")
            && record.get("itemGuid").and_then(Value::as_str).is_none()
        {
            anyhow::bail!("inventory check has no item")
        }
        if !matches!(kind, "create" | "adopt_check" | "adopt_complete")
            && !incoming_creates.contains(session)
        {
            let exists: i64 = conn.query_row(
                "SELECT count(*) FROM inventory_records WHERE session_guid=?1 AND kind IN ('create','adopt_check','adopt_complete')",
                [session],
                |r| r.get(0),
            )?;
            if exists == 0 {
                anyhow::bail!("inventory session root unavailable")
            }
        }
        if inventory_payload_hash(record)? != payload
            || inventory_record_hash(payload, ledger_hash) != hash
        {
            anyhow::bail!("inventory record hash mismatch")
        }
        let expected_type = format!("inventory_{kind}");
        let validate = |event: &&Value| {
            event.get("type").and_then(Value::as_str) == Some(expected_type.as_str())
                && event.get("workspaceGuid").and_then(Value::as_str) == Some(ws)
                && event.get("actorGuid").and_then(Value::as_str) == Some(actor)
                && event.get("fromLabel").and_then(Value::as_str) == Some(session)
                && event.get("toLabel").and_then(Value::as_str) == Some(payload)
                && event.get("itemGuid").unwrap_or(&Value::Null)
                    == record.get("itemGuid").unwrap_or(&Value::Null)
                && [
                    "requestDeviceId",
                    "requestPublicKey",
                    "requestNonce",
                    "requestSignature",
                    "requestHash",
                    "requestTimestamp",
                    "requestPath",
                ]
                .iter()
                .all(|key| {
                    event
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty())
                })
        };
        if let Some(event) = history.get(ledger_hash) {
            if !validate(event) {
                anyhow::bail!("inventory ledger evidence mismatch")
            }
            verify_inventory_intent(record, event)?;
        } else {
            let stored:Option<Value>=conn.query_row("SELECT h.event_version,h.request_body,h.request_hash,h.request_path FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1 AND w.guid=?2 AND u.guid=?3 AND h.type=?4 AND h.from_label=?5 AND h.to_label=?6 AND h.request_device_id IS NOT NULL AND h.request_signature IS NOT NULL",params![ledger_hash,ws,actor,expected_type,session,payload],|r|Ok(json!({"eventVersion":r.get::<_,i64>(0)?,"requestBody":r.get::<_,Option<String>>(1)?,"requestHash":r.get::<_,Option<String>>(2)?,"requestPath":r.get::<_,Option<String>>(3)?}))).optional()?;
            let stored =
                stored.ok_or_else(|| anyhow::anyhow!("inventory ledger event unavailable"))?;
            verify_inventory_intent(record, &stored)?;
        }
    }
    Ok(())
}
fn verify_stored_inventory_records(conn: &Connection) -> anyhow::Result<usize> {
    let journal = export_journal(conn);
    verify_inventory_records(conn, &journal)?;
    Ok(journal["inventoryRecords"].as_array().map_or(0, Vec::len))
}

fn rebuild_inventory_state(conn: &Connection) -> anyhow::Result<()> {
    let mut sessions = conn
        .prepare("SELECT DISTINCT session_guid FROM inventory_records")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    sessions.sort();
    for session_guid in sessions {
        let root:Option<(String,String,String,String)>=conn.query_row("SELECT workspace_guid,actor_guid,fields_json,created_at FROM inventory_records WHERE session_guid=?1 AND kind IN ('create','adopt_check','adopt_complete') ORDER BY created_at,record_hash LIMIT 1",[&session_guid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((ws_guid, actor_guid, raw, created_at)) = root else {
            continue;
        };
        let Some(ws) = id_by_guid(conn, "workspaces", &ws_guid) else {
            continue;
        };
        let Some(actor) = id_by_guid(conn, "users", &actor_guid) else {
            continue;
        };
        let fields: Value = serde_json::from_str(&raw)?;
        let number = fields
            .get("number")
            .and_then(Value::as_str)
            .unwrap_or("ИНВ-OFFLINE");
        let scope_type = fields
            .get("scopeType")
            .and_then(Value::as_str)
            .unwrap_or("all");
        let scope_ref_id = fields
            .get("scopeRefGuid")
            .and_then(Value::as_str)
            .and_then(|guid| match scope_type {
                "storage" => id_by_guid(conn, "storages", guid),
                "site" => id_by_guid(conn, "building_sites", guid),
                _ => None,
            });
        let block_transfers = fields
            .get("blockTransfers")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        conn.execute("INSERT OR IGNORE INTO inventory_sessions(guid,number,workspace_id,status,started_by,created_at) VALUES(?1,?2,?3,'in_progress',?4,?5)",params![session_guid,number,ws,actor,created_at])?;
        conn.execute(
            "UPDATE inventory_sessions SET number=?1,workspace_id=?2,started_by=?3,scope_type=?4,scope_ref_id=?5,block_transfers=?6 WHERE guid=?7",
            params![number,ws,actor,scope_type,scope_ref_id,block_transfers,session_guid],
        )?;
        let sid = id_by_guid(conn, "inventory_sessions", &session_guid)
            .ok_or_else(|| anyhow::anyhow!("inventory session unavailable"))?;
        conn.execute("DELETE FROM inventory_results WHERE session_id=?1", [sid])?;
        if let Some(results) = fields.get("results").and_then(Value::as_array) {
            for result in results {
                if let Some(item) = result
                    .get("itemGuid")
                    .and_then(Value::as_str)
                    .and_then(|g| id_by_guid(conn, "items", g))
                {
                    conn.execute("INSERT OR IGNORE INTO inventory_results(session_id,item_id,expected_qty,actual_qty,checked) VALUES(?1,?2,?3,?4,?5)",params![sid,item,result.get("expectedQty").and_then(Value::as_f64),result.get("actualQty").and_then(Value::as_f64),result.get("checked").and_then(Value::as_bool).unwrap_or(false) as i64])?;
                }
            }
        }
        let mut checks=conn.prepare("SELECT item_guid,fields_json FROM inventory_records WHERE session_guid=?1 AND kind='check' ORDER BY created_at,record_hash")?.query_map([&session_guid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
        for (item_guid, raw) in checks.drain(..) {
            if let Some(item) = id_by_guid(conn, "items", &item_guid) {
                let fields: Value = serde_json::from_str(&raw)?;
                conn.execute("INSERT INTO inventory_results(session_id,item_id,expected_qty,actual_qty,checked) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(session_id,item_id) DO UPDATE SET actual_qty=excluded.actual_qty,checked=excluded.checked",params![sid,item,fields.get("expectedQty").and_then(Value::as_f64),fields.get("actualQty").and_then(Value::as_f64),fields.get("checked").and_then(Value::as_bool).unwrap_or(false) as i64])?;
            }
        }
        let completion:Option<String>=conn.query_row("SELECT created_at FROM inventory_records WHERE session_guid=?1 AND kind IN ('complete','adopt_complete') ORDER BY created_at DESC,record_hash DESC LIMIT 1",[&session_guid],|r|r.get(0)).optional()?;
        conn.execute(
            "UPDATE inventory_sessions SET status=?1,completed_at=?2 WHERE id=?3",
            params![
                if completion.is_some() {
                    "completed"
                } else {
                    "in_progress"
                },
                completion,
                sid
            ],
        )?;
    }
    Ok(())
}

fn verify_config_versions(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("configMode").and_then(Value::as_str) != Some("portable-branches/v1") {
        anyhow::bail!("journal does not provide portable config");
    }
    let records = journal
        .get("configVersions")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no config version array"))?;
    let by_hash: HashMap<&str, &Value> = records
        .iter()
        .filter_map(|r| r.get("versionHash").and_then(Value::as_str).map(|h| (h, r)))
        .collect();
    if by_hash.len() != records.len() {
        anyhow::bail!("duplicate config version");
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("opId").and_then(Value::as_str).map(|h| (h, e)))
        .collect();
    for record in records {
        let get = |name: &str| {
            record
                .get(name)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow::anyhow!("config version has no {name}"))
        };
        let hash = get("versionHash")?;
        let ledger_hash = get("ledgerHash")?;
        let payload_hash = get("payloadHash")?;
        let kind = get("kind")?;
        let entity = get("entityGuid")?;
        let workspace = get("workspaceGuid")?;
        let actor = get("actorGuid")?;
        if !matches!(kind, "storage" | "site" | "category" | "brand" | "status")
            || record["fields"]
                .get("name")
                .and_then(Value::as_str)
                .is_none_or(|name| name.is_empty() || name.chars().count() > 120)
        {
            anyhow::bail!("invalid config fields");
        }
        let depth = record
            .get("depth")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("config depth missing"))?;
        let active = record
            .get("active")
            .and_then(Value::as_bool)
            .ok_or_else(|| anyhow::anyhow!("config active missing"))?;
        if config_payload_hash(record)? != payload_hash
            || config_version_hash(payload_hash, ledger_hash) != hash
        {
            anyhow::bail!("config version hash mismatch");
        }
        let parent = record.get("parentHash").and_then(Value::as_str);
        if depth == 0 {
            if parent.is_some() {
                anyhow::bail!("config root has parent");
            }
        } else {
            let valid = if let Some(p) = parent.and_then(|h| by_hash.get(h).copied()) {
                p.get("depth").and_then(Value::as_i64) == Some(depth - 1)
                    && p.get("kind") == record.get("kind")
                    && p.get("entityGuid") == record.get("entityGuid")
                    && p.get("workspaceGuid") == record.get("workspaceGuid")
            } else if let Some(parent) = parent {
                conn.query_row("SELECT count(*) FROM config_versions WHERE version_hash=?1 AND depth=?2 AND kind=?3 AND entity_guid=?4 AND workspace_guid=?5",params![parent,depth-1,kind,entity,workspace],|r|r.get::<_,i64>(0))?==1
            } else {
                false
            };
            if !valid {
                anyhow::bail!("config parent unavailable or mismatched");
            }
        }
        let expected = if depth > 0 && !active {
            "config_archive"
        } else if depth > 0 {
            "config_update"
        } else {
            ""
        };
        let validate_event = |event: &&Value| {
            let ty = event.get("type").and_then(Value::as_str);
            let type_ok = if depth == 0 {
                matches!(ty, Some("config_create" | "config_adopt"))
            } else {
                ty == Some(expected)
            };
            let proof = [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ];
            type_ok
                && event.get("workspaceGuid").and_then(Value::as_str) == Some(workspace)
                && event.get("actorGuid").and_then(Value::as_str) == Some(actor)
                && event.get("fromLabel").and_then(Value::as_str) == Some(entity)
                && event.get("toLabel").and_then(Value::as_str) == Some(payload_hash)
                && proof.iter().all(|name| {
                    event
                        .get(*name)
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty())
                })
        };
        if let Some(event) = history.get(ledger_hash) {
            if !validate_event(event) {
                anyhow::bail!("config ledger evidence mismatch");
            }
        } else {
            let exists:i64=conn.query_row("SELECT count(*) FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1 AND w.guid=?2 AND u.guid=?3 AND h.from_label=?4 AND h.to_label=?5 AND h.type IN ('config_create','config_adopt','config_update','config_archive') AND h.request_device_id IS NOT NULL AND h.request_signature IS NOT NULL",params![ledger_hash,workspace,actor,entity,payload_hash],|r|r.get(0))?;
            if exists == 0 {
                anyhow::bail!("config ledger event unavailable");
            }
        }
    }
    Ok(())
}
fn verify_stored_config_versions(conn: &Connection) -> anyhow::Result<usize> {
    let snapshot = export_journal(conn);
    verify_config_versions(conn, &snapshot)?;
    Ok(snapshot["configVersions"].as_array().map_or(0, Vec::len))
}

fn verify_item_state_versions(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("itemStateMode").and_then(Value::as_str) != Some("portable-branches/v1") {
        anyhow::bail!("journal does not provide portable item state");
    }
    let records = journal
        .get("itemStateVersions")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no item state array"))?;
    let by_hash: HashMap<&str, &Value> = records
        .iter()
        .filter_map(|record| {
            record
                .get("versionHash")
                .and_then(Value::as_str)
                .map(|hash| (hash, record))
        })
        .collect();
    if by_hash.len() != records.len() {
        anyhow::bail!("duplicate item state version");
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    for record in records {
        let get = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("item state has no {key}"))
        };
        let version_hash = get("versionHash")?;
        let item = get("itemGuid")?;
        let workspace = get("workspaceGuid")?;
        let actor = get("actorGuid")?;
        let payload_hash = get("payloadHash")?;
        let ledger_hash = get("ledgerHash")?;
        let depth = record
            .get("depth")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("item state depth missing"))?;
        let fields = record
            .get("fields")
            .ok_or_else(|| anyhow::anyhow!("item state fields missing"))?;
        if fields
            .get("title")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            || fields
                .get("internalId")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            anyhow::bail!("invalid item state fields");
        }
        if item_state_payload_hash(record)? != payload_hash
            || item_state_version_hash(payload_hash, ledger_hash) != version_hash
        {
            anyhow::bail!("item state hash mismatch");
        }
        let parent = record.get("parentHash").and_then(Value::as_str);
        if depth == 0 {
            if parent.is_some() {
                anyhow::bail!("item state root has parent");
            }
        } else {
            let valid = if let Some(p) = parent.and_then(|hash| by_hash.get(hash).copied()) {
                p.get("depth").and_then(Value::as_i64) == Some(depth - 1)
                    && p.get("itemGuid") == record.get("itemGuid")
                    && p.get("workspaceGuid") == record.get("workspaceGuid")
            } else if let Some(parent) = parent {
                conn.query_row("SELECT count(*) FROM item_state_versions WHERE version_hash=?1 AND depth=?2 AND item_guid=?3 AND workspace_guid=?4",params![parent,depth-1,item,workspace],|r|r.get::<_,i64>(0))?==1
            } else {
                false
            };
            if !valid {
                anyhow::bail!("item state parent unavailable or mismatched");
            }
        }
        let validate = |event: &&Value| {
            let ty = event.get("type").and_then(Value::as_str);
            let type_ok = if depth == 0 {
                matches!(ty, Some("item_state_create" | "item_state_adopt"))
            } else {
                ty == Some("item_state_update")
            };
            let proof = [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ];
            type_ok
                && event.get("workspaceGuid").and_then(Value::as_str) == Some(workspace)
                && event.get("itemGuid").and_then(Value::as_str) == Some(item)
                && event.get("actorGuid").and_then(Value::as_str) == Some(actor)
                && event.get("fromLabel").and_then(Value::as_str) == Some(item)
                && event.get("toLabel").and_then(Value::as_str) == Some(payload_hash)
                && proof.iter().all(|key| {
                    event
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty())
                })
        };
        if let Some(event) = history.get(ledger_hash) {
            if !validate(event) {
                anyhow::bail!("item state ledger evidence mismatch");
            }
        } else {
            let exists:i64=conn.query_row("SELECT count(*) FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN items i ON i.id=h.item_id JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1 AND w.guid=?2 AND i.guid=?3 AND u.guid=?4 AND h.from_label=?3 AND h.to_label=?5 AND h.type IN ('item_state_create','item_state_adopt','item_state_update') AND h.request_device_id IS NOT NULL AND h.request_signature IS NOT NULL",params![ledger_hash,workspace,item,actor,payload_hash],|r|r.get(0))?;
            if exists == 0 {
                anyhow::bail!("item state ledger event unavailable");
            }
        }
    }
    let snapshots: HashMap<&str, &Value> = journal
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.get("guid")
                .and_then(Value::as_str)
                .map(|guid| (guid, item))
        })
        .collect();
    let mut winners: HashMap<&str, &Value> = HashMap::new();
    for record in records {
        let item = record["itemGuid"].as_str().unwrap_or_default();
        let candidate = (
            record["depth"].as_i64().unwrap_or(-1),
            record["versionHash"].as_str().unwrap_or_default(),
        );
        let replace = winners.get(item).is_none_or(|current| {
            candidate
                > (
                    current["depth"].as_i64().unwrap_or(-1),
                    current["versionHash"].as_str().unwrap_or_default(),
                )
        });
        if replace {
            winners.insert(item, record);
        }
    }
    let master_keys = [
        "internalId",
        "title",
        "categoryGuid",
        "brandGuid",
        "serialNumber",
        "qrCode",
        "calibratedUntil",
        "minQuantity",
        "quantitative",
        "unit",
        "cost",
        "comment",
        "sourceSystem",
        "externalId",
        "metadata",
        "organizationNodeGuid",
    ];
    let mut change_winners: HashMap<&str, &Value> = HashMap::new();
    for record in journal
        .get("changeRequests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let request = record
            .get("requestGuid")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let candidate = (
            record.get("depth").and_then(Value::as_i64).unwrap_or(-1),
            record
                .get("recordHash")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        if change_winners.get(request).is_none_or(|current| {
            candidate
                > (
                    current.get("depth").and_then(Value::as_i64).unwrap_or(-1),
                    current
                        .get("recordHash")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
        }) {
            change_winners.insert(request, record);
        }
    }
    let mut config_refs: HashMap<(&str, &str), &str> = HashMap::new();
    for record in journal
        .get("configVersions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if record.get("active").and_then(Value::as_bool) == Some(true) {
            if let (Some(kind), Some(guid), Some(name)) = (
                record.get("kind").and_then(Value::as_str),
                record.get("entityGuid").and_then(Value::as_str),
                record
                    .get("fields")
                    .and_then(|fields| fields.get("name"))
                    .and_then(Value::as_str),
            ) {
                config_refs.insert((kind, name), guid);
            }
        }
    }
    for (guid, winner) in winners {
        let snapshot = snapshots
            .get(guid)
            .ok_or_else(|| anyhow::anyhow!("item state snapshot unavailable"))?;
        let updated = winner
            .get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut expected = winner["fields"].clone();
        let mut decisions: Vec<&Value> = change_winners
            .values()
            .copied()
            .filter(|record| {
                record.get("itemGuid").and_then(Value::as_str) == Some(guid)
                    && record.get("depth").and_then(Value::as_i64).unwrap_or(0) > 0
                    && record
                        .get("createdAt")
                        .and_then(Value::as_str)
                        .is_some_and(|created| created >= updated)
            })
            .collect();
        decisions.sort_by_key(|record| {
            (
                record
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                record
                    .get("recordHash")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
        });
        for decision in decisions {
            let source = if decision.get("status").and_then(Value::as_str) == Some("accepted") {
                decision.get("patch")
            } else {
                decision.get("before")
            };
            for (key, value) in source.and_then(Value::as_object).into_iter().flatten() {
                let (target, value) = match key.as_str() {
                    "categoryId" => (
                        "categoryGuid",
                        value
                            .get("name")
                            .and_then(Value::as_str)
                            .and_then(|name| config_refs.get(&("category", name)).copied())
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    ),
                    "brandId" => (
                        "brandGuid",
                        value
                            .get("name")
                            .and_then(Value::as_str)
                            .and_then(|name| config_refs.get(&("brand", name)).copied())
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    ),
                    "statusId" => (
                        "statusSlug",
                        value.get("slug").cloned().unwrap_or(Value::Null),
                    ),
                    "responsibleUserId" => (
                        "responsibleGuid",
                        value.get("guid").cloned().unwrap_or(Value::Null),
                    ),
                    "buildingSiteId" => (
                        "buildingSiteGuid",
                        value
                            .get("name")
                            .and_then(Value::as_str)
                            .and_then(|name| config_refs.get(&("site", name)).copied())
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    ),
                    "storageId" => (
                        "storageGuid",
                        value
                            .get("name")
                            .and_then(Value::as_str)
                            .and_then(|name| config_refs.get(&("storage", name)).copied())
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    ),
                    other => (other, value.clone()),
                };
                expected[target] = value;
            }
        }
        let later_operational = history.values().any(|event| {
            event.get("itemGuid").and_then(Value::as_str) == Some(guid)
                && event
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .is_some_and(|created| created >= updated)
                && matches!(
                    event.get("type").and_then(Value::as_str),
                    Some(
                        "take"
                            | "return"
                            | "move"
                            | "inventory"
                            | "replenish"
                            | "write_off"
                            | "transfer_send"
                            | "transfer_receive"
                            | "fault_report"
                            | "fault_update"
                    )
                )
        });
        let has_custody = journal
            .get("custody")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|entry| entry.get("itemGuid").and_then(Value::as_str) == Some(guid));
        let mut operational_keys = vec![
            "statusSlug",
            "responsibleGuid",
            "buildingSiteGuid",
            "storageGuid",
        ];
        if !has_custody {
            operational_keys.push("quantity");
        }
        for key in master_keys.into_iter().chain(
            (!later_operational)
                .then_some(operational_keys)
                .into_iter()
                .flatten(),
        ) {
            if snapshot.get(key).unwrap_or(&Value::Null)
                != expected.get(key).unwrap_or(&Value::Null)
            {
                anyhow::bail!(
                    "item state snapshot mismatch for {key}: snapshot={} expected={}",
                    snapshot.get(key).unwrap_or(&Value::Null),
                    expected.get(key).unwrap_or(&Value::Null)
                );
            }
        }
    }
    Ok(())
}
fn verify_stored_item_state_versions(conn: &Connection) -> anyhow::Result<usize> {
    let snapshot = export_journal(conn);
    verify_item_state_versions(conn, &snapshot)?;
    Ok(snapshot["itemStateVersions"].as_array().map_or(0, Vec::len))
}

fn verify_fault_records(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("faultMode").and_then(Value::as_str) != Some("append-only-branches/v1") {
        anyhow::bail!("journal does not provide fault branches");
    }
    let records = journal
        .get("faults")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no fault array"))?;
    let by_hash: HashMap<&str, &Value> = records
        .iter()
        .filter_map(|r| r.get("recordHash").and_then(Value::as_str).map(|h| (h, r)))
        .collect();
    if by_hash.len() != records.len() {
        anyhow::bail!("duplicate or missing fault record hash");
    }
    let history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("opId").and_then(Value::as_str).map(|h| (h, e)))
        .collect();
    for record in records {
        let get = |name: &str| {
            record
                .get(name)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow::anyhow!("fault record has no {name}"))
        };
        let record_hash = get("recordHash")?;
        let ledger_hash = get("ledgerHash")?;
        let payload_hash = get("payloadHash")?;
        let fault = get("faultGuid")?;
        let workspace = get("workspaceGuid")?;
        let item = get("itemGuid")?;
        let actor = get("actorGuid")?;
        let depth = record
            .get("depth")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("fault depth missing"))?;
        if get("description")?.chars().count() > 8_000
            || !matches!(get("severity")?, "low" | "medium" | "high")
            || !matches!(get("status")?, "open" | "repair" | "resolved")
        {
            anyhow::bail!("invalid fault fields");
        }
        if fault_payload_hash(record)? != payload_hash
            || fault_record_hash(payload_hash, ledger_hash) != record_hash
        {
            anyhow::bail!("fault record hash mismatch");
        }
        let parent = record.get("parentHash").and_then(Value::as_str);
        if depth == 0 {
            if parent.is_some() {
                anyhow::bail!("invalid fault root");
            }
        } else {
            let parent_record = parent
                .and_then(|h| by_hash.get(h).copied())
                .ok_or_else(|| anyhow::anyhow!("fault parent unavailable"))?;
            if parent_record.get("depth").and_then(Value::as_i64) != Some(depth - 1) {
                anyhow::bail!("fault depth does not follow parent");
            }
            for field in [
                "faultGuid",
                "workspaceGuid",
                "itemGuid",
                "reporterGuid",
                "severity",
                "description",
                "photoUrl",
            ] {
                if parent_record.get(field) != record.get(field) {
                    anyhow::bail!("fault immutable field changed");
                }
            }
        }
        let event_type = if depth == 0 && get("reporterGuid")? == actor && get("status")? == "open"
        {
            "fault_report"
        } else if depth == 0 {
            "fault_adopt"
        } else {
            "fault_update"
        };
        if let Some(event) = history.get(ledger_hash) {
            let proof = [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ];
            if event.get("type").and_then(Value::as_str) != Some(event_type)
                || event.get("workspaceGuid").and_then(Value::as_str) != Some(workspace)
                || event.get("itemGuid").and_then(Value::as_str) != Some(item)
                || event.get("actorGuid").and_then(Value::as_str) != Some(actor)
                || event.get("fromLabel").and_then(Value::as_str) != Some(fault)
                || event.get("toLabel").and_then(Value::as_str) != Some(payload_hash)
                || !proof.iter().all(|name| {
                    event
                        .get(*name)
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty())
                })
            {
                anyhow::bail!("fault ledger evidence mismatch");
            }
        } else {
            let exists:i64=conn.query_row("SELECT count(*) FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN items i ON i.id=h.item_id JOIN users u ON u.id=h.actor_user_id WHERE h.hash=?1 AND h.type=?2 AND w.guid=?3 AND i.guid=?4 AND u.guid=?5 AND h.from_label=?6 AND h.to_label=?7 AND h.request_device_id IS NOT NULL AND h.request_signature IS NOT NULL",params![ledger_hash,event_type,workspace,item,actor,fault,payload_hash],|r|r.get(0))?;
            if exists == 0 {
                anyhow::bail!("fault ledger event unavailable");
            }
        }
    }
    Ok(())
}

fn verify_stored_fault_records(conn: &Connection) -> anyhow::Result<usize> {
    let snapshot = export_journal(conn);
    verify_fault_records(conn, &snapshot)?;
    Ok(snapshot
        .get("faults")
        .and_then(Value::as_array)
        .map_or(0, Vec::len))
}

fn verify_item_comments(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("itemCommentMode").and_then(Value::as_str) != Some("ledger-records/v1") {
        anyhow::bail!("journal does not provide ledger-bound item comments");
    }
    let incoming_history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|h| (h, event))
        })
        .collect();
    let mut seen = HashSet::new();
    for record in journal
        .get("itemComments")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no item comment array"))?
    {
        let get = |name: &str| {
            record
                .get(name)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow::anyhow!("item comment has no {name}"))
        };
        let guid = get("guid")?;
        let workspace = get("workspaceGuid")?;
        let item = get("itemGuid")?;
        let actor = get("authorGuid")?;
        let text = get("text")?;
        let ledger_hash = get("ledgerHash")?;
        let payload_hash = get("payloadHash")?;
        let record_hash = get("recordHash")?;
        if text.chars().count() > 8_000 || !seen.insert(guid) {
            anyhow::bail!("invalid or duplicate item comment");
        }
        if item_comment_payload_hash(record)? != payload_hash
            || item_comment_record_hash(payload_hash, ledger_hash) != record_hash
        {
            anyhow::bail!("item comment hash mismatch");
        }
        if let Some(event) = incoming_history.get(ledger_hash) {
            let proof = [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ];
            let valid = event.get("type").and_then(Value::as_str) == Some("item_comment")
                && event.get("workspaceGuid").and_then(Value::as_str) == Some(workspace)
                && event.get("itemGuid").and_then(Value::as_str) == Some(item)
                && event.get("actorGuid").and_then(Value::as_str) == Some(actor)
                && event.get("fromLabel").and_then(Value::as_str) == Some(guid)
                && event.get("toLabel").and_then(Value::as_str) == Some(payload_hash)
                && event.get("comment").and_then(Value::as_str) == Some(text)
                && proof.iter().all(|name| {
                    event
                        .get(*name)
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty())
                });
            if !valid {
                anyhow::bail!("item comment ledger evidence mismatch");
            }
        } else {
            let exists:i64=conn.query_row(
                "SELECT count(*) FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN users u ON u.id=h.actor_user_id JOIN items i ON i.id=h.item_id
                 WHERE h.hash=?1 AND h.type='item_comment' AND w.guid=?2 AND i.guid=?3 AND u.guid=?4
                   AND h.from_label=?5 AND h.to_label=?6 AND h.comment=?7
                   AND h.request_device_id IS NOT NULL AND h.request_signature IS NOT NULL",
                params![ledger_hash,workspace,item,actor,guid,payload_hash,text],|row|row.get(0))?;
            if exists == 0 {
                anyhow::bail!("item comment ledger event is unavailable");
            }
        }
    }
    Ok(())
}

fn verify_stored_item_comments(conn: &Connection) -> anyhow::Result<usize> {
    let mut statement=conn.prepare(
        "SELECT r.record_hash,r.guid,r.workspace_guid,r.item_guid,r.author_guid,r.text,r.payload_hash,r.ledger_hash,r.created_at,
                h.type,h.from_label,h.to_label,h.comment,h.request_device_id,h.request_signature
         FROM item_comment_records r LEFT JOIN history_entries h ON h.hash=r.ledger_hash")?;
    let records:Vec<Value>=statement.query_map([],|row|Ok(json!({
        "recordHash":row.get::<_,String>(0)?,"guid":row.get::<_,String>(1)?,"workspaceGuid":row.get::<_,String>(2)?,
        "itemGuid":row.get::<_,String>(3)?,"authorGuid":row.get::<_,String>(4)?,"text":row.get::<_,String>(5)?,
        "payloadHash":row.get::<_,String>(6)?,"ledgerHash":row.get::<_,String>(7)?,"createdAt":row.get::<_,String>(8)?,
        "type":row.get::<_,Option<String>>(9)?,"from":row.get::<_,Option<String>>(10)?,"to":row.get::<_,Option<String>>(11)?,
        "eventText":row.get::<_,Option<String>>(12)?,"device":row.get::<_,Option<String>>(13)?,"signature":row.get::<_,Option<String>>(14)?,
    })))?.collect::<Result<_,_>>()?;
    for record in &records {
        let payload = item_comment_payload_hash(record)?;
        let ledger = record["ledgerHash"].as_str().unwrap_or("");
        let valid = record["payloadHash"].as_str() == Some(payload.as_str())
            && record["recordHash"].as_str()
                == Some(item_comment_record_hash(&payload, ledger).as_str())
            && record["type"].as_str() == Some("item_comment")
            && record["from"] == record["guid"]
            && record["to"] == record["payloadHash"]
            && record["eventText"] == record["text"]
            && record["device"].as_str().is_some_and(|v| !v.is_empty())
            && record["signature"].as_str().is_some_and(|v| !v.is_empty());
        if !valid {
            anyhow::bail!("stored item comment is not bound to its ledger event");
        }
    }
    Ok(records.len())
}

fn verify_item_tombstones(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    if journal.get("itemTombstoneMode").and_then(Value::as_str) != Some("monotonic/v1") {
        anyhow::bail!("journal does not provide item tombstones");
    }
    let incoming_history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    let archived_items: HashSet<&str> = journal
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("archived").and_then(Value::as_bool) == Some(true))
        .filter_map(|item| item.get("guid").and_then(Value::as_str))
        .collect();
    let mut seen = HashSet::new();
    for record in journal
        .get("itemTombstones")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("journal has no item tombstone array"))?
    {
        let field = |name: &str| {
            record
                .get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("item tombstone has no {name}"))
        };
        let item = field("itemGuid")?;
        let workspace = field("workspaceGuid")?;
        let actor = field("actorGuid")?;
        let ledger_hash = field("ledgerHash")?;
        let deleted_at = field("deletedAt")?;
        let tombstone_hash = field("tombstoneHash")?;
        if !seen.insert(item) {
            anyhow::bail!("duplicate item tombstone");
        }
        if !archived_items.contains(item) {
            anyhow::bail!("item tombstone is not reflected in archived state");
        }
        let expected = item_tombstone_hash(workspace, item, actor, ledger_hash, deleted_at);
        if expected != tombstone_hash {
            anyhow::bail!("item tombstone hash mismatch");
        }
        if let Some(event) = incoming_history.get(ledger_hash) {
            let proof_fields = [
                "requestDeviceId",
                "requestPublicKey",
                "requestNonce",
                "requestSignature",
                "requestHash",
                "requestTimestamp",
                "requestPath",
            ];
            let valid = event.get("type").and_then(Value::as_str) == Some("item_archive")
                && event.get("workspaceGuid").and_then(Value::as_str) == Some(workspace)
                && event.get("itemGuid").and_then(Value::as_str) == Some(item)
                && event.get("actorGuid").and_then(Value::as_str) == Some(actor)
                && event.get("fromLabel").and_then(Value::as_str) == Some(item)
                && event.get("toLabel").is_none_or(Value::is_null)
                && event.get("createdAt").and_then(Value::as_str) == Some(deleted_at)
                && proof_fields.iter().all(|name| {
                    event
                        .get(*name)
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty())
                });
            if !valid {
                anyhow::bail!("item tombstone ledger evidence mismatch");
            }
        } else {
            let exists: i64 = conn.query_row(
                "SELECT count(*) FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN users u ON u.id=h.actor_user_id JOIN items i ON i.id=h.item_id
                 WHERE h.hash=?1 AND h.type='item_archive' AND w.guid=?2 AND i.guid=?3 AND u.guid=?4
                   AND h.from_label=?3 AND h.to_label IS NULL AND h.created_at=?5
                   AND h.request_device_id IS NOT NULL AND h.request_signature IS NOT NULL",
                params![ledger_hash,workspace,item,actor,deleted_at],
                |row| row.get(0),
            )?;
            if exists == 0 {
                anyhow::bail!("item tombstone ledger event is unavailable");
            }
        }
    }
    if archived_items.len() != seen.len() {
        anyhow::bail!("archived item has no monotonic tombstone");
    }
    Ok(())
}

fn verify_stored_item_tombstones(conn: &Connection) -> anyhow::Result<usize> {
    let mut statement = conn.prepare(
        "SELECT t.item_guid,t.workspace_guid,t.actor_guid,t.ledger_hash,t.deleted_at,t.tombstone_hash,
                i.archived,w.guid,u.guid,h.type,h.from_label,h.to_label,h.created_at,
                h.request_device_id,h.request_public_key,h.request_nonce,h.request_signature,
                h.request_hash,h.request_timestamp,h.request_path
         FROM item_tombstones t
         LEFT JOIN items i ON i.guid=t.item_guid
         LEFT JOIN workspaces w ON w.id=i.workspace_id
         LEFT JOIN history_entries h ON h.hash=t.ledger_hash AND h.item_id=i.id
         LEFT JOIN users u ON u.id=h.actor_user_id",
    )?;
    let mut rows = statement.query([])?;
    let mut verified = 0usize;
    while let Some(row) = rows.next()? {
        let item: String = row.get(0)?;
        let workspace: String = row.get(1)?;
        let actor: String = row.get(2)?;
        let ledger_hash: String = row.get(3)?;
        let deleted_at: String = row.get(4)?;
        let stored_hash: String = row.get(5)?;
        let proof_complete = (13..20).all(|index| {
            row.get::<_, Option<String>>(index)
                .ok()
                .flatten()
                .is_some_and(|value| !value.is_empty())
        });
        let valid = stored_hash
            == item_tombstone_hash(&workspace, &item, &actor, &ledger_hash, &deleted_at)
            && row.get::<_, Option<i64>>(6)? == Some(1)
            && row.get::<_, Option<String>>(7)?.as_deref() == Some(workspace.as_str())
            && row.get::<_, Option<String>>(8)?.as_deref() == Some(actor.as_str())
            && row.get::<_, Option<String>>(9)?.as_deref() == Some("item_archive")
            && row.get::<_, Option<String>>(10)?.as_deref() == Some(item.as_str())
            && row.get::<_, Option<String>>(11)?.is_none()
            && row.get::<_, Option<String>>(12)?.as_deref() == Some(deleted_at.as_str())
            && proof_complete;
        if !valid {
            anyhow::bail!("stored item tombstone is not bound to its ledger event");
        }
        verified += 1;
    }
    let archived: i64 =
        conn.query_row("SELECT count(*) FROM items WHERE archived=1", [], |row| {
            row.get(0)
        })?;
    if archived != verified as i64 {
        anyhow::bail!("archived item has no stored tombstone");
    }
    Ok(verified)
}

struct JournalReceipt {
    public_key: String,
    scope: String,
    sequence: i64,
    journal_hash: String,
    duplicate: bool,
}

fn journal_receipt(conn: &Connection, journal: &Value) -> anyhow::Result<JournalReceipt> {
    if journal.get("v").and_then(Value::as_i64) != Some(2) {
        anyhow::bail!("journal v2 обязателен; legacy snapshot не имеет монотонного номера");
    }
    let public_key = journal
        .get("journalPublicKey")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("нет ключа журнала"))?
        .to_owned();
    let journal_hash = journal
        .get("journalHash")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("нет hash журнала"))?
        .to_owned();
    let sequence = journal
        .get("journalSequence")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow::anyhow!("неверный journalSequence"))?;
    let scope = journal
        .get("journalScope")
        .and_then(Value::as_str)
        .filter(|value| {
            *value == "*"
                || (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        })
        .ok_or_else(|| anyhow::anyhow!("неверный journalScope"))?
        .to_ascii_lowercase();
    let previous: Option<(i64, String)> = conn
        .query_row(
            "SELECT sequence,journal_hash FROM accepted_node_journals WHERE public_key=?1 AND scope=?2",
            params![public_key, scope],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let duplicate = match previous {
        Some((accepted, _)) if sequence < accepted => {
            anyhow::bail!("устаревший журнал {sequence}; уже принят {accepted}")
        }
        Some((accepted, hash)) if sequence == accepted && hash != journal_hash => {
            anyhow::bail!("эквивокация: разные журналы с номером {sequence}")
        }
        Some((accepted, _)) if sequence == accepted => true,
        _ => false,
    };
    Ok(JournalReceipt {
        public_key,
        scope,
        sequence,
        journal_hash,
        duplicate,
    })
}

fn store_journal_receipt(conn: &Connection, receipt: &JournalReceipt) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO accepted_node_journals(public_key,scope,sequence,journal_hash,accepted_at)
         VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(public_key,scope) DO UPDATE SET
           sequence=excluded.sequence,journal_hash=excluded.journal_hash,accepted_at=excluded.accepted_at
         WHERE excluded.sequence > accepted_node_journals.sequence",
        params![
            receipt.public_key,
            receipt.scope,
            receipt.sequence,
            receipt.journal_hash,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

fn enforce_node_trust(conn: &Connection, journal: &Value, peer_url: &str) -> anyhow::Result<()> {
    enforce_node_trust_mode(conn, journal, peer_url, strict_node_trust())
}

fn strict_node_trust() -> bool {
    std::env::var("MESHKEEPER_STRICT_NODE_TRUST").as_deref() != Ok("0")
}

fn enforce_node_trust_mode(
    conn: &Connection,
    journal: &Value,
    peer_url: &str,
    strict: bool,
) -> anyhow::Result<()> {
    if !strict {
        return Ok(());
    }
    let key = journal
        .get("journalPublicKey")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("нет публичного ключа"))?;
    if conn
        .query_row(
            "SELECT 1 FROM trusted_node_keys WHERE public_key=?1",
            [key],
            |_| Ok(()),
        )
        .is_ok()
    {
        return Ok(());
    }
    let workspaces: i64 = conn
        .query_row("SELECT count(*) FROM workspaces", [], |r| r.get(0))
        .unwrap_or(0);
    if workspaces == 0 {
        conn.execute("INSERT OR IGNORE INTO trusted_node_keys(public_key,label,source,created_at) VALUES(?1,?2,'bootstrap',?3)",params![key,journal.get("nodeName").and_then(Value::as_str),chrono::Utc::now().to_rfc3339()])?;
        return Ok(());
    }
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute("INSERT INTO pending_node_keys(public_key,peer_url,node_name,first_seen,last_seen) VALUES(?1,?2,?3,?4,?4) ON CONFLICT(public_key) DO UPDATE SET peer_url=excluded.peer_url,node_name=excluded.node_name,last_seen=excluded.last_seen",params![key,peer_url,journal.get("nodeName").and_then(Value::as_str),now])?;
    anyhow::bail!(
        "требуется одобрение владельца: {}",
        &key[..key.len().min(16)]
    )
}

pub fn node_keys(conn: &Connection) -> Value {
    let mut trusted = Vec::new();
    if let Ok(mut s)=conn.prepare("SELECT public_key,label,approved_by,source,created_at FROM trusted_node_keys ORDER BY created_at") {if let Ok(rows)=s.query_map([],|r|Ok(json!({"publicKey":r.get::<_,String>(0)?,"label":r.get::<_,Option<String>>(1)?,"approvedBy":r.get::<_,Option<i64>>(2)?,"source":r.get::<_,String>(3)?,"createdAt":r.get::<_,String>(4)?}))){trusted.extend(rows.flatten())}}
    let mut pending = Vec::new();
    if let Ok(mut s)=conn.prepare("SELECT public_key,peer_url,node_name,first_seen,last_seen FROM pending_node_keys ORDER BY last_seen DESC") {if let Ok(rows)=s.query_map([],|r|Ok(json!({"publicKey":r.get::<_,String>(0)?,"peerUrl":r.get::<_,Option<String>>(1)?,"nodeName":r.get::<_,Option<String>>(2)?,"firstSeen":r.get::<_,String>(3)?,"lastSeen":r.get::<_,String>(4)?}))){pending.extend(rows.flatten())}}
    json!({"strict":strict_node_trust(),"trusted":trusted,"pending":pending})
}

pub fn approve_node_key(
    conn: &Connection,
    key: &str,
    label: Option<&str>,
    actor: i64,
) -> anyhow::Result<()> {
    let pending: i64 = conn.query_row(
        "SELECT count(*) FROM pending_node_keys WHERE public_key=?1",
        [key],
        |r| r.get(0),
    )?;
    if pending != 1 {
        anyhow::bail!("ключ отсутствует в ожидающих")
    }
    conn.execute("INSERT INTO trusted_node_keys(public_key,label,approved_by,source,created_at) VALUES(?1,?2,?3,'approved',?4) ON CONFLICT(public_key) DO NOTHING",params![key,label,actor,chrono::Utc::now().to_rfc3339()])?;
    conn.execute("DELETE FROM pending_node_keys WHERE public_key=?1", [key])?;
    Ok(())
}

pub fn revoke_node_key(conn: &Connection, key: &str) -> anyhow::Result<()> {
    let changed = conn.execute(
        "DELETE FROM trusted_node_keys WHERE public_key=?1 AND source!='local'",
        [key],
    )?;
    if changed != 1 {
        anyhow::bail!("локальный или неизвестный ключ нельзя отозвать")
    }
    Ok(())
}

pub fn encrypt_backup(password: &str, plain: &str) -> anyhow::Result<Value> {
    use argon2::Argon2;
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    if password.chars().count() < 12 || password.chars().count() > 128 {
        anyhow::bail!("пароль архива должен содержать от 12 до 128 символов");
    }
    let salt: [u8; 16] = rand::random();
    let mut key = [0_u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)?;
    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plain.as_bytes())
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(json!({
        "v": 2,
        "alg": "argon2id+chacha20poly1305",
        "salt": hex::encode(salt),
        "nonce": hex::encode(nonce_bytes),
        "ciphertext": hex::encode(ct),
        "sha256": hex::encode(Sha256::digest(plain.as_bytes())),
    }))
}

pub fn decrypt_backup(password: &str, blob: &Value) -> anyhow::Result<String> {
    use argon2::Argon2;
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Nonce};
    let version = blob.get("v").and_then(|v| v.as_i64()).unwrap_or(1);
    let mut key = [0_u8; 32];
    if version == 1 {
        key.copy_from_slice(&Sha256::digest(password.as_bytes()));
    } else if version == 2 {
        let salt_hex = blob
            .get("salt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("нет salt"))?;
        let salt = hex::decode(salt_hex)?;
        if salt.len() != 16 {
            anyhow::bail!("некорректная длина salt");
        }
        Argon2::default()
            .hash_password_into(password.as_bytes(), &salt, &mut key)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    } else {
        anyhow::bail!("неподдерживаемая версия архива");
    }
    let cipher = ChaCha20Poly1305::new_from_slice(&key)?;
    let nonce_hex = blob
        .get("nonce")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("нет nonce"))?;
    let ct_hex = blob
        .get("ciphertext")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("нет ciphertext"))?;
    let nonce_raw = hex::decode(nonce_hex)?;
    let ct = hex::decode(ct_hex)?;
    if nonce_raw.len() != 12 {
        anyhow::bail!("некорректная длина nonce");
    }
    if ct.len() > 100 * 1024 * 1024 {
        anyhow::bail!("архив слишком большой");
    }
    let nonce = Nonce::from_slice(&nonce_raw);
    let pt = cipher
        .decrypt(nonce, ct.as_ref())
        .map_err(|_| anyhow::anyhow!("Неверный пароль или повреждённый архив"))?;
    Ok(String::from_utf8(pt)?)
}

pub fn status(conn: &Connection) -> Value {
    let (id, name) = ensure_node(conn);
    let peers = list_peers(conn);
    let conflicts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM conflicts WHERE status='open'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let upstream = std::env::var("MESHKEEPER_UPSTREAM")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty());
    let (last_sync, last_error): (Option<String>, Option<String>) = upstream
        .as_deref()
        .and_then(|u| {
            conn.query_row(
                "SELECT last_sync, last_error FROM peers WHERE url=?1",
                params![u],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()
        })
        .unwrap_or((None, None));
    let role = if upstream.is_some() {
        "node"
    } else if !peer_urls(conn).is_empty() {
        "mesh"
    } else {
        "server"
    };
    let (scope_mode, workspace_scope, capability_count) = crate::sync_capability_summary();
    json!({
        "nodeId": id,
        "name": name,
        "role": role,
        "upstream": upstream,
        "lastSync": last_sync,
        "lastError": last_error,
        "url": guess_lan_base(),
        "localUrl": local_http_base(),
        "peers": peers,
        "openConflicts": conflicts
        ,"bytesSent": metric(conn, "sync_bytes_sent")
        ,"bytesReceived": metric(conn, "sync_bytes_received")
        ,"syncSuccesses": metric(conn, "sync_successes")
        ,"workspaceScopeMode": scope_mode
        ,"workspaceScope": workspace_scope
        ,"capabilityCount": capability_count
    })
}

/// Полная локальная самопроверка для владельца узла. В отличие от `/health`
/// она читает весь криптографический журнал и строит подписанный снимок, поэтому
/// предназначена для явного аудита/редкого фонового запуска, а не для probe.
pub fn integrity_audit(conn: &Connection) -> Value {
    let database_check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .unwrap_or_else(|error| format!("error: {error}"));
    let ledger_result = ledger::verify_all(conn);
    let chat_result = ledger::verify_chat_links(conn);
    let accounting_result = crate::accounting::verify(conn);
    let knowledge_result = crate::knowledge::verify(conn);
    let snapshot = export_journal(conn);
    let snapshot_result = ledger::verify_journal(&snapshot);
    let device_result = verify_stored_device_bindings(conn);
    let custody_result = verify_stored_custody(conn);
    let item_tombstone_result = verify_stored_item_tombstones(conn);
    let item_comment_result = verify_stored_item_comments(conn);
    let fault_result = verify_stored_fault_records(conn);
    let change_result = verify_stored_change_records(conn);
    let config_result = verify_stored_config_versions(conn);
    let item_state_result = verify_stored_item_state_versions(conn);
    let organization_node_result = verify_stored_organization_node_versions(conn);
    let inventory_result = verify_stored_inventory_records(conn);
    let photo_result = verify_photo_records(conn, &snapshot);
    let document_result = verify_document_records(conn, &snapshot);
    let knowledge_intent_result = verify_knowledge_records(conn, &snapshot);
    let chat_intent_result = verify_chat_records(conn, &snapshot);
    let sale_offer_result = verify_sale_offers(&snapshot);
    let membership_result = verify_membership_records(conn, &snapshot);

    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap_or(-1)
    };
    let orphan_history: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM history_entries h
             LEFT JOIN workspaces w ON w.id=h.workspace_id
             LEFT JOIN users u ON u.id=h.actor_user_id
             WHERE w.id IS NULL OR u.id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(-1);
    let missing_guids: i64 = conn
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM workspaces WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM users WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM items WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM storages WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM building_sites WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM categories WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM brands WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM statuses WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM inventory_sessions WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM history_entries WHERE guid IS NULL OR guid='') +
               (SELECT COUNT(*) FROM chat_messages WHERE guid IS NULL OR guid='')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(-1);
    let missing_referenced_blobs: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT p.url) FROM item_photos p
             LEFT JOIN content_blobs b ON p.url='cas:' || b.hash
             WHERE p.url LIKE 'cas:%' AND b.hash IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(-1);
    let missing_blobs = crate::content::wanted_missing(conn, &snapshot).len() as i64;
    let pending_downloads = count("blob_downloads");
    let last_event_at: Option<String> = conn
        .query_row("SELECT MAX(created_at) FROM history_entries", [], |row| {
            row.get(0)
        })
        .ok()
        .flatten();
    let mut heads = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT w.guid,h.pubkey,h.hash,h.created_at
         FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id
         WHERE h.pubkey IS NOT NULL AND h.id=(
           SELECT MAX(h2.id) FROM history_entries h2
           WHERE h2.workspace_id=h.workspace_id AND h2.pubkey=h.pubkey)
         ORDER BY w.guid,h.pubkey",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok(json!({
                "workspaceGuid": row.get::<_, Option<String>>(0)?,
                "publicKey": row.get::<_, String>(1)?,
                "head": row.get::<_, String>(2)?,
                "createdAt": row.get::<_, String>(3)?,
            }))
        }) {
            heads.extend(rows.flatten());
        }
    }

    let ledger_error = ledger_result.as_ref().err().map(ToString::to_string);
    let chat_error = chat_result.as_ref().err().map(ToString::to_string);
    let accounting_error = accounting_result.as_ref().err().map(ToString::to_string);
    let knowledge_error = knowledge_result.as_ref().err().map(ToString::to_string);
    let snapshot_error = snapshot_result.as_ref().err().map(ToString::to_string);
    let device_error = device_result.as_ref().err().map(ToString::to_string);
    let custody_error = custody_result.as_ref().err().map(ToString::to_string);
    let item_tombstone_error = item_tombstone_result
        .as_ref()
        .err()
        .map(ToString::to_string);
    let item_comment_error = item_comment_result.as_ref().err().map(ToString::to_string);
    let fault_error = fault_result.as_ref().err().map(ToString::to_string);
    let change_error = change_result.as_ref().err().map(ToString::to_string);
    let config_error = config_result.as_ref().err().map(ToString::to_string);
    let item_state_error = item_state_result.as_ref().err().map(ToString::to_string);
    let organization_node_error = organization_node_result
        .as_ref()
        .err()
        .map(ToString::to_string);
    let inventory_error = inventory_result.as_ref().err().map(ToString::to_string);
    let photo_error = photo_result.as_ref().err().map(ToString::to_string);
    let document_error = document_result.as_ref().err().map(ToString::to_string);
    let knowledge_intent_error = knowledge_intent_result
        .as_ref()
        .err()
        .map(ToString::to_string);
    let chat_intent_error = chat_intent_result.as_ref().err().map(ToString::to_string);
    let membership_error = membership_result.as_ref().err().map(ToString::to_string);
    let sale_offer_error = sale_offer_result.as_ref().err().map(ToString::to_string);
    let healthy = database_check == "ok"
        && ledger_result.is_ok()
        && chat_result.is_ok()
        && accounting_result.is_ok()
        && knowledge_result.is_ok()
        && snapshot_result.is_ok()
        && device_result.is_ok()
        && custody_result.is_ok()
        && item_tombstone_result.is_ok()
        && item_comment_result.is_ok()
        && fault_result.is_ok()
        && change_result.is_ok()
        && config_result.is_ok()
        && item_state_result.is_ok()
        && organization_node_result.is_ok()
        && inventory_result.is_ok()
        && photo_result.is_ok()
        && document_result.is_ok()
        && knowledge_intent_result.is_ok()
        && chat_intent_result.is_ok()
        && membership_result.is_ok()
        && sale_offer_result.is_ok()
        && orphan_history == 0
        && missing_guids == 0
        && missing_blobs == 0
        && pending_downloads == 0;
    let counts = json!({
        "workspaces": count("workspaces"), "users": count("users"),
        "devices": count("user_devices"), "custodyEntries": count("custody_entries"),
        "items": count("items"), "history": count("history_entries"),
        "messages": count("chat_messages"), "organizationNodes": count("organization_nodes"),
        "blobs": count("content_blobs"), "accountingTransactions": count("accounting_transactions"),
        "accountingLines": count("accounting_lines"), "knowledgePages": count("knowledge_pages"),
        "knowledgeRevisions": count("knowledge_revisions"),
        "membershipVersions": count("membership_versions"),
        "itemTombstones": count("item_tombstones"),
        "itemComments": count("item_comment_records"),
        "faultRecords":count("fault_records"),
        "changeRequestRecords":count("change_request_records"),
        "configVersions":count("config_versions"),
        "itemStateVersions":count("item_state_versions"),
        "organizationNodeVersions":count("organization_node_versions"),
        "inventoryRecords":count("inventory_records"),
        "photos":count("item_photos"),
        "documents":count("item_documents"),
    });
    let mut audit = json!({
        "healthy": healthy,
        "checkedAt": chrono::Utc::now().to_rfc3339(),
        "database": database_check,
        "ledgerVerified": ledger_result.unwrap_or(0),
        "chatVerified": chat_result.unwrap_or(0),
        "ledgerError": ledger_error,
        "chatError": chat_error,
        "accountingError": accounting_error,
        "accountingVerified": accounting_result.is_ok(),
        "knowledgeError": knowledge_error,
        "knowledgeVerified": knowledge_result.is_ok(),
        "snapshotError": snapshot_error,
        "deviceError": device_error,
        "deviceRegistryVerified": device_result.is_ok(),
        "deviceProofsVerified": device_result.unwrap_or(0),
        "custodyError": custody_error,
        "custodyVerified": custody_result.as_ref().is_ok(),
        "custodyEntriesVerified": custody_result.unwrap_or(0),
        "itemTombstoneError": item_tombstone_error,
        "itemTombstonesVerified": item_tombstone_result.unwrap_or(0),
        "itemCommentError": item_comment_error,
        "itemCommentsVerified": item_comment_result.unwrap_or(0),
        "faultError":fault_error,
        "faultRecordsVerified":fault_result.unwrap_or(0),
        "changeRequestError":change_error,
        "changeRequestRecordsVerified":change_result.unwrap_or(0),
        "configError":config_error,
        "configVersionsVerified":config_result.unwrap_or(0),
        "itemStateError":item_state_error,
        "itemStateVersionsVerified":item_state_result.unwrap_or(0),
        "membershipError": membership_error,
        "membershipVerified": membership_result.is_ok(),
        "snapshotHash": snapshot.get("journalHash"),
        "lastEventAt": last_event_at,
        "orphanHistory": orphan_history,
        "missingGuids": missing_guids,
        "missingBlobs": missing_blobs,
        "missingReferencedBlobs": missing_referenced_blobs,
        "pendingDownloads": pending_downloads,
        "counts": counts,
        "ledgerHeads": heads,
    });
    if let Some(object) = audit.as_object_mut() {
        object.insert(
            "saleOffersVerified".into(),
            json!(sale_offer_result.is_ok()),
        );
        object.insert(
            "saleOfferError".into(),
            sale_offer_error.map(Value::String).unwrap_or(Value::Null),
        );
        object.insert(
            "inventoryError".into(),
            inventory_error.map(Value::String).unwrap_or(Value::Null),
        );
        object.insert(
            "inventoryRecordsVerified".into(),
            json!(inventory_result.unwrap_or(0)),
        );
        object.insert(
            "photoError".into(),
            photo_error.map(Value::String).unwrap_or(Value::Null),
        );
        object.insert("photoIntentVerified".into(), json!(photo_result.is_ok()));
        object.insert(
            "documentError".into(),
            document_error.map(Value::String).unwrap_or(Value::Null),
        );
        object.insert(
            "documentIntentVerified".into(),
            json!(document_result.is_ok()),
        );
        object.insert(
            "knowledgeIntentError".into(),
            knowledge_intent_error
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "knowledgeIntentVerified".into(),
            json!(knowledge_intent_result.is_ok()),
        );
        object.insert(
            "chatIntentError".into(),
            chat_intent_error.map(Value::String).unwrap_or(Value::Null),
        );
        object.insert(
            "chatIntentVerified".into(),
            json!(chat_intent_result.is_ok()),
        );
        object.insert(
            "organizationNodeError".into(),
            organization_node_error
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "organizationNodeVersionsVerified".into(),
            json!(organization_node_result.unwrap_or(0)),
        );
    }
    audit
}

/// Used from api.rs without making find_user_phone public — thin wrapper filled in api.
pub fn touch_peer_error(conn: &Connection, url: &str, err: &str) {
    let _ = conn.execute(
        "UPDATE peers SET last_error=?1 WHERE url=?2",
        params![err, url.trim().trim_end_matches('/')],
    );
    crate::diagnostics::record(
        conn,
        "warning",
        "sync",
        "peer_error",
        err,
        Some(&json!({"peer":url.trim().trim_end_matches('/')})),
    );
}

pub fn resolve_peer_error(conn: &Connection, url: &str) {
    crate::diagnostics::resolve(
        conn,
        "sync",
        "peer_error",
        Some(&json!({"peer":url.trim().trim_end_matches('/')})),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    const BUNDLE_SECRET: &str = "offline-bundle-test-secret-at-least-32-chars";

    fn signed_device_proof(key: &SigningKey, device_id: &str, path: &str) -> crate::device::Proof {
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let nonce = uuid::Uuid::new_v4().to_string();
        let request_hash = hex::encode(Sha256::digest(b"portable-device-test"));
        let message = format!(
            "everyday/device-request/v1\nPOST\n{path}\n{timestamp}\n{nonce}\n{request_hash}"
        );
        crate::device::Proof {
            device_id: device_id.to_owned(),
            public_key: URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes()),
            nonce,
            signature: URL_SAFE_NO_PAD.encode(key.sign(message.as_bytes()).to_bytes()),
            request_hash,
            timestamp,
            path: path.to_owned(),
            request_body: None,
        }
    }
    fn signed_device_proof_body(
        key: &SigningKey,
        device_id: &str,
        path: &str,
        body: &Value,
    ) -> crate::device::Proof {
        let request_body = body.to_string();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let nonce = uuid::Uuid::new_v4().to_string();
        let request_hash = hex::encode(Sha256::digest(request_body.as_bytes()));
        let message = format!(
            "everyday/device-request/v1\nPOST\n{path}\n{timestamp}\n{nonce}\n{request_hash}"
        );
        crate::device::Proof {
            device_id: device_id.to_owned(),
            public_key: URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes()),
            nonce,
            signature: URL_SAFE_NO_PAD.encode(key.sign(message.as_bytes()).to_bytes()),
            request_hash,
            timestamp,
            path: path.to_owned(),
            request_body: Some(request_body),
        }
    }

    #[test]
    fn pending_sale_offer_crosses_partition_and_is_accepted_by_buyer() {
        let source_path =
            std::env::temp_dir().join(format!("sale-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("sale-target-{}.db", uuid::Uuid::new_v4()));
        let rejected_path =
            std::env::temp_dir().join(format!("sale-rejected-{}.db", uuid::Uuid::new_v4()));
        let quantity_rejected_path = std::env::temp_dir().join(format!(
            "sale-quantity-rejected-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut source = db::open(&source_path).unwrap();
        let mut target = db::open(&target_path).unwrap();
        let workspace_guid = uuid::Uuid::new_v4().to_string();
        let seller_guid = uuid::Uuid::new_v4().to_string();
        let buyer_guid = uuid::Uuid::new_v4().to_string();
        let item_guid = uuid::Uuid::new_v4().to_string();
        let offer_guid = uuid::Uuid::new_v4().to_string();
        source.execute("INSERT INTO workspaces(guid,name,internal_id_prefix,created_at) VALUES(?1,'Offline sale','S-',?2)",params![workspace_guid,chrono::Utc::now().to_rfc3339()]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(guid,full_name,phone,status,role_rights,created_at) VALUES(?1,'Seller','700000001','active',?2,?3)",params![seller_guid,db::owner_rights().to_string(),chrono::Utc::now().to_rfc3339()]).unwrap();
        let seller = source.last_insert_rowid();
        source.execute("INSERT INTO users(guid,full_name,phone,status,role_rights,created_at) VALUES(?1,'Buyer','700000002','active',?2,?3)",params![buyer_guid,db::default_rights().to_string(),chrono::Utc::now().to_rfc3339()]).unwrap();
        let buyer = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![seller, workspace, db::owner_rights().to_string()],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![buyer, workspace, db::default_rights().to_string()],
            )
            .unwrap();
        source.execute("INSERT INTO items(guid,internal_id,title,responsible_user_id,workspace_id,quantitative,created_at) VALUES(?1,'SALE-1','Offline tool',?2,?3,0,?4)",params![item_guid,seller,workspace,chrono::Utc::now().to_rfc3339()]).unwrap();
        let item = source.last_insert_rowid();
        let seller_key = SigningKey::generate(&mut OsRng);
        let buyer_key = SigningKey::generate(&mut OsRng);
        let seller_device = "sale-seller-device";
        let buyer_device = "sale-buyer-device";
        for (device, user, key) in [
            (seller_device, seller, &seller_key),
            (buyer_device, buyer, &buyer_key),
        ] {
            source.execute("INSERT INTO user_devices(device_id,user_id,name,public_key,created_at) VALUES(?1,?2,'Phone',?3,?4)",params![device,user,URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes()),chrono::Utc::now().to_rfc3339()]).unwrap();
        }
        crate::device::set_pending(
            &source,
            seller,
            &signed_device_proof(&seller_key, seller_device, "/api/trpc/bit.mint"),
        )
        .unwrap();
        crate::api::dispatch(
            &mut source,
            "bit.mint",
            &json!({"workspaceId":workspace,"recipientUserId":buyer,"amount":50}),
            Some(seller),
        )
        .unwrap();
        let input = json!({"itemId":item,"toUserId":buyer,"bitAmount":20,
            "offerGuid":offer_guid,"workspaceGuid":workspace_guid,"itemGuid":item_guid,
            "buyerGuid":buyer_guid,"comment":"Partition sale"});
        let body = json!({"0":{"json":input}});
        crate::device::set_pending(
            &source,
            seller,
            &signed_device_proof_body(&seller_key, seller_device, "/api/trpc/bit.offer", &body),
        )
        .unwrap();
        let created = crate::api::dispatch(&mut source, "bit.offer", &input, Some(seller)).unwrap();
        assert_eq!(created["status"], "pending");
        let snapshot = export_journal(&source);
        assert_eq!(snapshot["saleOffers"].as_array().unwrap().len(), 1);
        let mut tampered = snapshot.clone();
        tampered["saleOffers"][0]["bitAmount"] = json!(1);
        tampered["saleOffers"][0]["recordHash"] =
            json!(sale_offer_hash(&tampered["saleOffers"][0]).unwrap());
        ledger::sign_journal(&source, &mut tampered).unwrap();
        let rejected = db::open(&rejected_path).unwrap();
        let rejection = apply_remote_journal(&rejected, &tampered, "");
        assert_eq!(rejection["ok"], false, "{rejection}");
        assert!(rejection["error"]
            .as_str()
            .unwrap_or_default()
            .contains("signed request"));
        assert_eq!(
            rejected
                .query_row("SELECT count(*) FROM transfers", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let mut quantity_tampered = snapshot.clone();
        quantity_tampered["saleOffers"][0]["quantity"] = json!(1.0);
        quantity_tampered["saleOffers"][0]["recordHash"] =
            json!(sale_offer_hash(&quantity_tampered["saleOffers"][0]).unwrap());
        ledger::sign_journal(&source, &mut quantity_tampered).unwrap();
        let quantity_rejected = db::open(&quantity_rejected_path).unwrap();
        let quantity_rejection = apply_remote_journal(&quantity_rejected, &quantity_tampered, "");
        assert_eq!(quantity_rejection["ok"], false, "{quantity_rejection}");
        let imported = apply_remote_journal(&target, &snapshot, "");
        assert_eq!(imported["ok"], true, "{imported}");
        let imported_offer: i64 = target
            .query_row(
                "SELECT id FROM transfers WHERE guid=?1 AND status='pending'",
                [offer_guid.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        let target_buyer = id_by_guid(&target, "users", &buyer_guid).unwrap();
        crate::device::set_pending(
            &target,
            target_buyer,
            &signed_device_proof(&buyer_key, buyer_device, "/api/trpc/bit.acceptSale"),
        )
        .unwrap();
        let accepted = crate::api::dispatch(
            &mut target,
            "bit.acceptSale",
            &json!({"id":imported_offer}),
            Some(target_buyer),
        )
        .unwrap();
        assert_eq!(accepted["status"], "accepted");
        assert!(accepted["bitTransactionGuid"].as_str().is_some());
        let settled = export_journal_since(&target, Some(&frontier(&source)));
        assert_eq!(settled["historyMode"], "delta");
        let first_return = apply_remote_journal(&source, &settled, "");
        assert_eq!(first_return["ok"], false);
        let target_key = ledger::node_public_key(&target).unwrap();
        approve_node_key(&source, &target_key, Some("Buyer node"), seller).unwrap();
        let returned = apply_remote_journal(&source, &settled, "");
        assert_eq!(returned["ok"], true, "{returned}");
        assert_eq!(
            source.query_row("SELECT u.guid FROM items i JOIN users u ON u.id=i.responsible_user_id WHERE i.guid=?1",[item_guid.as_str()],|row|row.get::<_,String>(0)).unwrap(),
            buyer_guid
        );
        assert_eq!(
            source
                .query_row(
                    "SELECT status FROM transfers WHERE guid=?1",
                    [offer_guid.as_str()],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "accepted"
        );
        assert_eq!(
            crate::accounting::balance(&source, workspace, buyer).unwrap(),
            30
        );
        assert_eq!(
            crate::accounting::balance(&source, workspace, seller).unwrap(),
            20
        );
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
        let _ = std::fs::remove_file(rejected_path);
        let _ = std::fs::remove_file(quantity_rejected_path);
    }

    #[test]
    fn portable_device_registry_binds_remote_proof_and_keeps_revocation_tombstone() {
        let source_path = std::env::temp_dir().join(format!(
            "portable-device-source-{}.db",
            uuid::Uuid::new_v4()
        ));
        let target_path = std::env::temp_dir().join(format!(
            "portable-device-target-{}.db",
            uuid::Uuid::new_v4()
        ));
        let rejected_path = std::env::temp_dir().join(format!(
            "portable-device-rejected-{}.db",
            uuid::Uuid::new_v4()
        ));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        let rejected = crate::db::open(&rejected_path).unwrap();
        let created = "2026-01-01T00:00:00Z";
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Org','O-',?1,'device-workspace')",[created]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,created_at,guid) VALUES('Owner','+70000000666','active',?1,'device-owner')",[created]).unwrap();
        let owner = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![owner, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, owner, true, None, true).unwrap();

        let key = SigningKey::generate(&mut OsRng);
        let device_id = "portable-device-0001";
        crate::device::register(
            &source,
            owner,
            &json!({
                "deviceId":device_id,"name":"Телефон владельца",
                "publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        let proof = signed_device_proof(&key, device_id, "/api/trpc/admin.users.update");
        crate::device::set_pending(&source, owner, &proof).unwrap();
        ledger::append(
            &source,
            workspace,
            owner,
            None,
            "update",
            None,
            None,
            None,
            Some("valid portable device proof"),
        )
        .unwrap();
        let active_journal = export_journal(&source);
        let accepted = apply_remote_journal(&target, &active_journal, "");
        assert_eq!(accepted["ok"], true, "{accepted}");
        assert_eq!(
            target.query_row(
                "SELECT u.guid FROM user_devices d JOIN users u ON u.id=d.user_id WHERE d.device_id=?1 AND d.revoked_at IS NULL",
                [device_id],
                |row| row.get::<_,String>(0),
            ).unwrap(),
            "device-owner"
        );

        let forged_key = SigningKey::generate(&mut OsRng);
        let forged = signed_device_proof(
            &forged_key,
            "unregistered-device-0002",
            "/api/trpc/admin.users.update",
        );
        crate::device::set_pending(&source, owner, &forged).unwrap();
        ledger::append(
            &source,
            workspace,
            owner,
            None,
            "update",
            None,
            None,
            None,
            Some("forged unregistered proof"),
        )
        .unwrap();
        let forged_journal = export_journal(&source);
        let result = apply_remote_journal(&rejected, &forged_journal, "");
        assert_eq!(result["ok"], false);
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no registered identity"));

        source
            .execute(
                "DELETE FROM history_entries WHERE comment='forged unregistered proof'",
                [],
            )
            .unwrap();
        source
            .execute(
                "UPDATE user_devices SET revoked_at=?1 WHERE device_id=?2",
                params![chrono::Utc::now().to_rfc3339(), device_id],
            )
            .unwrap();
        let revoked_journal = export_journal(&source);
        let result = apply_remote_journal(&target, &revoked_journal, "");
        assert_eq!(result["ok"], true, "{result}");
        assert!(target
            .query_row(
                "SELECT revoked_at FROM user_devices WHERE device_id=?1",
                [device_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
            .is_some());
        import_journal(&target, &active_journal);
        assert!(
            target
                .query_row(
                    "SELECT revoked_at FROM user_devices WHERE device_id=?1",
                    [device_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap()
                .is_some(),
            "old offline snapshot must not resurrect a revoked device"
        );

        drop((source, target, rejected));
        for path in [source_path, target_path, rejected_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn item_tombstone_is_monotonic_and_rejects_node_signed_falsification() {
        let source_path =
            std::env::temp_dir().join(format!("item-tombstone-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("item-tombstone-target-{}.db", uuid::Uuid::new_v4()));
        let rejected_path = std::env::temp_dir().join(format!(
            "item-tombstone-rejected-{}.db",
            uuid::Uuid::new_v4()
        ));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        let rejected = crate::db::open(&rejected_path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Org','O-',?1,'tombstone-workspace')",[&created]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000991','active',?1,?2,'tombstone-owner')",params![crate::db::owner_rights().to_string(),created]).unwrap();
        let owner = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![owner, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, owner, true, None, true).unwrap();
        source.execute("INSERT INTO items(internal_id,title,workspace_id,created_at,guid) VALUES('O-1','Archive me',?1,?2,'tombstone-item')",params![workspace,created]).unwrap();
        let item = source.last_insert_rowid();

        let key = SigningKey::generate(&mut OsRng);
        let device = "tombstone-device-0001";
        crate::device::register(
            &source,
            owner,
            &json!({
                "deviceId":device,"name":"Owner phone",
                "publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        let proof = signed_device_proof(&key, device, "/api/trpc/items.remove");
        crate::device::set_pending(&source, owner, &proof).unwrap();
        let event = ledger::append(
            &source,
            workspace,
            owner,
            Some(item),
            "item_archive",
            Some("tombstone-item"),
            None,
            None,
            Some("archive"),
        )
        .unwrap();
        record_item_tombstone(&source, workspace, item, owner, &event).unwrap();
        let valid = export_journal(&source);
        let accepted = apply_remote_journal(&target, &valid, "");
        assert_eq!(accepted["ok"], true, "{accepted}");
        assert_eq!(
            target
                .query_row(
                    "SELECT archived FROM items WHERE guid='tombstone-item'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        verify_stored_item_tombstones(&target).unwrap();

        // Даже нода, владеющая своим snapshot-ключом, не может переписать
        // время/смысл удаления без несовпадения с device-signed Ledger event.
        let mut forged = valid.clone();
        forged["itemTombstones"][0]["deletedAt"] = json!("2099-01-01T00:00:00Z");
        let hash = item_tombstone_hash(
            "tombstone-workspace",
            "tombstone-item",
            "tombstone-owner",
            forged["itemTombstones"][0]["ledgerHash"].as_str().unwrap(),
            "2099-01-01T00:00:00Z",
        );
        forged["itemTombstones"][0]["tombstoneHash"] = json!(hash);
        ledger::sign_journal(&source, &mut forged).unwrap();
        let result = apply_remote_journal(&rejected, &forged, "");
        assert_eq!(result["ok"], false);
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("tombstone"));
        assert_eq!(
            rejected
                .query_row("SELECT count(*) FROM items", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );

        drop((source, target, rejected));
        for path in [source_path, target_path, rejected_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn portable_text_fault_and_change_branches_reject_falsification() {
        let paths = (0..3)
            .map(|kind| {
                std::env::temp_dir()
                    .join(format!("item-comment-{kind}-{}.db", uuid::Uuid::new_v4()))
            })
            .collect::<Vec<_>>();
        let mut source = crate::db::open(&paths[0]).unwrap();
        let target = crate::db::open(&paths[1]).unwrap();
        let rejected = crate::db::open(&paths[2]).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Org','O-',?1,'comment-workspace')",[&created]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000992','active',?1,?2,'comment-owner')",params![crate::db::owner_rights().to_string(),created]).unwrap();
        let owner = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![owner, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, owner, true, None, true).unwrap();
        source.execute("INSERT INTO items(internal_id,title,workspace_id,created_at,guid) VALUES('O-1','Drill',?1,?2,'comment-item')",params![workspace,created]).unwrap();
        let item = source.last_insert_rowid();
        let key = SigningKey::generate(&mut OsRng);
        let device = "comment-device-0001";
        crate::device::register(&source,owner,&json!({"deviceId":device,"name":"Owner phone","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.addComment"),
        )
        .unwrap();
        let guid = "offline-comment-1";
        let text = "Проверить кабель перед сменой";
        let mut record = json!({"workspaceGuid":"comment-workspace","itemGuid":"comment-item","authorGuid":"comment-owner","guid":guid,"text":text,"createdAt":created});
        let payload = item_comment_payload_hash(&record).unwrap();
        let event = ledger::append(
            &source,
            workspace,
            owner,
            Some(item),
            "item_comment",
            Some(guid),
            Some(&payload),
            None,
            Some(text),
        )
        .unwrap();
        let ledger_hash = event["opId"].as_str().unwrap();
        let record_hash = item_comment_record_hash(&payload, ledger_hash);
        record["payloadHash"] = json!(payload);
        record["ledgerHash"] = json!(ledger_hash);
        record["recordHash"] = json!(record_hash);
        source.execute("INSERT INTO item_comment_records(record_hash,guid,workspace_guid,item_guid,author_guid,text,payload_hash,ledger_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![record_hash,guid,"comment-workspace","comment-item","comment-owner",text,payload,ledger_hash,created]).unwrap();
        source
            .execute(
                "INSERT INTO item_comments(item_id,user_id,text,created_at) VALUES(?1,?2,?3,?4)",
                params![item, owner, text, created],
            )
            .unwrap();
        crate::db::ensure_workspace_statuses(&source, workspace).unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.reportFault"),
        )
        .unwrap();
        let fault = crate::api::dispatch(
            &mut source,
            "items.reportFault",
            &json!({"itemId":item,"severity":"high","description":"Искрит кабель"}),
            Some(owner),
        )
        .unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.resolveFault"),
        )
        .unwrap();
        let resolved = crate::api::dispatch(
            &mut source,
            "items.resolveFault",
            &json!({"id":fault["id"],"status":"resolved","comment":"Кабель заменён"}),
            Some(owner),
        )
        .unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.requestChange"),
        )
        .unwrap();
        let change = crate::api::dispatch(
            &mut source,
            "items.requestChange",
            &json!({"itemId":item,"payload":{"title":"Drill v2"},"comment":"Уточнить модель"}),
            Some(owner),
        )
        .unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.decideChange"),
        )
        .unwrap();
        let accepted_change = crate::api::dispatch(
            &mut source,
            "items.decideChange",
            &json!({"id":change["id"],"accept":true,"reason":"Подтверждено"}),
            Some(owner),
        )
        .unwrap();
        let valid = export_journal(&source);
        let accepted = apply_remote_journal(&target, &valid, "");
        assert_eq!(accepted["ok"], true, "{accepted}");
        assert_eq!(
            target
                .query_row(
                    "SELECT text FROM item_comment_records WHERE guid=?1",
                    [guid],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            text
        );
        verify_stored_item_comments(&target).unwrap();
        assert_eq!(
            target
                .query_row(
                    "SELECT status FROM faults WHERE guid=?1",
                    [fault["guid"].as_str().unwrap()],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "resolved"
        );
        assert_eq!(verify_stored_fault_records(&target).unwrap(), 2);
        assert_eq!(
            target
                .query_row(
                    "SELECT status FROM change_requests WHERE guid=?1",
                    [change["guid"].as_str().unwrap()],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "accepted"
        );
        assert_eq!(
            target
                .query_row(
                    "SELECT title FROM items WHERE guid='comment-item'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "Drill v2"
        );
        assert_eq!(verify_stored_change_records(&target).unwrap(), 2);

        // Два администратора могут принять разные решения от одного offline-parent.
        // Обе ветки сохраняются, а materialized state выбирается одинаково на всех нодах.
        let root_hash: String = source
            .query_row(
                "SELECT record_hash FROM fault_records WHERE fault_guid=?1 AND depth=0",
                [fault["guid"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let branch_at = chrono::Utc::now().to_rfc3339();
        let mut branch = json!({"faultGuid":fault["guid"],"parentHash":root_hash,"depth":1,"workspaceGuid":"comment-workspace","itemGuid":"comment-item","reporterGuid":"comment-owner","actorGuid":"comment-owner","severity":"high","description":"Искрит кабель","photoUrl":null,"status":"repair","resolution":"Отправить в сервис","createdAt":branch_at});
        let branch_payload = fault_payload_hash(&branch).unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.resolveFault"),
        )
        .unwrap();
        let branch_event = ledger::append(
            &source,
            workspace,
            owner,
            Some(item),
            "fault_update",
            Some(fault["guid"].as_str().unwrap()),
            Some(&branch_payload),
            None,
            Some("Отправить в сервис"),
        )
        .unwrap();
        let branch_ledger = branch_event["opId"].as_str().unwrap();
        let branch_hash = fault_record_hash(&branch_payload, branch_ledger);
        branch["payloadHash"] = json!(branch_payload);
        branch["ledgerHash"] = json!(branch_ledger);
        branch["recordHash"] = json!(branch_hash);
        source.execute("INSERT INTO fault_records(record_hash,fault_guid,parent_hash,depth,workspace_guid,item_guid,reporter_guid,actor_guid,severity,description,status,resolution,payload_hash,ledger_hash,created_at) VALUES(?1,?2,?3,1,'comment-workspace','comment-item','comment-owner','comment-owner','high','Искрит кабель','repair','Отправить в сервис',?4,?5,?6)",params![branch_hash,fault["guid"].as_str(),root_hash,branch_payload,branch_ledger,branch_at]).unwrap();
        let branched = export_journal(&source);
        let merged = apply_remote_journal(&target, &branched, "");
        assert_eq!(merged["ok"], true, "{merged}");
        let expected = if branch_hash.as_str() > resolved["recordHash"].as_str().unwrap() {
            "repair"
        } else {
            "resolved"
        };
        assert_eq!(
            target
                .query_row(
                    "SELECT status FROM faults WHERE guid=?1",
                    [fault["guid"].as_str().unwrap()],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            expected
        );
        assert_eq!(
            target
                .query_row(
                    "SELECT count(*) FROM fault_records WHERE fault_guid=?1",
                    [fault["guid"].as_str().unwrap()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            3
        );
        let change_root: String = source
            .query_row(
                "SELECT record_hash FROM change_request_records WHERE request_guid=?1 AND depth=0",
                [change["guid"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let (change_patch,change_before,change_comment):(String,String,Option<String>)=source.query_row("SELECT patch_json,before_json,comment FROM change_request_records WHERE record_hash=?1",[&change_root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let decision_at = chrono::Utc::now().to_rfc3339();
        let mut rejected_branch = json!({"requestGuid":change["guid"],"parentHash":change_root,"depth":1,"workspaceGuid":"comment-workspace","itemGuid":"comment-item","requesterGuid":"comment-owner","actorGuid":"comment-owner","patch":serde_json::from_str::<Value>(&change_patch).unwrap(),"before":serde_json::from_str::<Value>(&change_before).unwrap(),"comment":change_comment,"status":"rejected","reason":"Отклонено параллельно","createdAt":decision_at});
        let decision_payload = change_payload_hash(&rejected_branch).unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.decideChange"),
        )
        .unwrap();
        let decision_event = ledger::append(
            &source,
            workspace,
            owner,
            Some(item),
            "change_decision",
            Some(change["guid"].as_str().unwrap()),
            Some(&decision_payload),
            None,
            Some("Отклонено параллельно"),
        )
        .unwrap();
        let decision_ledger = decision_event["opId"].as_str().unwrap();
        let decision_hash = change_record_hash(&decision_payload, decision_ledger);
        rejected_branch["payloadHash"] = json!(decision_payload);
        rejected_branch["ledgerHash"] = json!(decision_ledger);
        rejected_branch["recordHash"] = json!(decision_hash);
        source.execute("INSERT INTO change_request_records(record_hash,request_guid,parent_hash,depth,workspace_guid,item_guid,requester_guid,actor_guid,patch_json,before_json,comment,status,reason,payload_hash,ledger_hash,created_at) VALUES(?1,?2,?3,1,'comment-workspace','comment-item','comment-owner','comment-owner',?4,?5,?6,'rejected','Отклонено параллельно',?7,?8,?9)",params![decision_hash,change["guid"].as_str(),change_root,change_patch,change_before,change_comment,decision_payload,decision_ledger,decision_at]).unwrap();
        rebuild_change_requests(&source).unwrap();
        let change_branches = export_journal(&source);
        let merged = apply_remote_journal(&target, &change_branches, "");
        assert_eq!(merged["ok"], true, "{merged}");
        let rejection_wins =
            decision_hash.as_str() > accepted_change["recordHash"].as_str().unwrap();
        assert_eq!(
            target
                .query_row(
                    "SELECT status FROM change_requests WHERE guid=?1",
                    [change["guid"].as_str().unwrap()],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            if rejection_wins {
                "rejected"
            } else {
                "accepted"
            }
        );
        assert_eq!(
            target
                .query_row(
                    "SELECT title FROM items WHERE guid='comment-item'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            if rejection_wins { "Drill" } else { "Drill v2" }
        );
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.update"),
        )
        .unwrap();
        crate::api::dispatch(
            &mut source,
            "items.update",
            &json!({"id":item,"title":"Drill final"}),
            Some(owner),
        )
        .unwrap();
        let after_direct = export_journal(&source);
        let merged = apply_remote_journal(&target, &after_direct, "");
        assert_eq!(merged["ok"], true, "{merged}");
        assert_eq!(
            target
                .query_row(
                    "SELECT title FROM items WHERE guid='comment-item'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "Drill final"
        );

        let mut forged = valid.clone();
        forged["itemComments"][0]["text"] = json!("Поддельное указание");
        let forged_payload = item_comment_payload_hash(&forged["itemComments"][0]).unwrap();
        forged["itemComments"][0]["payloadHash"] = json!(forged_payload);
        let forged_record = item_comment_record_hash(
            forged["itemComments"][0]["payloadHash"].as_str().unwrap(),
            ledger_hash,
        );
        forged["itemComments"][0]["recordHash"] = json!(forged_record);
        ledger::sign_journal(&source, &mut forged).unwrap();
        let result = apply_remote_journal(&rejected, &forged, "");
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("комментар"));
        assert_eq!(
            rejected
                .query_row("SELECT count(*) FROM item_comment_records", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
        let mut forged_fault = valid.clone();
        let index = forged_fault["faults"]
            .as_array()
            .unwrap()
            .iter()
            .position(|r| r["depth"] == 1)
            .unwrap();
        forged_fault["faults"][index]["resolution"] = json!("Поддельное решение");
        let forged_payload = fault_payload_hash(&forged_fault["faults"][index]).unwrap();
        forged_fault["faults"][index]["payloadHash"] = json!(forged_payload);
        let fault_ledger = forged_fault["faults"][index]["ledgerHash"]
            .as_str()
            .unwrap();
        forged_fault["faults"][index]["recordHash"] = json!(fault_record_hash(
            forged_fault["faults"][index]["payloadHash"]
                .as_str()
                .unwrap(),
            fault_ledger
        ));
        ledger::sign_journal(&source, &mut forged_fault).unwrap();
        let result = apply_remote_journal(&rejected, &forged_fault, "");
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("неисправност"));
        let mut forged_change = valid;
        forged_change["changeRequests"][0]["patch"]["title"] = json!("Поддельная модель");
        let forged_payload = change_payload_hash(&forged_change["changeRequests"][0]).unwrap();
        forged_change["changeRequests"][0]["payloadHash"] = json!(forged_payload);
        let ledger = forged_change["changeRequests"][0]["ledgerHash"]
            .as_str()
            .unwrap();
        forged_change["changeRequests"][0]["recordHash"] = json!(change_record_hash(
            forged_change["changeRequests"][0]["payloadHash"]
                .as_str()
                .unwrap(),
            ledger
        ));
        ledger::sign_journal(&source, &mut forged_change).unwrap();
        let result = apply_remote_journal(&rejected, &forged_change, "");
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("заяв"));
        drop((source, target, rejected));
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn quantitative_custody_restores_from_ledger_and_rejects_forged_delta() {
        let source_path =
            std::env::temp_dir().join(format!("custody-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("custody-target-{}.db", uuid::Uuid::new_v4()));
        let rejected_path =
            std::env::temp_dir().join(format!("custody-rejected-{}.db", uuid::Uuid::new_v4()));
        let mut source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        let rejected = crate::db::open(&rejected_path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Org','O-',?1,'custody-workspace')",[&now]).unwrap();
        let workspace = source.last_insert_rowid();
        crate::db::ensure_workspace_statuses(&source, workspace).unwrap();
        source.execute("INSERT INTO users(full_name,phone,status,created_at,guid) VALUES('Worker','+70000000401','active',?1,'custody-worker')",[&now]).unwrap();
        let worker = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![worker, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, worker, true, None, true).unwrap();
        let status: i64 = source
            .query_row(
                "SELECT id FROM statuses WHERE workspace_id=?1 AND slug='in-stock'",
                [workspace],
                |row| row.get(0),
            )
            .unwrap();
        source.execute(
            "INSERT INTO items(internal_id,title,status_id,workspace_id,quantitative,quantity,created_at,guid)
             VALUES('MAT-1','Кабель',?1,?2,1,10,?3,'custody-item')",
            params![status,workspace,now],
        ).unwrap();
        let item = source.last_insert_rowid();
        let key = SigningKey::generate(&mut OsRng);
        let device_id = "custody-device-0001";
        crate::device::register(
            &source,
            worker,
            &json!({
                "deviceId":device_id,"name":"Телефон рабочего",
                "publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        let proof = signed_device_proof(&key, device_id, "/api/trpc/transfers.take");
        crate::device::set_pending(&source, worker, &proof).unwrap();
        crate::api::dispatch(
            &mut source,
            "transfers.take",
            &json!({"itemId":item,"quantity":4.0,"dueAt":"2026-09-10T12:00:00Z"}),
            Some(worker),
        )
        .unwrap();

        let journal = export_journal(&source);
        assert_eq!(journal["custody"].as_array().unwrap().len(), 1);
        let result = apply_remote_journal(&target, &journal, "");
        assert_eq!(result["ok"], true, "{result}");
        let restored: f64 = target.query_row(
            "SELECT COALESCE(SUM(h.quantity),0) FROM item_holdings h JOIN items i ON i.id=h.item_id
             WHERE i.guid='custody-item' AND h.returned_at IS NULL",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!((restored - 4.0).abs() < 1e-9, "{restored}");

        let mut forged = journal.clone();
        let record = forged["custody"]
            .as_array_mut()
            .unwrap()
            .first_mut()
            .unwrap();
        record["quantityDelta"] = json!(2.0);
        record["entryHash"] = Value::String(custody_entry_hash(
            record["workspaceGuid"].as_str().unwrap(),
            record["itemGuid"].as_str().unwrap(),
            record["userGuid"].as_str().unwrap(),
            2.0,
            record["dueAt"].as_str(),
            record["comment"].as_str(),
            record["photoUrl"].as_str(),
            record["ledgerHash"].as_str().unwrap(),
            record["createdAt"].as_str().unwrap(),
        ));
        ledger::sign_journal(&source, &mut forged).unwrap();
        let result = apply_remote_journal(&rejected, &forged, "");
        assert_eq!(result["ok"], false);
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("semantically bound"));
        assert_eq!(
            rejected
                .query_row("SELECT COUNT(*) FROM custody_entries", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );

        source.execute("INSERT INTO users(full_name,phone,status,created_at,guid) VALUES('Receiver','+70000000402','active',?1,'custody-receiver')",[&now]).unwrap();
        let receiver = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![receiver, workspace, crate::db::default_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, receiver, true, None, true).unwrap();
        let receiver_key = SigningKey::generate(&mut OsRng);
        let receiver_device = "custody-receiver-device-0002";
        crate::device::register(
            &source,
            receiver,
            &json!({
                "deviceId":receiver_device,"name":"Телефон получателя",
                "publicKey":URL_SAFE_NO_PAD.encode(receiver_key.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        let prepare_proof = signed_device_proof(&key, device_id, "/api/trpc/transfers.prepare");
        crate::device::set_pending(&source, worker, &prepare_proof).unwrap();
        let transfer = crate::api::dispatch(
            &mut source,
            "transfers.prepare",
            &json!({"itemId":item,"toUserId":receiver,"quantity":3.0}),
            Some(worker),
        )
        .unwrap();
        let accept_proof =
            signed_device_proof(&receiver_key, receiver_device, "/api/trpc/transfers.accept");
        crate::device::set_pending(&source, receiver, &accept_proof).unwrap();
        crate::api::dispatch(
            &mut source,
            "transfers.accept",
            &json!({"id":transfer["id"]}),
            Some(receiver),
        )
        .unwrap();
        let direct = export_journal(&source);
        let result = apply_remote_journal(&target, &direct, "");
        assert_eq!(result["ok"], true, "{result}");
        let balances: Vec<(String, f64)> = {
            let mut statement = target.prepare(
                "SELECT u.guid,SUM(h.quantity) FROM item_holdings h
                 JOIN items i ON i.id=h.item_id JOIN users u ON u.id=h.user_id
                 WHERE i.guid='custody-item' AND h.returned_at IS NULL GROUP BY u.guid ORDER BY u.guid",
            ).unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .flatten()
                .collect()
        };
        assert_eq!(
            balances,
            vec![
                ("custody-receiver".into(), 3.0),
                ("custody-worker".into(), 1.0)
            ]
        );

        let mut wrong_recipient = direct.clone();
        let record = wrong_recipient["custody"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["userGuid"] == "custody-receiver")
            .unwrap();
        record["userGuid"] = Value::String("custody-worker".into());
        record["entryHash"] = Value::String(custody_entry_hash(
            record["workspaceGuid"].as_str().unwrap(),
            record["itemGuid"].as_str().unwrap(),
            record["userGuid"].as_str().unwrap(),
            record["quantityDelta"].as_f64().unwrap(),
            record["dueAt"].as_str(),
            record["comment"].as_str(),
            record["photoUrl"].as_str(),
            record["ledgerHash"].as_str().unwrap(),
            record["createdAt"].as_str().unwrap(),
        ));
        ledger::sign_journal(&source, &mut wrong_recipient).unwrap();
        let result = apply_remote_journal(&rejected, &wrong_recipient, "");
        assert_eq!(result["ok"], false);
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("semantically bound"));

        drop((source, target, rejected));
        for path in [source_path, target_path, rejected_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn organization_scope_redacts_state_users_and_cas_and_rejects_foreign_journal() {
        let path = std::env::temp_dir().join(format!("scope-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::open(&path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        for (guid, name, phone, byte) in [
            ("org-a", "Организация A", "+70000000001", "QQ=="),
            ("org-b", "Организация B", "+70000000002", "Qg=="),
        ] {
            db.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES(?1,'T-',?2,?3)", params![name,now,guid]).unwrap();
            let workspace = db.last_insert_rowid();
            db.execute("INSERT INTO users(full_name,phone,password_hash,created_at,guid) VALUES(?1,?2,'secret-hash',?3,?4)",params![format!("Участник {name}"),phone,now,format!("user-{guid}")]).unwrap();
            let user = db.last_insert_rowid();
            db.execute(
                "INSERT INTO user_workspaces(user_id,workspace_id) VALUES(?1,?2)",
                params![user, workspace],
            )
            .unwrap();
            db.execute("INSERT INTO items(internal_id,title,workspace_id,responsible_user_id,created_at,guid) VALUES(?1,?2,?3,?4,?5,?6)",params![format!("{guid}-1"),format!("Инструмент {name}"),workspace,user,now,format!("item-{guid}")]).unwrap();
            let item = db.last_insert_rowid();
            let cas =
                crate::content::ingest_data_url(&db, &format!("data:text/plain;base64,{byte}"))
                    .unwrap()
                    .unwrap();
            db.execute("INSERT INTO item_photos(item_id,url,thumb_url,is_title,guid) VALUES(?1,?2,?2,1,?3)",params![item,cas,format!("photo-{guid}")]).unwrap();
        }
        let full = export_journal(&db);
        let allowed = HashSet::from(["org-a".to_string()]);
        assert!(!journal_within_scope(&full, &allowed));
        let scoped = export_journal_scoped(&db, None, Some(&allowed));
        ledger::verify_journal(&scoped).unwrap();
        assert!(journal_within_scope(&scoped, &allowed));
        assert_eq!(scoped["workspaces"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["users"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["items"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["photos"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["contentCatalog"].as_array().unwrap().len(), 1);
        let allowed_hash = scoped["contentCatalog"][0]["hash"].as_str().unwrap();
        assert!(content_hash_allowed(&db, &allowed, allowed_hash));
        let foreign_hash = full["contentCatalog"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|entry| {
                let hash = entry["hash"].as_str()?;
                (hash != allowed_hash).then_some(hash)
            })
            .unwrap();
        assert!(!content_hash_allowed(&db, &allowed, foreign_hash));
        let target_path =
            std::env::temp_dir().join(format!("scope-target-{}.db", uuid::Uuid::new_v4()));
        let target = crate::db::open(&target_path).unwrap();
        let unscoped_bundle = export_transport_bundle(&db, Some(BUNDLE_SECRET));
        assert_eq!(
            import_transport_bundle_scoped(
                &target,
                &unscoped_bundle,
                Some(BUNDLE_SECRET),
                Some(&allowed)
            )["ok"],
            false
        );
        let scoped_bundle =
            export_transport_bundle_scoped(&db, Some(BUNDLE_SECRET), Some(&allowed));
        assert_eq!(
            import_transport_bundle_scoped(
                &target,
                &scoped_bundle,
                Some(BUNDLE_SECRET),
                Some(&allowed)
            )["ok"],
            true
        );
        assert_eq!(
            target
                .query_row("SELECT count(*) FROM workspaces", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(db);
        let _ = std::fs::remove_file(path);
        drop(target);
        let _ = std::fs::remove_file(target_path);
    }

    #[test]
    fn transport_bundle_round_trips_and_rejects_tampering() {
        let source_path =
            std::env::temp_dir().join(format!("bundle-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("bundle-target-{}.db", uuid::Uuid::new_v4()));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Offline org','O-',?1,?2)",params![chrono::Utc::now().to_rfc3339(),uuid::Uuid::new_v4().to_string()]).unwrap();
        let bundle = export_transport_bundle(&source, Some(BUNDLE_SECRET));
        assert_eq!(bundle["format"], "everyday-sync-bundle");
        assert_eq!(bundle["version"], 2);
        assert!(bundle.get("journal").is_none());
        let encoded = serde_json::to_vec(&bundle).unwrap();
        let frames = crate::stream_transport::fragment(
            crate::stream_transport::PayloadKind::EncryptedBundle,
            &encoded,
            185,
        )
        .unwrap();
        let mut assembler = crate::stream_transport::Assembler::default();
        let mut transported = None;
        for frame in frames.into_iter().rev() {
            if let Some(done) = assembler.accept(&frame).unwrap() {
                transported = Some(done.bytes);
            }
        }
        let bundle: Value = serde_json::from_slice(&transported.unwrap()).unwrap();
        assert_eq!(
            import_transport_bundle(&target, &bundle, Some(BUNDLE_SECRET))["ok"],
            true
        );
        assert_eq!(
            target
                .query_row("SELECT count(*) FROM workspaces", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );

        let wrong_key = import_transport_bundle(
            &target,
            &bundle,
            Some("wrong-offline-bundle-secret-at-least-32-chars"),
        );
        assert_eq!(wrong_key["ok"], false);
        let mut forged = bundle;
        let ciphertext = forged["ciphertext"].as_str().unwrap().to_string();
        let replacement = if ciphertext.starts_with('A') {
            "B"
        } else {
            "A"
        };
        forged["ciphertext"] = json!(format!("{replacement}{}", &ciphertext[1..]));
        let rejected = import_transport_bundle(&target, &forged, Some(BUNDLE_SECRET));
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"]
            .as_str()
            .unwrap_or_default()
            .contains("повреждён"));

        // Read-only migration path for packages produced by pre-v2 nodes.
        let legacy_target_path =
            std::env::temp_dir().join(format!("bundle-legacy-{}.db", uuid::Uuid::new_v4()));
        let legacy_target = crate::db::open(&legacy_target_path).unwrap();
        let legacy = json!({
            "format":"everyday-sync-bundle", "version":1,
            "createdAt":chrono::Utc::now().to_rfc3339(), "journal":export_journal(&source)
        });
        assert_eq!(
            import_transport_bundle(&legacy_target, &legacy, None)["ok"],
            true
        );
        drop((source, target));
        drop(legacy_target);
        for path in [source_path, target_path, legacy_target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn invitation_capability_syncs_as_digest_and_works_after_offline_pull() {
        let source_path =
            std::env::temp_dir().join(format!("invite-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("invite-target-{}.db", uuid::Uuid::new_v4()));
        let source = crate::db::open(&source_path).unwrap();
        let mut target = crate::db::open(&target_path).unwrap();
        let workspace_guid = uuid::Uuid::new_v4().to_string();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Offline org','O-',?1,?2)",params![chrono::Utc::now().to_rfc3339(),workspace_guid]).unwrap();
        let workspace = source.last_insert_rowid();
        let raw = "5b0847bdfca64707a94b2d94eaeba8fa";
        let expires = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        source.execute("INSERT INTO invites(workspace_id,token,role,max_uses,used_count,revoked,created_at,expires_at) VALUES(?1,?2,'member',1,0,0,?3,?4)",params![workspace,raw,chrono::Utc::now().to_rfc3339(),expires]).unwrap();

        let bundle = export_transport_bundle(&source, Some(BUNDLE_SECRET));
        let serialized = serde_json::to_string(&bundle).unwrap();
        assert!(
            !serialized.contains(raw),
            "bearer token leaked into sync bundle"
        );
        let decrypted = decrypt_transport_journal(&bundle, BUNDLE_SECRET).unwrap();
        assert_eq!(
            decrypted["invites"][0]["tokenDigest"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(
            import_transport_bundle(&target, &bundle, Some(BUNDLE_SECRET))["ok"],
            true
        );
        let stored: String = target
            .query_row("SELECT token FROM invites", [], |row| row.get(0))
            .unwrap();
        assert!(stored.starts_with("sha256:"));
        let info =
            crate::api::dispatch(&mut target, "auth.inviteInfo", &json!({"token":raw}), None)
                .unwrap();
        assert_eq!(info["role"], "member");
        assert_eq!(info["expiresAt"], expires);
        let joined = crate::api::dispatch(
            &mut target,
            "auth.joinRegister",
            &json!({
                "token": raw,
                "fullName": "Локальный сотрудник",
                "phone": "+7 999 777-66-55",
                "password": "OfflineJoin12345"
            }),
            None,
        )
        .unwrap();
        assert!(joined["id"].as_i64().is_some());
        assert_eq!(
            target
                .query_row("SELECT COUNT(*) FROM user_workspaces", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );

        drop((source, target));
        for path in [source_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn strict_node_trust_requires_explicit_approval_after_bootstrap() {
        let source_path =
            std::env::temp_dir().join(format!("trust-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("trust-target-{}.db", uuid::Uuid::new_v4()));
        let bootstrap_path =
            std::env::temp_dir().join(format!("trust-bootstrap-{}.db", uuid::Uuid::new_v4()));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        let bootstrap = crate::db::open(&bootstrap_path).unwrap();
        target.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Established','E-',?1,?2)",params![chrono::Utc::now().to_rfc3339(),uuid::Uuid::new_v4().to_string()]).unwrap();
        let journal = export_journal(&source);
        let key = journal["journalPublicKey"].as_str().unwrap();
        assert!(
            enforce_node_trust_mode(&target, &journal, "https://new-node.invalid", true).is_err()
        );
        assert_eq!(node_keys(&target)["pending"].as_array().unwrap().len(), 1);
        approve_node_key(&target, key, Some("Телефон прораба"), 7).unwrap();
        enforce_node_trust_mode(&target, &journal, "https://new-node.invalid", true).unwrap();
        enforce_node_trust_mode(&bootstrap, &journal, "https://first-node.invalid", true).unwrap();
        assert!(node_keys(&bootstrap)["trusted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["publicKey"] == key));
        drop((source, target, bootstrap));
        for path in [source_path, target_path, bootstrap_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn signed_journal_sequence_rejects_rollback_replay_and_equivocation() {
        let source_path =
            std::env::temp_dir().join(format!("sequence-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("sequence-target-{}.db", uuid::Uuid::new_v4()));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        source.execute(
            "INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Version one','V-',?1,'sequence-workspace')",
            [chrono::Utc::now().to_rfc3339()],
        ).unwrap();
        let workspace = source.last_insert_rowid();
        let now = chrono::Utc::now().to_rfc3339();
        source
            .execute(
                "INSERT INTO organization_nodes(guid,workspace_id,kind,name,created_at,updated_at)
             VALUES('sequence-section',?1,'warehouse','Warehouse one',?2,?2)",
                params![workspace, now],
            )
            .unwrap();

        let first = export_journal(&source);
        assert_eq!(first["v"], 2);
        assert_eq!(first["journalSequence"], 1);
        assert_eq!(apply_remote_journal(&target, &first, "")["ok"], true);

        source
            .execute(
                "UPDATE organization_nodes SET name='Warehouse two',updated_at=?1
                 WHERE guid='sequence-section'",
                [chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        let second = export_journal(&source);
        assert_eq!(second["journalSequence"], 2);
        assert_eq!(apply_remote_journal(&target, &second, "")["ok"], true);
        let duplicate = apply_remote_journal(&target, &second, "");
        assert_eq!(duplicate["ok"], true);
        assert_eq!(duplicate["duplicate"], true);

        let rollback = apply_remote_journal(&target, &first, "");
        assert_eq!(rollback["ok"], false);
        assert!(rollback["error"]
            .as_str()
            .unwrap_or_default()
            .contains("устаревший"));
        assert_eq!(
            target
                .query_row(
                    "SELECT name FROM organization_nodes WHERE guid='sequence-section'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "Warehouse two"
        );

        let mut equivocation = second.clone();
        equivocation["nodeName"] = json!("Alternate signed view");
        ledger::sign_journal(&source, &mut equivocation).unwrap();
        let rejected = apply_remote_journal(&target, &equivocation, "");
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"]
            .as_str()
            .unwrap_or_default()
            .contains("эквивокация"));

        let receipt: (i64, String) = target
            .query_row(
                "SELECT sequence,journal_hash FROM accepted_node_journals",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(receipt.0, 2);
        assert_eq!(receipt.1, second["journalHash"]);

        drop((source, target));
        for path in [source_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn membership_tombstone_wins_concurrent_role_update_in_any_delivery_order() {
        let path =
            std::env::temp_dir().join(format!("membership-merge-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::open(&path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO workspaces(name,internal_id_prefix,created_at,guid)
             VALUES('Org','O-',?1,'membership-workspace')",
            [&now],
        )
        .unwrap();
        let workspace = db.last_insert_rowid();
        db.execute(
            "INSERT INTO users(full_name,phone,status,created_at,guid)
             VALUES('Member','+70000000999','active',?1,'membership-user')",
            [&now],
        )
        .unwrap();
        let user = db.last_insert_rowid();
        let rights = crate::db::default_rights().to_string();
        db.execute(
            "INSERT INTO user_workspaces(user_id,workspace_id,rights_json,role_name)
             VALUES(?1,?2,?3,'Member')",
            params![user, workspace, rights],
        )
        .unwrap();
        record_membership_version(&db, workspace, user, true, None, true).unwrap();

        let active_hash = membership_version_hash(
            "membership-workspace",
            "membership-user",
            2,
            true,
            &MembershipFields {
                rights: Some(rights.clone()),
                role_name: Some("Auditor".into()),
                ..MembershipFields::default()
            },
            Some("active-ledger"),
        );
        let tombstone_hash = membership_version_hash(
            "membership-workspace",
            "membership-user",
            2,
            false,
            &MembershipFields::default(),
            Some("revoke-ledger"),
        );
        let active = json!({
            "workspaceGuid":"membership-workspace","userGuid":"membership-user",
            "revision":2,"active":true,"rights":rights,"roleName":"Auditor",
            "ledgerHash":"active-ledger","versionHash":active_hash,"updatedAt":now
        });
        let tombstone = json!({
            "workspaceGuid":"membership-workspace","userGuid":"membership-user",
            "revision":2,"active":false,"ledgerHash":"revoke-ledger",
            "versionHash":tombstone_hash,"updatedAt":now
        });
        merge_membership_record(&db, &active);
        merge_membership_record(&db, &tombstone);
        merge_membership_record(&db, &active);
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
                params![user, workspace],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        let resolved: (i64, i64, String) = db
            .query_row(
                "SELECT revision,active,version_hash FROM membership_versions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(resolved, (2, 0, tombstone_hash));

        let rejoin_hash = membership_version_hash(
            "membership-workspace",
            "membership-user",
            3,
            true,
            &MembershipFields {
                rights: Some(rights.clone()),
                role_name: Some("Rejoined".into()),
                ..MembershipFields::default()
            },
            Some("rejoin-ledger"),
        );
        merge_membership_record(
            &db,
            &json!({
                "workspaceGuid":"membership-workspace","userGuid":"membership-user",
                "revision":3,"active":true,"rights":rights,"roleName":"Rejoined",
                "ledgerHash":"rejoin-ledger","versionHash":rejoin_hash,"updatedAt":now
            }),
        );
        assert_eq!(
            db.query_row(
                "SELECT role_name FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
                params![user, workspace],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "Rejoined"
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn node_signed_membership_payload_with_wrong_version_hash_is_rejected() {
        let source_path = std::env::temp_dir().join(format!(
            "membership-forge-source-{}.db",
            uuid::Uuid::new_v4()
        ));
        let target_path = std::env::temp_dir().join(format!(
            "membership-forge-target-{}.db",
            uuid::Uuid::new_v4()
        ));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Org','O-',?1,'forged-membership-workspace')",[&now]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,created_at,guid) VALUES('Member','+70000000888','active',?1,'forged-membership-user')",[&now]).unwrap();
        let user = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,'{}')",
                params![user, workspace],
            )
            .unwrap();
        record_membership_version(&source, workspace, user, true, None, true).unwrap();
        let mut forged = export_journal(&source);
        forged["memberships"][0]["rights"] = json!(crate::db::owner_rights().to_string());
        ledger::sign_journal(&source, &mut forged).unwrap();
        let rejected = apply_remote_journal(&target, &forged, "");
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"]
            .as_str()
            .unwrap_or_default()
            .contains("version hash mismatch"));
        assert_eq!(
            target
                .query_row("SELECT COUNT(*) FROM user_workspaces", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop((source, target));
        for path in [source_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn node_signed_admin_membership_event_without_device_proof_is_rejected() {
        let source_path = std::env::temp_dir().join(format!(
            "membership-proof-source-{}.db",
            uuid::Uuid::new_v4()
        ));
        let target_path = std::env::temp_dir().join(format!(
            "membership-proof-target-{}.db",
            uuid::Uuid::new_v4()
        ));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Org','O-',?1,'proof-workspace')",[&now]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,created_at,guid) VALUES('Owner','+70000000777','active',?1,'proof-owner')",[&now]).unwrap();
        let owner = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![owner, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, owner, true, None, true).unwrap();
        let event = ledger::append(
            &source,
            workspace,
            owner,
            None,
            "membership_update",
            Some("proof-owner"),
            Some("proof-owner"),
            None,
            Some("forged without user device"),
        )
        .unwrap();
        record_membership_version(
            &source,
            workspace,
            owner,
            true,
            event["opId"].as_str(),
            false,
        )
        .unwrap();
        let journal = export_journal(&source);
        let remote_key = journal["journalPublicKey"].as_str().unwrap();
        let rejected = apply_remote_journal(&target, &journal, "");
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no device proof"));
        assert_eq!(
            target
                .query_row(
                    "SELECT COUNT(*) FROM trusted_node_keys WHERE public_key=?1",
                    [remote_key],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "invalid journal must not bootstrap the remote key"
        );
        drop((source, target));
        for path in [source_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn membership_event_for_another_user_cannot_authorize_role_change() {
        let source_path = std::env::temp_dir().join(format!(
            "membership-target-source-{}.db",
            uuid::Uuid::new_v4()
        ));
        let target_path = std::env::temp_dir().join(format!(
            "membership-target-target-{}.db",
            uuid::Uuid::new_v4()
        ));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Org','O-',?1,'target-workspace')",[&now]).unwrap();
        let workspace = source.last_insert_rowid();
        let mut users = Vec::new();
        for (name, phone, guid) in [
            ("Owner", "+70000000301", "target-owner"),
            ("Alice", "+70000000302", "target-alice"),
            ("Bob", "+70000000303", "target-bob"),
        ] {
            source.execute(
                "INSERT INTO users(full_name,phone,status,created_at,guid) VALUES(?1,?2,'active',?3,?4)",
                params![name,phone,now,guid],
            ).unwrap();
            let user = source.last_insert_rowid();
            source.execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![user, workspace, crate::db::default_rights().to_string()],
            ).unwrap();
            record_membership_version(&source, workspace, user, true, None, true).unwrap();
            users.push(user);
        }
        let owner = users[0];
        let bob = users[2];
        let key = SigningKey::generate(&mut OsRng);
        let device_id = "membership-target-device-0001";
        crate::device::register(
            &source,
            owner,
            &json!({
                "deviceId":device_id,"name":"Телефон владельца",
                "publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
            }),
        )
        .unwrap();
        let proof = signed_device_proof(&key, device_id, "/api/trpc/admin.users.update");
        crate::device::set_pending(&source, owner, &proof).unwrap();
        let event = ledger::append(
            &source,
            workspace,
            owner,
            None,
            "membership_update",
            Some("target-alice"),
            Some("target-alice"),
            None,
            Some("Valid signature, wrong membership target"),
        )
        .unwrap();
        source
            .execute(
                "UPDATE user_workspaces SET role_name='owner' WHERE workspace_id=?1 AND user_id=?2",
                params![workspace, bob],
            )
            .unwrap();
        record_membership_version(&source, workspace, bob, true, event["opId"].as_str(), false)
            .unwrap();

        let journal = export_journal(&source);
        let remote_key = journal["journalPublicKey"].as_str().unwrap();
        let result = apply_remote_journal(&target, &journal, "");
        assert_eq!(result["ok"], false);
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("target does not match"));
        assert_eq!(
            target
                .query_row(
                    "SELECT COUNT(*) FROM trusted_node_keys WHERE public_key=?1",
                    [remote_key],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "wrong-target journal must fail before trust bootstrap"
        );
        drop((source, target));
        for path in [source_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn frontier_returns_only_descendants_and_falls_back_for_unknown_heads() {
        let event = |op: &str, previous: Option<&str>, key: &str| {
            json!({
                "workspaceGuid":"ws", "pubkey":key, "opId":op, "prevHash":previous
            })
        };
        let original = vec![
            event("a1", None, "key-a"),
            event("b1", None, "key-b"),
            event("a2", Some("a1"), "key-a"),
            event("a3", Some("a2"), "key-a"),
        ];
        let mut history = original.clone();
        retain_after_frontier(
            &mut history,
            &json!([
                {"workspaceGuid":"ws","publicKey":"key-a","head":"a2"},
                {"workspaceGuid":"ws","publicKey":"key-b","head":"b1"}
            ]),
        );
        assert_eq!(history, vec![event("a3", Some("a2"), "key-a")]);

        let mut divergent = original.clone();
        retain_after_frontier(
            &mut divergent,
            &json!([{"workspaceGuid":"ws","publicKey":"key-a","head":"unknown"}]),
        );
        assert_eq!(divergent, original);
    }

    #[test]
    fn photo_intent_rejects_trusted_node_snapshot_rewrite() {
        let path = std::env::temp_dir().join(format!("photo-intent-{}.db", uuid::Uuid::new_v4()));
        let conn = crate::db::open(&path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Photo org','P-',?1,'photo-workspace')",[&created]).unwrap();
        let workspace = conn.last_insert_rowid();
        conn.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000991','active',?1,?2,'photo-owner')",params![crate::db::owner_rights().to_string(),created]).unwrap();
        let owner = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
            params![owner, workspace, crate::db::owner_rights().to_string()],
        )
        .unwrap();
        conn.execute("INSERT INTO items(internal_id,title,workspace_id,qr_code,created_at,guid) VALUES('P-1','Camera',?1,'P-1',?2,'photo-item')",params![workspace,created]).unwrap();
        let item = conn.last_insert_rowid();
        let key = SigningKey::generate(&mut OsRng);
        let device = "photo-device-0001";
        crate::device::register(&conn,owner,&json!({"deviceId":device,"name":"Camera phone","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();
        let cas = crate::content::ingest_data_url(&conn, "data:image/png;base64,QUJD")
            .unwrap()
            .unwrap();
        let photo_guid = uuid::Uuid::new_v4().to_string();
        let checksum = cas.trim_start_matches("cas:");
        conn.execute("INSERT INTO item_photos(item_id,url,thumb_url,sha256,is_title,guid) VALUES(?1,?2,?2,?3,1,?4)",params![item,cas,checksum,photo_guid]).unwrap();
        let photo = json!({"guid":photo_guid,"itemGuid":"photo-item","url":cas,
            "thumbUrl":cas,"sha256":checksum,"isTitle":true});
        let payload_hash = photo_commitment(&photo).unwrap();
        let body = json!({"json":{"itemId":item,"itemGuid":"photo-item",
            "photoGuid":photo_guid,"url":cas,"isTitle":true}});
        let proof = signed_device_proof_body(&key, device, "/api/trpc/items.addPhoto", &body);
        crate::device::set_pending(&conn, owner, &proof).unwrap();
        ledger::append(
            &conn,
            workspace,
            owner,
            Some(item),
            "photo_add",
            Some(&photo_guid),
            Some(&payload_hash),
            None,
            Some("Photo added"),
        )
        .unwrap();

        let valid = export_journal(&conn);
        verify_photo_records(&conn, &valid).unwrap();
        let mut forged = valid;
        forged["photos"][0]["isTitle"] = json!(false);
        ledger::sign_journal(&conn, &mut forged).unwrap();
        ledger::verify_journal(&forged).unwrap();
        let error = verify_photo_records(&conn, &forged).unwrap_err();
        assert!(error.to_string().contains("ledger evidence mismatch"));

        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn document_intent_rejects_trusted_node_snapshot_rewrite() {
        let path =
            std::env::temp_dir().join(format!("document-intent-{}.db", uuid::Uuid::new_v4()));
        let conn = crate::db::open(&path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Docs org','D-',?1,'docs-workspace')",[&created]).unwrap();
        let workspace = conn.last_insert_rowid();
        conn.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000992','active',?1,?2,'docs-owner')",params![crate::db::owner_rights().to_string(),created]).unwrap();
        let owner = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
            params![owner, workspace, crate::db::owner_rights().to_string()],
        )
        .unwrap();
        conn.execute("INSERT INTO items(internal_id,title,workspace_id,qr_code,created_at,guid) VALUES('D-1','Manual',?1,'D-1',?2,'docs-item')",params![workspace,created]).unwrap();
        let item = conn.last_insert_rowid();
        let key = SigningKey::generate(&mut OsRng);
        let device = "docs-device-0001";
        crate::device::register(&conn,owner,&json!({"deviceId":device,"name":"Docs phone","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();
        let cas = crate::content::ingest_data_url(&conn, "data:application/pdf;base64,QUJD")
            .unwrap()
            .unwrap();
        let document_guid = uuid::Uuid::new_v4().to_string();
        let checksum = cas.trim_start_matches("cas:");
        conn.execute("INSERT INTO item_documents(item_id,name,url,guid,mime,sha256,author_id,access_level) VALUES(?1,'Manual.pdf',?2,?3,'application/pdf',?4,?5,'members')",params![item,cas,document_guid,checksum,owner]).unwrap();
        let document = json!({"guid":document_guid,"itemGuid":"docs-item",
            "name":"Manual.pdf","url":cas,"mime":"application/pdf","sha256":checksum,
            "authorGuid":"docs-owner","accessLevel":"members"});
        let payload_hash = document_commitment(&document).unwrap();
        let body = json!({"json":{"itemId":item,"itemGuid":"docs-item",
            "documentGuid":document_guid,"name":"Manual.pdf","url":cas,
            "mime":"application/pdf","accessLevel":"members"}});
        let proof = signed_device_proof_body(&key, device, "/api/trpc/items.addDocument", &body);
        crate::device::set_pending(&conn, owner, &proof).unwrap();
        ledger::append(
            &conn,
            workspace,
            owner,
            Some(item),
            "document_add",
            Some(&document_guid),
            Some(&payload_hash),
            None,
            Some("Document added"),
        )
        .unwrap();

        let valid = export_journal(&conn);
        verify_document_records(&conn, &valid).unwrap();
        let mut forged = valid;
        forged["documents"][0]["accessLevel"] = json!("accounting");
        ledger::sign_journal(&conn, &mut forged).unwrap();
        ledger::verify_journal(&forged).unwrap();
        let error = verify_document_records(&conn, &forged).unwrap_err();
        assert!(error.to_string().contains("ledger evidence mismatch"));

        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn knowledge_intent_rejects_trusted_node_snapshot_rewrite() {
        let path =
            std::env::temp_dir().join(format!("knowledge-intent-{}.db", uuid::Uuid::new_v4()));
        let conn = crate::db::open(&path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        let workspace_guid = uuid::Uuid::new_v4().to_string();
        let owner_guid = uuid::Uuid::new_v4().to_string();
        conn.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Wiki org','W-',?1,?2)",params![created,workspace_guid]).unwrap();
        let workspace = conn.last_insert_rowid();
        conn.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000993','active',?1,?2,?3)",params![crate::db::owner_rights().to_string(),created,owner_guid]).unwrap();
        let owner = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
            params![owner, workspace, crate::db::owner_rights().to_string()],
        )
        .unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let device = "wiki-device-0001";
        crate::device::register(&conn,owner,&json!({"deviceId":device,"name":"Wiki phone","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();
        let page_guid = uuid::Uuid::new_v4().to_string();
        let revision_guid = uuid::Uuid::new_v4().to_string();
        let page = crate::knowledge::save(
            &conn,
            workspace,
            owner,
            "safety",
            "Safety",
            "Inspect tools",
            "members",
            None,
            &json!([]),
            Some(&page_guid),
            Some(&revision_guid),
        )
        .unwrap();
        let revision_hash = page["savedRevisionHash"].as_str().unwrap();
        let body = json!({"json":{"workspaceId":workspace,"workspaceGuid":workspace_guid,
            "pageGuid":page_guid,"revisionGuid":revision_guid,"slug":"safety",
            "title":"Safety","content":"Inspect tools","visibility":"members",
            "attachments":[]}});
        let proof = signed_device_proof_body(&key, device, "/api/trpc/knowledge.save", &body);
        crate::device::set_pending(&conn, owner, &proof).unwrap();
        ledger::append(
            &conn,
            workspace,
            owner,
            None,
            "knowledge_revision",
            Some(&page_guid),
            Some(revision_hash),
            None,
            Some("Wiki revision"),
        )
        .unwrap();

        let valid = export_journal(&conn);
        verify_knowledge_records(&conn, &valid).unwrap();
        assert!(valid["knowledge"]["revisions"][0]["ledgerHash"].is_string());
        let mut forged = valid;
        forged["knowledge"]["revisions"][0]["title"] = json!("Forged title");
        ledger::sign_journal(&conn, &mut forged).unwrap();
        ledger::verify_journal(&forged).unwrap();
        let error = verify_knowledge_records(&conn, &forged).unwrap_err();
        assert!(error
            .to_string()
            .contains("differs from signed user intent"));

        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chat_intent_rejects_trusted_node_snapshot_rewrite() {
        let path = std::env::temp_dir().join(format!("chat-intent-{}.db", uuid::Uuid::new_v4()));
        let conn = crate::db::open(&path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        let workspace_guid = uuid::Uuid::new_v4().to_string();
        let owner_guid = uuid::Uuid::new_v4().to_string();
        conn.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Chat org','C-',?1,?2)",params![created,workspace_guid]).unwrap();
        let workspace = conn.last_insert_rowid();
        conn.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000994','active',?1,?2,?3)",params![crate::db::owner_rights().to_string(),created,owner_guid]).unwrap();
        let owner = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
            params![owner, workspace, crate::db::owner_rights().to_string()],
        )
        .unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let device = "chat-device-0001";
        crate::device::register(&conn,owner,&json!({"deviceId":device,"name":"Chat phone","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();
        let cas = crate::content::ingest_data_url(&conn, "data:text/plain;base64,QUJD")
            .unwrap()
            .unwrap();
        let message_guid = uuid::Uuid::new_v4().to_string();
        let attachments = json!([{"name":"note.txt","url":cas,"mime":"text/plain",
            "sha256":cas.trim_start_matches("cas:")}]);
        let commitment = ledger::chat_commitment(
            &message_guid,
            &workspace_guid,
            &owner_guid,
            "Original message",
            &attachments,
        );
        let body = json!({"json":{"workspaceId":workspace,"workspaceGuid":workspace_guid,
            "messageGuid":message_guid,"text":"Original message",
            "attachments":[{"name":"note.txt","url":cas,"mime":"text/plain"}]}});
        let proof = signed_device_proof_body(&key, device, "/api/trpc/chat.send", &body);
        crate::device::set_pending(&conn, owner, &proof).unwrap();
        let event = ledger::append(
            &conn,
            workspace,
            owner,
            None,
            "chat_message",
            Some(&message_guid),
            Some(&commitment),
            None,
            Some("Original message"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_messages(guid,workspace_id,user_id,text,attachments_json,ledger_hash,created_at)
             VALUES(?1,?2,?3,'Original message',?4,?5,?6)",
            params![message_guid,workspace,owner,attachments.to_string(),event["opId"].as_str(),created],
        ).unwrap();

        let valid = export_journal(&conn);
        verify_chat_records(&conn, &valid).unwrap();
        let mut forged = valid;
        forged["messages"][0]["text"] = json!("Forged message");
        ledger::sign_journal(&conn, &mut forged).unwrap();
        ledger::verify_journal(&forged).unwrap();
        let error = verify_chat_records(&conn, &forged).unwrap_err();
        assert!(error.to_string().contains("ledger evidence mismatch"));

        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn signed_config_survives_offline_sync_and_rejects_falsification() {
        let source_path =
            std::env::temp_dir().join(format!("config-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("config-target-{}.db", uuid::Uuid::new_v4()));
        let rejected_path =
            std::env::temp_dir().join(format!("config-rejected-{}.db", uuid::Uuid::new_v4()));
        let source = crate::db::open(&source_path).unwrap();
        let target = crate::db::open(&target_path).unwrap();
        let rejected = crate::db::open(&rejected_path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Offline org','O-',?1,'config-workspace')",[&created]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000077','active',?1,?2,'config-owner')",params![crate::db::owner_rights().to_string(),created]).unwrap();
        let owner = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![owner, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, owner, true, None, true).unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let device = "config-device-0001";
        crate::device::register(&source,owner,&json!({"deviceId":device,"name":"Owner phone","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();
        source.execute("INSERT INTO storages(name,address,workspace_id,guid) VALUES('Mobile store','Mesh sector',?1,'storage-portable')",[workspace]).unwrap();
        let fields =
            json!({"name":"Mobile store","address":"Mesh sector","responsibleGuid":Value::Null});
        let payload_record = json!({"kind":"storage","entityGuid":"storage-portable","parentHash":Value::Null,"depth":0,"workspaceGuid":"config-workspace","actorGuid":"config-owner","active":true,"fields":fields,"updatedAt":created});
        let payload_hash = config_payload_hash(&payload_record).unwrap();
        let proof = signed_device_proof(&key, device, "/api/trpc/admin.storages.create");
        crate::device::set_pending(&source, owner, &proof).unwrap();
        let event = ledger::append(
            &source,
            workspace,
            owner,
            None,
            "config_create",
            Some("storage-portable"),
            Some(&payload_hash),
            None,
            Some("portable config"),
        )
        .unwrap();
        let ledger_hash = event["opId"].as_str().unwrap();
        let version_hash = config_version_hash(&payload_hash, ledger_hash);
        source.execute("INSERT INTO config_versions(version_hash,entity_guid,kind,parent_hash,depth,workspace_guid,actor_guid,active,fields_json,payload_hash,ledger_hash,updated_at) VALUES(?1,'storage-portable','storage',NULL,0,'config-workspace','config-owner',1,?2,?3,?4,?5)",params![version_hash,fields.to_string(),payload_hash,ledger_hash,created]).unwrap();
        let valid = export_journal(&source);
        let accepted = apply_remote_journal(&target, &valid, "");
        assert_eq!(accepted["ok"], true, "{accepted}");
        assert_eq!(
            target
                .query_row(
                    "SELECT address FROM storages WHERE guid='storage-portable'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "Mesh sector"
        );
        assert_eq!(verify_stored_config_versions(&target).unwrap(), 1);

        let mut forged = valid;
        forged["configVersions"][0]["fields"]["address"] = json!("Forged HQ");
        ledger::sign_journal(&source, &mut forged).unwrap();
        let result = apply_remote_journal(&rejected, &forged, "");
        assert_eq!(result["ok"], false);
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("справочников"));
        drop((source, target, rejected));
        for path in [source_path, target_path, rejected_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn signed_inventory_round_trip_and_tamper_rejection() {
        let source_path =
            std::env::temp_dir().join(format!("inventory-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("inventory-target-{}.db", uuid::Uuid::new_v4()));
        let rejected_path =
            std::env::temp_dir().join(format!("inventory-rejected-{}.db", uuid::Uuid::new_v4()));
        let mut source = crate::db::open(&source_path).unwrap();
        let mut target = crate::db::open(&target_path).unwrap();
        let rejected = crate::db::open(&rejected_path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Inventory org','I-',?1,'inventory-workspace')",[&created]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000111','active',?1,?2,'inventory-owner')",params![crate::db::owner_rights().to_string(),created]).unwrap();
        let owner = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![owner, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, owner, true, None, true).unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let device = "inventory-device-0001";
        crate::device::register(&source,owner,&json!({"deviceId":device,"name":"Scanner","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.create"),
        )
        .unwrap();
        let item=crate::api::dispatch(&mut source,"items.create",&json!({"workspaceId":workspace,"title":"Cable","internalId":"I-0001","quantitative":true,"quantity":10,"unit":"pcs"}),Some(owner)).unwrap();
        let create_input = json!({"workspaceId":workspace,"blockTransfers":true});
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof_body(
                &key,
                device,
                "/api/trpc/inventory.create",
                &json!({"0":{"json":create_input}}),
            ),
        )
        .unwrap();
        let session =
            crate::api::dispatch(&mut source, "inventory.create", &create_input, Some(owner))
                .unwrap();
        let check_input =
            json!({"sessionId":session["id"],"itemId":item["id"],"checked":true,"actualQty":7});
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof_body(
                &key,
                device,
                "/api/trpc/inventory.checkItem",
                &json!({"0":{"json":check_input}}),
            ),
        )
        .unwrap();
        crate::api::dispatch(
            &mut source,
            "inventory.checkItem",
            &check_input,
            Some(owner),
        )
        .unwrap();
        let complete_input = json!({"sessionId":session["id"]});
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof_body(
                &key,
                device,
                "/api/trpc/inventory.complete",
                &json!({"0":{"json":complete_input}}),
            ),
        )
        .unwrap();
        crate::api::dispatch(
            &mut source,
            "inventory.complete",
            &complete_input,
            Some(owner),
        )
        .unwrap();
        let portable_act = crate::api::dispatch(
            &mut source,
            "inventory.act",
            &json!({"id":session["id"]}),
            Some(owner),
        )
        .unwrap();
        let valid = export_journal(&source);
        assert_eq!(valid["inventoryRecords"].as_array().unwrap().len(), 3);
        let accepted = apply_remote_journal(&target, &valid, "");
        assert_eq!(accepted["ok"], true, "{accepted}");
        let restored:(String,f64,bool,bool)=target.query_row("SELECT s.status,r.actual_qty,r.checked!=0,s.block_transfers!=0 FROM inventory_sessions s JOIN inventory_results r ON r.session_id=s.id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(restored, ("completed".into(), 7.0, true, true));
        assert_eq!(verify_stored_inventory_records(&target).unwrap(), 3);
        let restored_owner: i64 = target
            .query_row(
                "SELECT id FROM users WHERE guid='inventory-owner'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        target
            .execute(
                "INSERT OR IGNORE INTO trusted_node_keys(public_key,label,source,created_at)
             VALUES(?1,'Source inventory node','approved',?2)",
                params![ledger::node_public_key(&source).unwrap(), created],
            )
            .unwrap();
        let remote_verification = crate::api::dispatch(
            &mut target,
            "inventory.verifyAct",
            &json!({"document":portable_act}),
            Some(restored_owner),
        )
        .unwrap();
        assert_eq!(remote_verification["verdict"], "verified");
        assert_eq!(remote_verification["localMatch"], true);
        let mut forged = valid.clone();
        forged["inventoryRecords"][1]["fields"]["actualQty"] = json!(99);
        ledger::sign_journal(&source, &mut forged).unwrap();
        let result = apply_remote_journal(&rejected, &forged, "");
        assert_eq!(result["ok"], false, "{result}");
        assert!(
            result["error"]
                .as_str()
                .unwrap_or_default()
                .contains("инвентаризации"),
            "{result}"
        );
        let session_guid = valid["inventoryRecords"][0]["sessionGuid"]
            .as_str()
            .unwrap()
            .to_string();
        let workspace_guid = "inventory-workspace";
        let actor_guid = "inventory-owner";
        let item_guid = item["guid"].as_str().unwrap();
        let malicious_fields = json!({"expectedQty":10.0,"actualQty":99.0,"checked":true});
        let malicious_created = chrono::Utc::now().to_rfc3339();
        let malicious_record = json!({"sessionGuid":session_guid,"workspaceGuid":workspace_guid,"actorGuid":actor_guid,"kind":"check","itemGuid":item_guid,"fields":malicious_fields,"createdAt":malicious_created});
        let malicious_payload = inventory_payload_hash(&malicious_record).unwrap();
        let signed_intent = json!({"0":{"json":{"sessionId":session["id"],"itemId":item["id"],"checked":true,"actualQty":7}}});
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof_body(
                &key,
                device,
                "/api/trpc/inventory.checkItem",
                &signed_intent,
            ),
        )
        .unwrap();
        let malicious_event = ledger::append(
            &source,
            workspace,
            owner,
            Some(item["id"].as_i64().unwrap()),
            "inventory_check",
            Some(&session_guid),
            Some(&malicious_payload),
            None,
            Some("semantic substitution attempt"),
        )
        .unwrap();
        let malicious_ledger = malicious_event["opId"].as_str().unwrap();
        let malicious_hash = inventory_record_hash(&malicious_payload, malicious_ledger);
        source.execute("INSERT INTO inventory_records(record_hash,session_guid,workspace_guid,actor_guid,kind,item_guid,fields_json,payload_hash,ledger_hash,created_at) VALUES(?1,?2,?3,?4,'check',?5,?6,?7,?8,?9)",params![malicious_hash,session_guid,workspace_guid,actor_guid,item_guid,malicious_fields.to_string(),malicious_payload,malicious_ledger,malicious_created]).unwrap();
        let semantic_result = apply_remote_journal(&rejected, &export_journal(&source), "");
        assert_eq!(semantic_result["ok"], false, "{semantic_result}");
        assert!(
            semantic_result["error"]
                .as_str()
                .unwrap_or_default()
                .contains("signed user intent"),
            "{semantic_result}"
        );
        drop((source, target, rejected));
        for path in [source_path, target_path, rejected_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn signed_organization_tree_replication_rejects_snapshot_rewrite() {
        let source_path =
            std::env::temp_dir().join(format!("org-tree-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("org-tree-target-{}.db", uuid::Uuid::new_v4()));
        let rejected_path =
            std::env::temp_dir().join(format!("org-tree-rejected-{}.db", uuid::Uuid::new_v4()));
        let mut source = crate::db::open(&source_path).unwrap();
        let mut target = crate::db::open(&target_path).unwrap();
        let rejected = crate::db::open(&rejected_path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Tree org','T-',?1,'tree-workspace')",[&created]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000077','active',?1,?2,'tree-owner')",params![crate::db::owner_rights().to_string(),created]).unwrap();
        let owner = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![owner, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, owner, true, None, true).unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let device = "tree-device-0001";
        crate::device::register(&source,owner,&json!({"deviceId":device,"name":"Owner phone","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();

        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/admin.organizationNodes.create"),
        )
        .unwrap();
        let division = crate::api::dispatch(
            &mut source,
            "admin.organizationNodes.create",
            &json!({"workspaceId":workspace,"kind":"division","name":"Production"}),
            Some(owner),
        )
        .unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/admin.organizationNodes.create"),
        )
        .unwrap();
        crate::api::dispatch(&mut source,"admin.organizationNodes.create",&json!({"workspaceId":workspace,"parentId":division["id"],"kind":"warehouse","name":"Offline warehouse"}),Some(owner)).unwrap();

        let valid = export_journal(&source);
        assert_eq!(
            valid["organizationNodeVersions"].as_array().unwrap().len(),
            2
        );
        let accepted = apply_remote_journal(&target, &valid, "");
        assert_eq!(accepted["ok"], true, "{accepted}");
        assert_eq!(target.query_row("SELECT count(*) FROM organization_nodes WHERE name IN ('Production','Offline warehouse')",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert_eq!(
            verify_stored_organization_node_versions(&target).unwrap(),
            2
        );

        let target_owner = id_by_guid(&target, "users", "tree-owner").unwrap();
        let target_division = id_by_guid(
            &target,
            "organization_nodes",
            division["guid"].as_str().unwrap(),
        )
        .unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/admin.organizationNodes.update"),
        )
        .unwrap();
        crate::api::dispatch(
            &mut source,
            "admin.organizationNodes.update",
            &json!({"id":division["id"],"name":"Offline branch A"}),
            Some(owner),
        )
        .unwrap();
        crate::device::set_pending(
            &target,
            target_owner,
            &signed_device_proof(&key, device, "/api/trpc/admin.organizationNodes.update"),
        )
        .unwrap();
        crate::api::dispatch(
            &mut target,
            "admin.organizationNodes.update",
            &json!({"id":target_division,"name":"Offline branch B"}),
            Some(target_owner),
        )
        .unwrap();
        assert_eq!(
            apply_remote_journal(&target, &export_journal(&source), "")["ok"],
            true
        );
        let target_branch = export_journal(&target);
        assert_eq!(
            apply_remote_journal(&source, &target_branch, "")["ok"],
            false
        );
        approve_node_key(
            &source,
            target_branch["journalPublicKey"].as_str().unwrap(),
            Some("offline target"),
            owner,
        )
        .unwrap();
        assert_eq!(
            apply_remote_journal(&source, &target_branch, "")["ok"],
            true
        );
        let source_name: String = source
            .query_row(
                "SELECT name FROM organization_nodes WHERE guid=?1",
                [division["guid"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let target_name: String = target
            .query_row(
                "SELECT name FROM organization_nodes WHERE guid=?1",
                [division["guid"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(source_name, target_name, "offline branches must converge");
        assert_eq!(
            verify_stored_organization_node_versions(&source).unwrap(),
            4
        );

        let mut forged = valid;
        forged["organizationNodes"][0]["name"] = json!("Forged division");
        ledger::sign_journal(&source, &mut forged).unwrap();
        let result = apply_remote_journal(&rejected, &forged, "");
        assert_eq!(result["ok"], false, "{result}");
        assert!(
            result["error"]
                .as_str()
                .unwrap_or_default()
                .contains("структуры"),
            "{result}"
        );
        drop((source, target, rejected));
        for path in [source_path, target_path, rejected_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn device_signed_item_state_rejects_a_trusted_node_rewrite() {
        let source_path =
            std::env::temp_dir().join(format!("item-state-source-{}.db", uuid::Uuid::new_v4()));
        let target_path =
            std::env::temp_dir().join(format!("item-state-target-{}.db", uuid::Uuid::new_v4()));
        let rejected_path =
            std::env::temp_dir().join(format!("item-state-rejected-{}.db", uuid::Uuid::new_v4()));
        let mut source = crate::db::open(&source_path).unwrap();
        let mut target = crate::db::open(&target_path).unwrap();
        let rejected = crate::db::open(&rejected_path).unwrap();
        let created = chrono::Utc::now().to_rfc3339();
        source.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Item org','I-',?1,'item-state-workspace')",[&created]).unwrap();
        let workspace = source.last_insert_rowid();
        source.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES('Owner','+70000000088','active',?1,?2,'item-state-owner')",params![crate::db::owner_rights().to_string(),created]).unwrap();
        let owner = source.last_insert_rowid();
        source
            .execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![owner, workspace, crate::db::owner_rights().to_string()],
            )
            .unwrap();
        record_membership_version(&source, workspace, owner, true, None, true).unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let device = "item-state-device-1";
        crate::device::register(&source,owner,&json!({"deviceId":device,"name":"Owner phone","publicKey":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})).unwrap();
        let proof = signed_device_proof(&key, device, "/api/trpc/items.create");
        crate::device::set_pending(&source, owner, &proof).unwrap();
        let created_item=crate::api::dispatch(&mut source,"items.create",&json!({"workspaceId":workspace,"title":"Signed drill","internalId":"I-0001","metadata":{"manual":"A-1"},"quantitative":true,"quantity":10,"unit":"pcs"}),Some(owner)).unwrap();
        let item_guid = created_item["guid"].as_str().unwrap().to_string();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/transfers.take"),
        )
        .unwrap();
        crate::api::dispatch(
            &mut source,
            "transfers.take",
            &json!({"itemId":created_item["id"],"quantity":4}),
            Some(owner),
        )
        .unwrap();
        let valid = export_journal(&source);
        assert_eq!(valid["itemStateVersions"].as_array().unwrap().len(), 1);
        let accepted = apply_remote_journal(&target, &valid, "");
        assert_eq!(accepted["ok"], true, "{accepted}");
        assert_eq!(
            target
                .query_row("SELECT title FROM items WHERE guid=?1", [&item_guid], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "Signed drill"
        );
        assert_eq!(target.query_row("SELECT COALESCE(sum(h.quantity),0) FROM item_holdings h JOIN items i ON i.id=h.item_id WHERE i.guid=?1 AND h.returned_at IS NULL",[&item_guid],|r|r.get::<_,f64>(0)).unwrap(),4.0);
        let repeated = apply_remote_journal(&target, &export_journal(&source), "");
        assert_eq!(repeated["ok"], true, "{repeated}");
        assert_eq!(target.query_row("SELECT COALESCE(sum(h.quantity),0) FROM item_holdings h JOIN items i ON i.id=h.item_id WHERE i.guid=?1 AND h.returned_at IS NULL",[&item_guid],|r|r.get::<_,f64>(0)).unwrap(),4.0);
        let target_owner = id_by_guid(&target, "users", "item-state-owner").unwrap();
        let target_item = id_by_guid(&target, "items", &item_guid).unwrap();
        crate::device::set_pending(
            &source,
            owner,
            &signed_device_proof(&key, device, "/api/trpc/items.update"),
        )
        .unwrap();
        crate::api::dispatch(
            &mut source,
            "items.update",
            &json!({"id":created_item["id"],"title":"Offline branch A"}),
            Some(owner),
        )
        .unwrap();
        crate::device::set_pending(
            &target,
            target_owner,
            &signed_device_proof(&key, device, "/api/trpc/items.update"),
        )
        .unwrap();
        crate::api::dispatch(
            &mut target,
            "items.update",
            &json!({"id":target_item,"title":"Offline branch B"}),
            Some(target_owner),
        )
        .unwrap();
        let branch_a = export_journal(&source);
        let merged = apply_remote_journal(&target, &branch_a, "");
        assert_eq!(merged["ok"], true, "{merged}");
        let target_journal = export_journal(&target);
        assert_eq!(
            apply_remote_journal(&source, &target_journal, "")["ok"],
            false
        );
        approve_node_key(
            &source,
            target_journal["journalPublicKey"].as_str().unwrap(),
            Some("target"),
            owner,
        )
        .unwrap();
        let converged = apply_remote_journal(&source, &target_journal, "");
        assert_eq!(converged["ok"], true, "{converged}");
        let source_title: String = source
            .query_row("SELECT title FROM items WHERE guid=?1", [&item_guid], |r| {
                r.get(0)
            })
            .unwrap();
        let target_title: String = target
            .query_row("SELECT title FROM items WHERE guid=?1", [&item_guid], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(source_title, target_title);
        assert_eq!(
            source
                .query_row(
                    "SELECT count(*) FROM item_state_versions WHERE item_guid=?1",
                    [&item_guid],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            3
        );
        let mut forged = valid;
        let index = forged["items"]
            .as_array()
            .unwrap()
            .iter()
            .position(|item| item["guid"] == item_guid)
            .unwrap();
        forged["items"][index]["title"] = json!("Node-forged drill");
        ledger::sign_journal(&source, &mut forged).unwrap();
        let result = apply_remote_journal(&rejected, &forged, "");
        assert_eq!(result["ok"], false);
        assert!(result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("master-летописи"));
        assert_eq!(
            rejected
                .query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop((source, target, rejected));
        for path in [source_path, target_path, rejected_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn backup_v2_round_trip() {
        let blob = encrypt_backup("correct horse battery staple", "important data").unwrap();
        assert_eq!(blob["v"], 2);
        assert_eq!(
            decrypt_backup("correct horse battery staple", &blob).unwrap(),
            "important data"
        );
        assert!(decrypt_backup("wrong password", &blob).is_err());
    }

    #[test]
    fn malformed_backup_nonce_is_an_error_not_a_panic() {
        let blob = json!({
            "v": 1,
            "nonce": "00",
            "ciphertext": "00"
        });
        let result = std::panic::catch_unwind(|| decrypt_backup("password", &blob));
        assert!(result.is_ok());
        assert!(result.unwrap().is_err());
    }
}
