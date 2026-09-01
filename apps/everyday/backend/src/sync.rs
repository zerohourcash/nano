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
        "SELECT id, internal_id, title, category_id, status_id, responsible_user_id, workspace_id, serial_number, qr_code, due_at, guid, calibrated_until, min_quantity, quantitative, quantity, unit, cost, comment, source_system, external_id, metadata_json, organization_node_id FROM items",
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
                "localId": id,
            }))
        }).into_iter().flatten().flatten() {
            items.push(row);
        }
    }
    let mut history = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, workspace_id, item_id, type, actor_user_id, from_label, to_label, quantity_delta, comment, hash, created_at, guid, prev_hash, signature, pubkey,event_version,request_device_id,request_public_key,request_nonce,request_signature,request_hash,request_timestamp,request_path FROM history_entries ORDER BY id",
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
            }))
        }).into_iter().flatten().flatten() {
            history.push(row);
        }
    }
    if let Some(recipient_frontier) = recipient_frontier {
        retain_after_frontier(&mut history, recipient_frontier);
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
    let mut messages = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT guid,workspace_id,user_id,text,ledger_hash,created_at
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
                    "ledgerHash": row.get::<_, String>(4)?,
                    "createdAt": row.get::<_, String>(5)?,
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
        "SELECT p.guid,p.item_id,p.url,p.thumb_url,p.sha256,p.is_title
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
            }))
        }) {
            photos.extend(rows.flatten());
        }
    }
    let mut documents = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT d.guid,d.item_id,d.name,d.url,d.mime,d.sha256,d.author_id,d.access_level
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
        "messages": messages,
        "photos": photos,
        "documents": documents,
        "blobs": crate::content::manifests(conn),
        "contentCatalog": crate::content::catalog(conn),
        "contentProviders": crate::content::provider_manifest(conn),
        "accounting": crate::accounting::export(conn),
        "knowledge": crate::knowledge::export(conn),
    });
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
        "items",
        "history",
        "invites",
        "memberships",
        "custody",
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
                             unit=?14, cost=?15, comment=?16 WHERE id=?1",
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
                                it.get("comment").and_then(Value::as_str)],
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
                "INSERT INTO items (internal_id, title, status_id, responsible_user_id, workspace_id, serial_number, qr_code, due_at, guid, calibrated_until, min_quantity, quantitative, quantity, unit, cost, comment, source_system, external_id, metadata_json, created_at, organization_node_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
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
                    organization_node
                ],
            );
            items_n += 1;
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
                "INSERT OR IGNORE INTO history_entries (workspace_id,item_id,type,actor_user_id,from_label,to_label,quantity_delta,comment,hash,created_at,guid,prev_hash,signature,pubkey,event_version,request_device_id,request_public_key,request_nonce,request_signature,request_hash,request_timestamp,request_path)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
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
                    h.get("requestPath").and_then(Value::as_str)
                ],
            );
            ops += 1;
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
    if let Some(arr) = journal.get("messages").and_then(Value::as_array) {
        for message in arr {
            let guid = message.get("guid").and_then(Value::as_str).unwrap_or("");
            let ledger_hash = message
                .get("ledgerHash")
                .and_then(Value::as_str)
                .unwrap_or("");
            let text = message.get("text").and_then(Value::as_str).unwrap_or("");
            if guid.is_empty()
                || ledger_hash.is_empty()
                || text.is_empty()
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
                    "INSERT OR IGNORE INTO chat_messages(guid,workspace_id,user_id,text,ledger_hash,created_at)
                     VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        guid,
                        workspace,
                        user,
                        text,
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

pub fn list_conflicts(conn: &Connection) -> Value {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id, workspace_id, item_id, item_guid, status, description, left_label, right_label, created_at FROM conflicts ORDER BY id DESC LIMIT 200") {
        for row in stmt.query_map([], |r| {
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
    let membership_error = membership_result.as_ref().err().map(ToString::to_string);
    let healthy = database_check == "ok"
        && ledger_result.is_ok()
        && chat_result.is_ok()
        && accounting_result.is_ok()
        && knowledge_result.is_ok()
        && snapshot_result.is_ok()
        && device_result.is_ok()
        && custody_result.is_ok()
        && membership_result.is_ok()
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
    });
    json!({
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
    })
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
        }
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
