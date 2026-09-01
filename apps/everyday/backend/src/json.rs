use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};

pub fn rights_value(raw: Option<String>) -> Value {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(crate::db::default_rights)
}

pub fn user_public(conn: &Connection, id: i64) -> Option<Value> {
    conn.query_row(
        "SELECT id, full_name, position, phone, avatar_url, status, role_rights, created_at, checkout_policy, guid FROM users WHERE id=?1",
        params![id],
        |r| {
            let policy = r.get::<_, Option<String>>(8)?
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .unwrap_or_else(crate::db::default_checkout_policy);
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "fullName": r.get::<_, String>(1)?,
                "position": r.get::<_, Option<String>>(2)?,
                "phone": r.get::<_, String>(3)?,
                "avatarUrl": r.get::<_, Option<String>>(4)?,
                "status": r.get::<_, String>(5)?,
                "roleRights": rights_value(r.get::<_, Option<String>>(6)?),
                "createdAt": r.get::<_, String>(7)?,
                "checkoutPolicy": policy,
                "guid": r.get::<_, Option<String>>(9)?,
            }))
        },
    )
    .optional()
    .ok()
    .flatten()
}

fn named(conn: &Connection, table: &str, id: Option<i64>) -> Value {
    let Some(id) = id else { return Value::Null };
    let sql = format!("SELECT id, name FROM {table} WHERE id=?1");
    conn.query_row(&sql, params![id], |r| {
        Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)? }))
    })
    .unwrap_or(Value::Null)
}

fn status_obj(conn: &Connection, id: Option<i64>) -> Value {
    let Some(id) = id else { return Value::Null };
    conn.query_row(
        "SELECT id, name, description, workspace_id, type, slug, color, bg FROM statuses WHERE id=?1",
        params![id],
        |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "description": r.get::<_, Option<String>>(2)?,
                "workspaceId": r.get::<_, i64>(3)?,
                "type": r.get::<_, String>(4)?,
                "slug": r.get::<_, String>(5)?,
                "color": r.get::<_, String>(6)?,
                "bg": r.get::<_, String>(7)?,
            }))
        },
    )
    .unwrap_or(Value::Null)
}

fn photos(conn: &Connection, item_id: i64) -> Vec<Value> {
    let mut stmt = conn
        .prepare(
            "SELECT id, item_id, url, is_title, thumb_url, sha256 FROM item_photos WHERE item_id=?1",
        )
        .unwrap();
    stmt.query_map(params![item_id], |r| {
        let url: String = r.get(2)?;
        let thumb: Option<String> = r.get(4)?;
        let resolved_url = crate::content::resolve_url(conn, &url);
        let resolved_thumb = crate::content::resolve_url(conn, thumb.as_deref().unwrap_or(&url));
        Ok(json!({
            "id": r.get::<_, i64>(0)?,
            "itemId": r.get::<_, i64>(1)?,
            "url": resolved_url,
            // Старые снимки миниатюры не имеют — отдаём оригинал,
            // чтобы карточка не осталась без картинки.
            "thumbUrl": resolved_thumb,
            "sha256": r.get::<_, Option<String>>(5)?,
            "isTitle": r.get::<_, i64>(3)? != 0
        }))
    })
    .unwrap()
    .filter_map(|x| x.ok())
    .collect()
}

pub fn storage_obj(conn: &Connection, id: Option<i64>) -> Value {
    let Some(id) = id else { return Value::Null };
    conn.query_row(
        "SELECT id, name, responsible_user_id, workspace_id, address, guid FROM storages WHERE id=?1",
        params![id],
        |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "responsibleUserId": r.get::<_, Option<i64>>(2)?,
                "workspaceId": r.get::<_, i64>(3)?,
                "address": r.get::<_, Option<String>>(4)?,
                "guid": r.get::<_, Option<String>>(5)?,
            }))
        },
    )
    .unwrap_or(Value::Null)
}

/// Убирает из карточки оригиналы снимков, оставляя миниатюры.
///
/// В списке каталога полноразмерные фото не нужны, а весят они всё: без этого
/// один запрос списка тянет десятки мегабайт data-URL.
pub fn strip_full_photos(value: &mut Value) {
    let Some(list) = value.get_mut("photos").and_then(Value::as_array_mut) else {
        return;
    };
    for photo in list {
        let thumb = photo.get("thumbUrl").cloned();
        if let (Value::Object(map), Some(thumb)) = (photo, thumb) {
            map.insert("url".into(), thumb);
        }
    }
}

pub fn item_json(conn: &Connection, id: i64, with_history: bool) -> Option<Value> {
    let mut base: Map<String, Value> = conn
        .query_row(
            "SELECT id, internal_id, title, category_id, brand_id, status_id, responsible_user_id,
                    building_site_id, storage_id, workspace_id, serial_number, cost, quantitative,
                    quantity, unit, comment, qr_code, notify_date, created_at, due_at,
                    guid, calibrated_until, min_quantity, source_system, external_id, metadata_json,
                    organization_node_id
             FROM items WHERE id=?1",
            params![id],
            |r| {
                let category_id: Option<i64> = r.get(3)?;
                let brand_id: Option<i64> = r.get(4)?;
                let status_id: Option<i64> = r.get(5)?;
                let resp: Option<i64> = r.get(6)?;
                let site: Option<i64> = r.get(7)?;
                let storage: Option<i64> = r.get(8)?;
                let organization_node: Option<i64> = r.get(26)?;
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "internalId": r.get::<_, String>(1)?,
                    "title": r.get::<_, String>(2)?,
                    "categoryId": category_id,
                    "brandId": brand_id,
                    "statusId": status_id,
                    "responsibleUserId": resp,
                    "buildingSiteId": site,
                    "storageId": storage,
                    "organizationNodeId": organization_node,
                    "workspaceId": r.get::<_, i64>(9)?,
                    "serialNumber": r.get::<_, Option<String>>(10)?,
                    "cost": r.get::<_, Option<f64>>(11)?,
                    "quantitative": r.get::<_, i64>(12)? != 0,
                    "quantity": r.get::<_, Option<f64>>(13)?,
                    "unit": r.get::<_, Option<String>>(14)?,
                    "comment": r.get::<_, Option<String>>(15)?,
                    "qrCode": r.get::<_, Option<String>>(16)?,
                    "notifyDate": r.get::<_, Option<String>>(17)?,
                    "createdAt": r.get::<_, String>(18)?,
                    "dueAt": r.get::<_, Option<String>>(19)?,
                    "guid": r.get::<_, Option<String>>(20)?,
                    "calibratedUntil": r.get::<_, Option<String>>(21)?,
                    "minQuantity": r.get::<_, Option<f64>>(22)?,
                    "sourceSystem": r.get::<_, Option<String>>(23)?,
                    "externalId": r.get::<_, Option<String>>(24)?,
                    "metadata": r.get::<_, Option<String>>(25)?
                        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                        .unwrap_or_else(|| json!({})),
                    "category": named(conn, "categories", category_id),
                    "brand": named(conn, "brands", brand_id),
                    "status": status_obj(conn, status_id),
                    "responsible": resp.and_then(|i| user_public(conn, i)).unwrap_or(Value::Null),
                    "buildingSite": named(conn, "building_sites", site),
                    "storage": storage_obj(conn, storage),
                    "organizationNode": organization_node.and_then(|node_id| conn.query_row(
                        "SELECT id,guid,kind,name,parent_id,tab_label FROM organization_nodes WHERE id=?1",
                        [node_id],
                        |node| Ok(json!({
                            "id": node.get::<_,i64>(0)?, "guid": node.get::<_,String>(1)?,
                            "kind": node.get::<_,String>(2)?, "name": node.get::<_,String>(3)?,
                            "parentId": node.get::<_,Option<i64>>(4)?,
                            "tabLabel": node.get::<_,Option<String>>(5)?,
                        })),
                    ).ok()).unwrap_or(Value::Null),
                    "photos": photos(conn, r.get(0)?),
                }))
            },
        )
        .ok()?
        .as_object()
        .cloned()?;

    attach_stock_and_holders(conn, id, &mut base);
    if with_history {
        base.insert("history".into(), Value::Array(item_history(conn, id)));
        base.insert("documents".into(), Value::Array(item_docs(conn, id)));
        base.insert("comments".into(), Value::Array(item_comments(conn, id)));
    }
    Some(Value::Object(base))
}

fn attach_stock_and_holders(conn: &Connection, id: i64, base: &mut Map<String, Value>) {
    let quantitative = base
        .get("quantitative")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut holders = Vec::new();
    let mut issued_qty = 0.0;
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, user_id, quantity, due_at, created_at FROM item_holdings WHERE item_id=?1 AND returned_at IS NULL ORDER BY id DESC",
    ) {
        if let Ok(rows) = stmt.query_map(params![id], |r| {
            let uid: i64 = r.get(1)?;
            let q: f64 = r.get(2)?;
            Ok((uid, q, json!({
                "id": r.get::<_, i64>(0)?,
                "userId": uid,
                "quantity": q,
                "dueAt": r.get::<_, Option<String>>(3)?,
                "createdAt": r.get::<_, String>(4)?,
                "user": user_public(conn, uid).unwrap_or(Value::Null),
            })))
        }) {
            for row in rows.flatten() {
                issued_qty += row.1;
                holders.push(row.2);
            }
        }
    }
    if quantitative {
        let stock = base.get("quantity").and_then(|v| v.as_f64()).unwrap_or(0.0);
        base.insert("stockQty".into(), json!(stock));
        base.insert("issuedQty".into(), json!(issued_qty));
        base.insert("totalQty".into(), json!(stock + issued_qty));
    } else if let Some(resp) = base.get("responsible").cloned() {
        if !resp.is_null() {
            holders.insert(
                0,
                json!({
                    "id": 0,
                    "userId": base.get("responsibleUserId").cloned(),
                    "quantity": 1,
                    "dueAt": base.get("dueAt").cloned(),
                    "createdAt": base.get("createdAt").cloned(),
                    "user": resp,
                    "internalId": base.get("internalId").cloned(),
                }),
            );
        }
    }

    let ws = base.get("workspaceId").and_then(|v| v.as_i64());
    let title = base
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let mut family_total = 0i64;
    let mut family_stock = 0i64;
    let mut family_issued = 0i64;
    let mut members = Vec::new();
    if let (Some(ws), Some(title)) = (ws, title) {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, internal_id, responsible_user_id, status_id FROM items WHERE workspace_id=?1 AND title=?2 AND archived=0 ORDER BY id",
        ) {
            if let Ok(rows) = stmt.query_map(params![ws, title], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            }) {
                for (sid, vn, resp, st) in rows.flatten() {
                    family_total += 1;
                    if resp.is_some() { family_issued += 1; } else { family_stock += 1; }
                    members.push(json!({
                        "id": sid,
                        "internalId": vn,
                        "responsibleUserId": resp,
                        "responsible": resp.and_then(|u| user_public(conn, u)),
                        "inStock": resp.is_none(),
                        "status": status_obj(conn, st),
                    }));
                    if sid != id {
                        if let Some(uid) = resp {
                            holders.push(json!({
                                "id": sid,
                                "userId": uid,
                                "quantity": 1,
                                "internalId": vn,
                                "user": user_public(conn, uid).unwrap_or(Value::Null),
                            }));
                        }
                    }
                }
            }
        }
    }
    base.insert("holders".into(), Value::Array(holders));
    base.insert(
        "family".into(),
        json!({
            "total": family_total,
            "inStock": family_stock,
            "issued": family_issued,
            "members": members,
        }),
    );
    if !quantitative {
        base.insert("stockQty".into(), json!(family_stock));
        base.insert("issuedQty".into(), json!(family_issued));
        base.insert("totalQty".into(), json!(family_total));
    }
}

fn item_docs(conn: &Connection, item_id: i64) -> Vec<Value> {
    let mut stmt = conn
        .prepare("SELECT id,item_id,name,url,guid,mime,sha256,author_id,access_level FROM item_documents WHERE item_id=?1")
        .unwrap();
    stmt.query_map(params![item_id], |r| {
        let url: String = r.get(3)?;
        Ok(json!({
            "id": r.get::<_, i64>(0)?,
            "itemId": r.get::<_, i64>(1)?,
            "name": r.get::<_, String>(2)?,
            "url": crate::content::resolve_url(conn, &url),
            "guid": r.get::<_, Option<String>>(4)?,
            "mime": r.get::<_, Option<String>>(5)?,
            "sha256": r.get::<_, Option<String>>(6)?,
            "authorId": r.get::<_, Option<i64>>(7)?,
            "accessLevel": r.get::<_, String>(8)?,
        }))
    })
    .unwrap()
    .filter_map(|x| x.ok())
    .collect()
}

fn item_comments(conn: &Connection, item_id: i64) -> Vec<Value> {
    let mut stmt = conn
        .prepare("SELECT id, item_id, user_id, text, active, created_at FROM item_comments WHERE item_id=?1 AND active=1 ORDER BY id DESC")
        .unwrap();
    stmt.query_map(params![item_id], |r| {
        let uid: i64 = r.get(2)?;
        Ok(json!({
            "id": r.get::<_, i64>(0)?,
            "itemId": r.get::<_, i64>(1)?,
            "userId": uid,
            "text": r.get::<_, String>(3)?,
            "active": r.get::<_, i64>(4)? != 0,
            "createdAt": r.get::<_, String>(5)?,
            "user": user_public(conn, uid).unwrap_or(Value::Null)
        }))
    })
    .unwrap()
    .filter_map(|x| x.ok())
    .collect()
}

pub fn item_history(conn: &Connection, item_id: i64) -> Vec<Value> {
    let mut stmt = conn
        .prepare(
            "SELECT id, workspace_id, item_id, type, actor_user_id, from_label, to_label, quantity_delta, comment, hash, created_at, photo_url,event_version,request_device_id,request_nonce,request_hash
             FROM history_entries WHERE item_id=?1 ORDER BY id DESC LIMIT 500",
        )
        .unwrap();
    stmt.query_map(params![item_id], |r| {
        let actor: i64 = r.get(4)?;
        let item_id: Option<i64> = r.get(2)?;
        Ok(json!({
            "id": r.get::<_, i64>(0)?,
            "workspaceId": r.get::<_, i64>(1)?,
            "itemId": item_id,
            "type": r.get::<_, String>(3)?,
            "actorUserId": actor,
            "fromLabel": r.get::<_, Option<String>>(5)?,
            "toLabel": r.get::<_, Option<String>>(6)?,
            "quantityDelta": r.get::<_, Option<f64>>(7)?,
            "comment": r.get::<_, Option<String>>(8)?,
            "opId": r.get::<_, String>(9)?,
            "createdAt": r.get::<_, String>(10)?,
            "photoUrl": r.get::<_, Option<String>>(11)?,
            "eventVersion": r.get::<_, i64>(12)?,
            "requestDeviceId": r.get::<_, Option<String>>(13)?,
            "requestNonce": r.get::<_, Option<String>>(14)?,
            "requestHash": r.get::<_, Option<String>>(15)?,
            "actor": user_public(conn, actor).unwrap_or(Value::Null),
            "item": item_id.and_then(|i| item_json(conn, i, false)).unwrap_or(Value::Null)
        }))
    })
    .unwrap()
    .filter_map(|x| x.ok())
    .collect()
}

pub fn transfer_json(conn: &Connection, id: i64) -> Option<Value> {
    conn.query_row(
        "SELECT id, code, item_id, from_user_id, to_user_id, to_storage_id, building_site_id, workspace_id, quantity, status, photo_url, comment, no_confirmation, created_at, completed_at, bit_amount, bit_transaction_guid, guid
         FROM transfers WHERE id=?1",
        params![id],
        |r| {
            let item_id: i64 = r.get(2)?;
            let from_id: i64 = r.get(3)?;
            let to_id: i64 = r.get(4)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "code": r.get::<_, Option<String>>(1)?,
                "itemId": item_id,
                "fromUserId": from_id,
                "toUserId": to_id,
                "toStorageId": r.get::<_, Option<i64>>(5)?,
                "buildingSiteId": r.get::<_, Option<i64>>(6)?,
                "workspaceId": r.get::<_, i64>(7)?,
                "quantity": r.get::<_, Option<f64>>(8)?,
                "status": r.get::<_, String>(9)?,
                "photoUrl": r.get::<_, Option<String>>(10)?,
                "comment": r.get::<_, Option<String>>(11)?,
                "noConfirmation": r.get::<_, i64>(12)? != 0,
                "createdAt": r.get::<_, String>(13)?,
                "completedAt": r.get::<_, Option<String>>(14)?,
                "bitAmount": r.get::<_, Option<i64>>(15)?,
                "bitTransactionGuid": r.get::<_, Option<String>>(16)?,
                "guid": r.get::<_, Option<String>>(17)?,
                "item": item_json(conn, item_id, false).unwrap_or(Value::Null),
                "fromUser": user_public(conn, from_id).unwrap_or(Value::Null),
                "toUser": user_public(conn, to_id).unwrap_or(Value::Null),
                "toStorage": storage_obj(conn, r.get(5)?),
            }))
        },
    )
    .ok()
}

pub fn workspace_json(conn: &Connection, id: i64) -> Option<Value> {
    let _ = conn.execute(
        "UPDATE workspaces SET guid=?1 WHERE id=?2 AND (guid IS NULL OR guid='')",
        params![uuid::Uuid::new_v4().to_string(), id],
    );
    conn.query_row(
        "SELECT id, name, timezone, internal_id_prefix, comment, created_at, sync_url, require_writeoff_photo, guid FROM workspaces WHERE id=?1",
        params![id],
        |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "timezone": r.get::<_, String>(2)?,
                "internalIdPrefix": r.get::<_, String>(3)?,
                "comment": r.get::<_, Option<String>>(4)?,
                "createdAt": r.get::<_, String>(5)?,
                "syncUrl": r.get::<_, Option<String>>(6)?,
                "requireWriteoffPhoto": r.get::<_, i64>(7)? != 0,
                "guid": r.get::<_, Option<String>>(8)?,
            }))
        },
    )
    .ok()
}

pub fn default_ws(conn: &Connection) -> i64 {
    conn.query_row("SELECT id FROM workspaces ORDER BY id LIMIT 1", [], |r| {
        r.get(0)
    })
    .unwrap_or(1)
}
