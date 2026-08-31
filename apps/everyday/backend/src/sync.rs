use crate::{db, json as jsn, ledger};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// Запрос немедленной синхронизации из UI. Обработчик API не может сам сходить
/// в сеть (он держит блокировку базы), поэтому просто поднимает флаг, а цикл
/// обмена подхватывает его в течение секунды.
static SYNC_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub const MAX_PEERS: i64 = 32;

fn sync_invites_enabled() -> bool {
    std::env::var("MESHKEEPER_SYNC_INVITES").as_deref() == Ok("1")
}

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
    if sync_invites_enabled() {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT token, workspace_id, role, max_uses, used_count, revoked, created_at FROM invites",
        ) {
            for row in stmt
                .query_map([], |r| {
                    let ws: i64 = r.get(1)?;
                    Ok(json!({
                        "token": r.get::<_, String>(0)?,
                        "workspaceGuid": guid_of(conn, "workspaces", ws),
                        "role": r.get::<_, String>(2)?,
                        "maxUses": r.get::<_, i64>(3)?,
                        "usedCount": r.get::<_, i64>(4)?,
                        "revoked": r.get::<_, i64>(5)? != 0,
                        "createdAt": r.get::<_, String>(6)?,
                    }))
                })
                .into_iter()
                .flatten()
                .flatten()
            {
                invites.push(row);
            }
        }
    }
    let mut memberships = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT user_id,workspace_id,rights_json,position,role_name,personnel_number FROM user_workspaces")
    {
        for row in stmt
            .query_map([], |r| {
                Ok(json!({
                    "userGuid": guid_of(conn, "users", r.get::<_, i64>(0)?),
                    "workspaceGuid": guid_of(conn, "workspaces", r.get::<_, i64>(1)?),
                    "rights": r.get::<_, Option<String>>(2)?,
                    "position": r.get::<_, Option<String>>(3)?,
                    "roleName": r.get::<_, Option<String>>(4)?,
                    "personnelNumber": r.get::<_, Option<String>>(5)?,
                }))
            })
            .into_iter()
            .flatten()
            .flatten()
        {
            memberships.push(row);
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
        "v": 1,
        "nodeId": node_id,
        "nodeName": name,
        "nodeUrl": guess_lan_base(),
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "historyMode": if recipient_frontier.is_some() { "delta" } else { "full" },
        "frontier": frontier(conn),
        "workspaces": workspaces,
        "users": users,
        "organizationNodes": organization_nodes,
        "items": items,
        "history": history,
        "invites": invites,
        "memberships": memberships,
        "messages": messages,
        "photos": photos,
        "documents": documents,
        "blobs": crate::content::manifests(conn),
        "contentCatalog": crate::content::catalog(conn),
        "contentProviders": crate::content::provider_manifest(conn),
        "accounting": crate::accounting::export(conn),
    });
    if let Err(error) = ledger::sign_journal(conn, &mut journal) {
        return json!({"ok": false, "error": format!("Не удалось подписать журнал: {error}")});
    }
    journal
}

pub fn export_journal(conn: &Connection) -> Value {
    export_journal_since(conn, None)
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
    if sync_invites_enabled() {
        if let Some(arr) = journal.get("invites").and_then(|v| v.as_array()) {
            for inv in arr {
                let token = inv.get("token").and_then(|v| v.as_str()).unwrap_or("");
                let ws_g = inv
                    .get("workspaceGuid")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let Some(ws) = id_by_guid(conn, "workspaces", ws_g) else {
                    continue;
                };
                if token.is_empty() {
                    continue;
                }
                let exists: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM invites WHERE token=?1",
                        params![token],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                if exists == 0 {
                    let _ = conn.execute(
                    "INSERT INTO invites (workspace_id, token, role, max_uses, used_count, revoked, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        ws,
                        token,
                        inv.get("role").and_then(|v| v.as_str()).unwrap_or("member"),
                        inv.get("maxUses").and_then(|v| v.as_i64()).unwrap_or(20),
                        inv.get("usedCount").and_then(|v| v.as_i64()).unwrap_or(0),
                        if inv.get("revoked").and_then(|v| v.as_bool()).unwrap_or(false) { 1 } else { 0 },
                        inv.get("createdAt").and_then(|v| v.as_str()).unwrap_or("")
                    ],
                );
                }
            }
        }
    }
    if let Some(arr) = journal.get("memberships").and_then(|v| v.as_array()) {
        for m in arr {
            let Some(user) = m
                .get("userGuid")
                .and_then(|v| v.as_str())
                .and_then(|g| id_by_guid(conn, "users", g))
            else {
                continue;
            };
            let Some(ws) = m
                .get("workspaceGuid")
                .and_then(|v| v.as_str())
                .and_then(|g| id_by_guid(conn, "workspaces", g))
            else {
                continue;
            };
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
                    params![user, ws],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if exists == 0 {
                let rights = m
                    .get("rights")
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.is_empty())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| crate::db::default_rights().to_string());
                let _ = conn.execute(
                    "INSERT INTO user_workspaces (user_id,workspace_id,rights_json,position,role_name,personnel_number) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![user,ws,rights,m.get("position").and_then(Value::as_str),m.get("roleName").and_then(Value::as_str),m.get("personnelNumber").and_then(Value::as_str)],
                );
            } else {
                let _ = conn.execute(
                    "UPDATE user_workspaces SET position=COALESCE(?1,position),role_name=COALESCE(?2,role_name),personnel_number=COALESCE(?3,personnel_number) WHERE user_id=?4 AND workspace_id=?5",
                    params![m.get("position").and_then(Value::as_str),m.get("roleName").and_then(Value::as_str),m.get("personnelNumber").and_then(Value::as_str),user,ws],
                );
            }
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
    json!({"ok": true, "url": url})
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
    if let Err(error) = enforce_node_trust(conn, journal, peer_url) {
        return json!({"ok":false,"error":format!("Ключ mesh-ноды не разрешён: {error}")});
    }
    if let Err(error) = conn.execute_batch("SAVEPOINT verified_sync") {
        return json!({"ok":false,"error":error.to_string()});
    }
    let result = import_journal(conn, journal);
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
    if let Err(error) = ledger::verify_chat_links(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Криптографическая проверка сообщений: {error}")});
    }
    if let Err(error) = crate::accounting::verify(conn) {
        let _ = conn.execute_batch("ROLLBACK TO verified_sync; RELEASE verified_sync");
        return json!({"ok":false,"error":format!("Проверка бухгалтерской летописи: {error}")});
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
    }
    result
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
    let snapshot = export_journal(conn);
    let snapshot_result = ledger::verify_journal(&snapshot);

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
    let snapshot_error = snapshot_result.as_ref().err().map(ToString::to_string);
    let healthy = database_check == "ok"
        && ledger_result.is_ok()
        && chat_result.is_ok()
        && accounting_result.is_ok()
        && snapshot_result.is_ok()
        && orphan_history == 0
        && missing_guids == 0
        && missing_blobs == 0
        && pending_downloads == 0;
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
        "snapshotError": snapshot_error,
        "snapshotHash": snapshot.get("journalHash"),
        "lastEventAt": last_event_at,
        "orphanHistory": orphan_history,
        "missingGuids": missing_guids,
        "missingBlobs": missing_blobs,
        "missingReferencedBlobs": missing_referenced_blobs,
        "pendingDownloads": pending_downloads,
        "counts": {
            "workspaces": count("workspaces"),
            "users": count("users"),
            "items": count("items"),
            "history": count("history_entries"),
            "messages": count("chat_messages"),
            "organizationNodes": count("organization_nodes"),
            "blobs": count("content_blobs"),
            "accountingTransactions": count("accounting_transactions"),
            "accountingLines": count("accounting_lines"),
        },
        "ledgerHeads": heads,
    })
}

/// Used from api.rs without making find_user_phone public — thin wrapper filled in api.
pub fn touch_peer_error(conn: &Connection, url: &str, err: &str) {
    let _ = conn.execute(
        "UPDATE peers SET last_error=?1 WHERE url=?2",
        params![err, url.trim().trim_end_matches('/')],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
