//! Журнал операций.
//!
//! Раньше здесь была локальная цепочка блоков с Ed25519-подписью и SHA-256
//! hash/prevHash. По решению заказчика криптография снята: журнал остаётся
//! обычным аудит-логом «кто, что, когда и почему», а целостность данных
//! обеспечивается сервером и транзакциями SQLite.
//!
//! Колонка `hash` сохранена как уникальный идентификатор операции: по ней
//! работает дедупликация при обмене с сервером (`INSERT OR IGNORE`) и на неё
//! опирается индекс `hist_hash_uq`. Колонки `prev_hash`, `signature` и
//! `pubkey` остаются в схеме ради старых записей, но больше не заполняются.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

/// Уникальный идентификатор операции. Используется как ключ дедупликации
/// между узлом и сервером.
pub fn new_op_id() -> String {
    uuid::Uuid::new_v4().to_string().replace('-', "")
}

#[allow(clippy::too_many_arguments)] // Границы записи журнала; меняются вместе со схемой.
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
    let op_id = new_op_id();

    conn.execute(
        "INSERT INTO history_entries (workspace_id, item_id, type, actor_user_id, from_label, to_label, quantity_delta, comment, hash, created_at, guid)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            workspace_id,
            item_id,
            op_type,
            actor_id,
            from_label,
            to_label,
            quantity_delta,
            comment,
            op_id,
            ts,
            op_id
        ],
    )?;
    let id = conn.last_insert_rowid();
    Ok(json!({
        "id": id,
        "opId": op_id,
        "workspaceId": workspace_id,
        "itemId": item_id,
        "type": op_type,
        "actorUserId": actor_id,
        "fromLabel": from_label,
        "toLabel": to_label,
        "quantityDelta": quantity_delta,
        "comment": comment,
        "createdAt": ts
    }))
}
