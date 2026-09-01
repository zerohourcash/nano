use crate::json as jsn;
use crate::{db, ledger};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use uuid::Uuid;

thread_local! {
    static CURRENT_UID: Cell<Option<i64>> = const { Cell::new(None) };
}

fn ws_fallback(conn: &Connection) -> i64 {
    CURRENT_UID.with(|c| {
        if let Some(uid) = c.get() {
            if let Ok(id) = conn.query_row(
                "SELECT workspace_id FROM user_workspaces WHERE user_id=?1 ORDER BY id DESC LIMIT 1",
                params![uid],
                |r| r.get(0),
            ) {
                return id;
            }
        }
        jsn::default_ws(conn)
    })
}

#[derive(Debug)]
pub struct ApiError {
    pub message: String,
    pub code: &'static str,
    pub http: u16,
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("BAD_REQUEST", 400, e.to_string())
    }
}

impl ApiError {
    pub fn new(code: &'static str, http: u16, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code,
            http,
        }
    }
    fn unauth(m: impl Into<String>) -> Self {
        Self::new("UNAUTHORIZED", 401, m)
    }
    fn not_found(m: impl Into<String>) -> Self {
        Self::new("NOT_FOUND", 404, m)
    }
    fn bad(m: impl Into<String>) -> Self {
        Self::new("BAD_REQUEST", 400, m)
    }
    fn conflict(m: impl Into<String>) -> Self {
        Self::new("CONFLICT", 409, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new("INTERNAL_SERVER_ERROR", 500, m)
    }
}

type ApiResult = Result<Value, ApiError>;

pub fn is_mutation(procedure: &str) -> bool {
    !matches!(
        procedure,
        "ping"
            | "auth.directory"
            | "auth.options"
            | "auth.me"
            | "auth.devices"
            | "auth.inviteInfo"
            | "meta.currentUser"
            | "meta.transferCounts"
            | "meta.workspaces"
            | "items.list"
            | "items.byId"
            | "items.byCode"
            | "items.nextInternalId"
            | "items.faults"
            | "items.changeRequests"
            | "chat.list"
            | "sync.status"
            | "sync.audit"
            | "sync.peers"
            | "sync.conflicts"
            | "sync.nodeKeys"
            | "sync.diagnostics"
            | "sync.exportBundle"
            | "content.status"
            | "bit.balance"
            | "bit.transactions"
            | "knowledge.list"
            | "knowledge.bySlug"
            | "transfers.outgoing"
            | "transfers.incoming"
            | "transfers.byId"
            | "history.movements"
            | "history.quantityOps"
            | "history.all"
            | "inventory.sessions"
            | "inventory.byId"
            | "inventory.results"
            | "notifications.list"
            | "notifications.unreadCount"
            | "reports.byUsers"
            | "reports.quantityTransactions"
            | "reports.allItems"
            | "profile.get"
            | "admin.users.list"
            | "admin.users.defaultRights"
            | "admin.workspaces.list"
            | "admin.workspaces.invites"
            | "admin.storages.list"
            | "admin.buildingSites.list"
            | "admin.organizationNodes.list"
            | "admin.dictionaries.list"
    )
}

fn atomic<T>(
    conn: &mut Connection,
    operation: impl FnOnce(&Connection) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let tx = conn.transaction()?;
    let result = operation(&tx)?;
    tx.commit()?;
    Ok(result)
}

fn g<'a>(input: &'a Value, key: &str) -> &'a Value {
    input.get(key).unwrap_or(&Value::Null)
}
fn s(input: &Value, key: &str) -> Option<String> {
    g(input, key)
        .as_str()
        .map(|x| x.to_string())
        .filter(|x| !x.is_empty())
}
fn i64v(input: &Value, key: &str) -> Option<i64> {
    g(input, key)
        .as_i64()
        .or_else(|| g(input, key).as_u64().map(|x| x as i64))
        .or_else(|| g(input, key).as_f64().map(|x| x as i64))
}
fn f64v(input: &Value, key: &str) -> Option<f64> {
    g(input, key)
        .as_f64()
        .or_else(|| g(input, key).as_i64().map(|x| x as f64))
}
fn b(input: &Value, key: &str) -> Option<bool> {
    g(input, key).as_bool()
}

/// Расширенные поля интеграций хранятся как JSON-объект. Ограничение не даёт
/// превратить карточку ТМЦ в неограниченное файловое хранилище.
fn item_metadata(input: &Value) -> Result<Option<String>, ApiError> {
    let Some(value) = input.get("metadata") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if !value.is_object() {
        return Err(ApiError::bad("metadata должно быть JSON-объектом"));
    }
    let raw = serde_json::to_string(value)
        .map_err(|_| ApiError::bad("Некорректные дополнительные данные"))?;
    if raw.len() > 64 * 1024 {
        return Err(ApiError::bad("Дополнительные данные превышают 64 КБ"));
    }
    Ok(Some(raw))
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn hash_password(password: &str) -> String {
    let salt_raw = rand::random::<[u8; 16]>();
    let salt = SaltString::encode_b64(&salt_raw).expect("valid salt length");
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("Argon2 password hashing")
        .to_string()
}
fn verify_password(password: &str, stored: &str) -> bool {
    if stored.starts_with("$argon2") {
        return PasswordHash::new(stored).ok().is_some_and(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        });
    }
    let Some((salt, digest)) = stored.split_once('$') else {
        return false;
    };
    let mut h = Sha256::new();
    h.update(format!("{salt}:{password}"));
    hex::encode(h.finalize()) == digest
}

fn validate_new_password(password: &str) -> Result<(), ApiError> {
    let length = password.chars().count();
    if !(12..=128).contains(&length) {
        return Err(ApiError::bad(
            "Пароль должен содержать от 12 до 128 символов",
        ));
    }
    Ok(())
}

fn validate_phone(phone: &str) -> Result<(), ApiError> {
    let digits = db::digits_only(phone);
    if !(7..=20).contains(&digits.len()) {
        return Err(ApiError::bad("Некорректный номер телефона"));
    }
    Ok(())
}

fn find_user_phone(conn: &Connection, phone: &str) -> Option<i64> {
    let want = db::digits_only(phone);
    let mut stmt = conn.prepare("SELECT id, phone FROM users").ok()?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .ok()?
        .filter_map(|x| x.ok())
        .collect();
    rows.into_iter()
        .find(|(_, p)| db::digits_only(p) == want)
        .map(|(id, _)| id)
}

fn require_user(conn: &Connection, user_id: Option<i64>) -> Result<i64, ApiError> {
    let Some(id) = user_id else {
        return Err(ApiError::unauth("Войдите в систему"));
    };
    let status: Option<String> = conn
        .query_row("SELECT status FROM users WHERE id=?1", params![id], |r| {
            r.get(0)
        })
        .optional()
        .ok()
        .flatten();
    match status.as_deref() {
        Some("disabled") => Err(ApiError::unauth("Аккаунт заблокирован")),
        Some(_) => Ok(id),
        None => Err(ApiError::unauth("Пользователь не найден")),
    }
}

fn user_can(conn: &Connection, uid: i64, key: &str) -> bool {
    let mut rights = db::default_rights();
    if let Some(stored) = jsn::user_public(conn, uid).and_then(|u| u.get("roleRights").cloned()) {
        if let (Value::Object(ref mut dest), Value::Object(src)) = (&mut rights, stored) {
            for (k, v) in src {
                dest.insert(k, v);
            }
        }
    }
    rights.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn require_can(conn: &Connection, uid: i64, key: &str) -> Result<(), ApiError> {
    if user_can(conn, uid, key) {
        Ok(())
    } else {
        Err(ApiError::new(
            "FORBIDDEN",
            403,
            "Недостаточно прав для этого действия",
        ))
    }
}

/// Права пользователя в пространстве, наложенные на набор по умолчанию.
///
/// Наложение обязательно: у записей, созданных до появления нового права,
/// ключа просто нет, и без слияния такой пользователь потерял бы доступ,
/// которого у него никто не отбирал.
fn merged_rights(conn: &Connection, uid: i64, ws: i64) -> Value {
    let raw: Option<String> = conn.query_row(
        "SELECT COALESCE(uw.rights_json,u.role_rights) FROM user_workspaces uw JOIN users u ON u.id=uw.user_id WHERE uw.user_id=?1 AND uw.workspace_id=?2",
        params![uid, ws], |r| r.get(0),
    ).optional().ok().flatten().flatten();
    let mut rights = db::default_rights();
    if let Some(stored) = raw.and_then(|v| serde_json::from_str::<Value>(&v).ok()) {
        if let (Value::Object(dest), Value::Object(src)) = (&mut rights, stored) {
            for (k, v) in src {
                dest.insert(k, v);
            }
        }
    }
    rights
}

fn require_can_in_workspace(
    conn: &Connection,
    uid: i64,
    ws: i64,
    key: &str,
) -> Result<(), ApiError> {
    let rights = merged_rights(conn, uid, ws);
    if rights.get(key).and_then(Value::as_bool).unwrap_or(false) {
        Ok(())
    } else {
        Err(ApiError::new(
            "FORBIDDEN",
            403,
            "Недостаточно прав в этом рабочем пространстве",
        ))
    }
}

/// Пространство пользователя по умолчанию — то же, которое подставит `ws_fallback`.
fn own_workspace(conn: &Connection, uid: i64) -> Option<i64> {
    conn.query_row(
        "SELECT workspace_id FROM user_workspaces WHERE user_id=?1 ORDER BY id DESC LIMIT 1",
        params![uid],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn require_member(conn: &Connection, uid: i64, workspace_id: i64) -> Result<(), ApiError> {
    let member: bool = conn
        .query_row(
            "SELECT 1 FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
            params![uid, workspace_id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if member {
        Ok(())
    } else {
        Err(ApiError::new(
            "FORBIDDEN",
            403,
            "Нет доступа к рабочему пространству",
        ))
    }
}

fn require_item_access(conn: &Connection, uid: i64, item_id: i64) -> Result<i64, ApiError> {
    let ws = conn
        .query_row(
            "SELECT workspace_id FROM items WHERE id=?1 AND archived=0",
            params![item_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    require_member(conn, uid, ws)?;
    Ok(ws)
}

fn validate_item_references(conn: &Connection, input: &Value, ws: i64) -> Result<(), ApiError> {
    for (key, table) in [
        ("categoryId", "categories"),
        ("brandId", "brands"),
        ("statusId", "statuses"),
        ("storageId", "storages"),
        ("buildingSiteId", "building_sites"),
        ("organizationNodeId", "organization_nodes"),
    ] {
        if input.get(key).is_some() {
            if let Some(id) = i64v(input, key) {
                let sql = format!("SELECT COUNT(*) FROM {table} WHERE id=?1 AND workspace_id=?2");
                let found: i64 = conn.query_row(&sql, params![id, ws], |r| r.get(0))?;
                if found == 0 {
                    return Err(ApiError::bad(format!(
                        "{key} относится к другому рабочему пространству"
                    )));
                }
            }
        }
    }
    if input.get("responsibleUserId").is_some() {
        if let Some(user) = i64v(input, "responsibleUserId") {
            require_member(conn, user, ws)
                .map_err(|_| ApiError::bad("Ответственный не состоит в рабочем пространстве"))?;
        }
    }
    Ok(())
}

/// Статусы, перевод в которые требует явной причины (ТЗ §8).
const STATUSES_REQUIRING_REASON: [&str; 4] = ["in-repair", "needs-check", "written-off", "broken"];

fn status_label(conn: &Connection, status_id: Option<i64>) -> (Option<String>, Option<String>) {
    let Some(id) = status_id else {
        return (None, None);
    };
    conn.query_row(
        "SELECT name, slug FROM statuses WHERE id=?1",
        params![id],
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        },
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or((None, None))
}

/// Причина перевода в «неисправен», «на ремонте» или «списан».
/// Возвращает текст причины, если он обязателен и указан.
fn status_change_reason(
    conn: &Connection,
    input: &Value,
    before_status: Option<i64>,
    next_status: Option<i64>,
) -> Result<Option<String>, ApiError> {
    if before_status == next_status {
        return Ok(None);
    }
    let (name, slug) = status_label(conn, next_status);
    let slug = slug.unwrap_or_default();
    if !STATUSES_REQUIRING_REASON.contains(&slug.as_str()) {
        return Ok(None);
    }
    let reason = s(input, "reason").or_else(|| s(input, "comment"));
    match reason {
        Some(text) if text.trim().chars().count() >= 3 => Ok(Some(text)),
        _ => Err(ApiError::bad(format!(
            "Укажите причину перевода в статус «{}»",
            name.unwrap_or(slug)
        ))),
    }
}

fn required_admin_right(procedure: &str) -> Option<&'static str> {
    if procedure.starts_with("admin.users.") {
        Some("manageUsers")
    } else if procedure.starts_with("admin.workspaces.") {
        Some("manageWorkspaces")
    } else if procedure.starts_with("admin.storages.") {
        Some("manageStorages")
    } else if procedure.starts_with("admin.buildingSites.") {
        Some("manageSites")
    } else if procedure.starts_with("admin.organizationNodes.") {
        Some("manageWorkspaces")
    } else if procedure.starts_with("admin.dictionaries.") {
        Some("manageDictionaries")
    } else if procedure.starts_with("sync.")
        || procedure.starts_with("backup.")
        || procedure.starts_with("content.")
    {
        Some("manageWorkspaces")
    } else {
        None
    }
}

fn required_right(procedure: &str) -> Option<&'static str> {
    if let Some(right) = required_admin_right(procedure) {
        return Some(right);
    }
    if matches!(procedure, "items.create") {
        Some("createItems")
    } else if matches!(procedure, "items.remove") {
        Some("deleteItems")
    } else if matches!(
        procedure,
        "items.update"
            | "items.addPhoto"
            | "history.move"
            | "items.resolveFault"
            | "items.decideChange"
    ) {
        Some("editItems")
    } else if matches!(procedure, "items.addDocument") {
        Some("manageDocuments")
    } else if matches!(procedure, "items.reportFault") {
        Some("reportFaults")
    } else if matches!(procedure, "items.requestChange") {
        Some("requestChanges")
    } else if procedure.starts_with("items.") {
        Some("viewItems")
    } else if procedure == "bit.mint" {
        Some("manageAccounting")
    } else if matches!(procedure, "bit.transfer" | "bit.sale" | "bit.balance") {
        Some("useBit")
    } else if procedure == "bit.transactions" {
        Some("viewAccounting")
    } else if procedure == "knowledge.save" {
        Some("editKnowledge")
    } else if procedure.starts_with("knowledge.") {
        Some("viewKnowledge")
    } else if matches!(
        procedure,
        "transfers.accept" | "transfers.reject" | "transfers.acceptAll"
    ) {
        Some("acceptTransfers")
    } else if procedure.starts_with("transfers.") {
        Some("transferItems")
    } else if matches!(procedure, "history.writeOff") {
        Some("writeOff")
    } else if matches!(procedure, "history.replenish") {
        Some("replenish")
    } else if procedure.starts_with("history.") {
        Some("viewHistory")
    } else if procedure.starts_with("inventory.") {
        Some("inventory")
    } else if procedure.starts_with("reports.") {
        Some("viewReports")
    } else {
        None
    }
}

fn target_workspace(
    conn: &Connection,
    procedure: &str,
    input: &Value,
) -> Result<Option<i64>, ApiError> {
    if let Some(ws) = i64v(input, "workspaceId") {
        return Ok(Some(ws));
    }
    if let Some(item_id) = i64v(input, "itemId") {
        return Ok(conn
            .query_row(
                "SELECT workspace_id FROM items WHERE id=?1",
                params![item_id],
                |r| r.get(0),
            )
            .optional()?);
    }
    let id = i64v(input, "id").or_else(|| i64v(input, "sessionId"));
    let Some(id) = id else { return Ok(None) };
    let table = if matches!(procedure, "items.resolveFault") {
        Some("faults")
    } else if matches!(procedure, "items.decideChange") {
        Some("change_requests")
    } else if procedure.starts_with("items.") {
        Some("items")
    } else if procedure.starts_with("transfers.") {
        Some("transfers")
    } else if procedure.starts_with("inventory.") {
        Some("inventory_sessions")
    } else if matches!(
        procedure,
        "admin.workspaces.update" | "admin.workspaces.remove"
    ) {
        return Ok(Some(id));
    } else if procedure.starts_with("admin.storages.") {
        Some("storages")
    } else if procedure.starts_with("admin.buildingSites.") {
        Some("building_sites")
    } else if procedure.starts_with("admin.organizationNodes.") {
        Some("organization_nodes")
    } else {
        None
    };
    if procedure.starts_with("admin.dictionaries.")
        && !matches!(
            procedure,
            "admin.dictionaries.list" | "admin.dictionaries.create"
        )
    {
        let kind = s(input, "kind").unwrap_or_else(|| "categories".into());
        let table = dict_table(&kind)?;
        let sql = format!("SELECT workspace_id FROM {table} WHERE id=?1");
        return Ok(conn.query_row(&sql, params![id], |r| r.get(0)).optional()?);
    }
    let Some(table) = table else { return Ok(None) };
    let sql = format!("SELECT workspace_id FROM {table} WHERE id=?1");
    Ok(conn.query_row(&sql, params![id], |r| r.get(0)).optional()?)
}

fn require_shared_workspace(
    conn: &Connection,
    actor: i64,
    target_user: i64,
) -> Result<(), ApiError> {
    let shared = conn.query_row(
        "SELECT 1 FROM user_workspaces a JOIN user_workspaces b ON b.workspace_id=a.workspace_id
         WHERE a.user_id=?1 AND b.user_id=?2 LIMIT 1",
        params![actor, target_user], |_| Ok(true),
    ).optional()?.unwrap_or(false);
    if shared {
        Ok(())
    } else {
        Err(ApiError::new(
            "FORBIDDEN",
            403,
            "Пользователь из другого рабочего пространства",
        ))
    }
}

pub fn dispatch(
    conn: &mut Connection,
    procedure: &str,
    input: &Value,
    user_id: Option<i64>,
) -> ApiResult {
    CURRENT_UID.with(|c| c.set(user_id));
    let public = matches!(
        procedure,
        "ping"
            | "auth.login"
            | "auth.register"
            | "auth.joinRegister"
            | "auth.inviteInfo"
            | "auth.logout"
            | "auth.options"
    ) || (procedure == "auth.directory"
        && std::env::var("MESHKEEPER_DEMO_LOGIN").as_deref() == Ok("1"));
    if !public {
        let uid = require_user(conn, user_id)?;
        let target_ws = target_workspace(conn, procedure, input)?;
        // Если пространство в запросе не указано, обработчик всё равно возьмёт
        // пространство пользователя по умолчанию — права проверяем там же,
        // иначе проверку можно было бы обойти, просто не передав workspaceId.
        let effective_ws = match target_ws {
            Some(ws) => {
                require_member(conn, uid, ws)?;
                Some(ws)
            }
            None => own_workspace(conn, uid),
        };
        if let Some(right) = required_right(procedure) {
            match effective_ws {
                Some(ws) => require_can_in_workspace(conn, uid, ws, right)?,
                None => require_can(conn, uid, right)?,
            }
        }
        if matches!(procedure, "admin.users.update" | "admin.users.remove") {
            let target = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
            require_shared_workspace(conn, uid, target)?;
        }
    }
    // Обработчик выполняется ровно один раз: повторный вызов создавал бы
    // дубли на мутациях.
    let mut value = dispatch_inner(conn, procedure, input, user_id)?;
    // Фото и местонахождение — отдельные права (ТЗ §4). Прячем их в ответе,
    // а не в каждом обработчике: карточка предмета встречается вложенной
    // в историю, заявки, отчёты и передачи.
    if let Some(uid) = user_id {
        if let Some(ws) = target_workspace(conn, procedure, input)
            .ok()
            .flatten()
            .or_else(|| own_workspace(conn, uid))
        {
            let rights = merged_rights(conn, uid, ws);
            let hide_photos = !rights
                .get("viewPhotos")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let hide_location = !rights
                .get("viewLocation")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let hide_documents = !rights
                .get("viewDocuments")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let hide_accounting_documents = !rights
                .get("viewAccounting")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let hide_manager_documents = !rights
                .get("manageDocuments")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if hide_photos
                || hide_location
                || hide_documents
                || hide_accounting_documents
                || hide_manager_documents
            {
                redact_item_fields(
                    &mut value,
                    hide_photos,
                    hide_location,
                    hide_documents,
                    hide_accounting_documents,
                    hide_manager_documents,
                );
            }
        }
    }
    Ok(value)
}

/// Рекурсивно вычищает из ответа поля карточки, закрытые правами.
fn redact_item_fields(
    value: &mut Value,
    hide_photos: bool,
    hide_location: bool,
    hide_documents: bool,
    hide_accounting_documents: bool,
    hide_manager_documents: bool,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                redact_item_fields(
                    item,
                    hide_photos,
                    hide_location,
                    hide_documents,
                    hide_accounting_documents,
                    hide_manager_documents,
                );
            }
        }
        Value::Object(map) => {
            // Признак карточки предмета: у неё есть внутренний номер и название.
            let is_item = map.contains_key("internalId") && map.contains_key("title");
            if is_item {
                if hide_photos {
                    map.remove("photos");
                    map.remove("photoUrl");
                }
                if hide_location {
                    for key in [
                        "storage",
                        "storageId",
                        "buildingSite",
                        "buildingSiteId",
                        "organizationNode",
                        "organizationNodeId",
                    ] {
                        map.remove(key);
                    }
                }
                if hide_documents {
                    map.remove("documents");
                } else if let Some(documents) =
                    map.get_mut("documents").and_then(Value::as_array_mut)
                {
                    documents.retain(|document| {
                        match document
                            .get("accessLevel")
                            .and_then(Value::as_str)
                            .unwrap_or("members")
                        {
                            "accounting" => !hide_accounting_documents,
                            "managers" => !hide_manager_documents,
                            _ => true,
                        }
                    });
                }
            }
            for (_, nested) in map.iter_mut() {
                redact_item_fields(
                    nested,
                    hide_photos,
                    hide_location,
                    hide_documents,
                    hide_accounting_documents,
                    hide_manager_documents,
                );
            }
        }
        _ => {}
    }
}

fn dispatch_inner(
    conn: &mut Connection,
    procedure: &str,
    input: &Value,
    user_id: Option<i64>,
) -> ApiResult {
    match procedure {
        "ping" => Ok(json!({"ok": true, "ts": chrono::Utc::now().timestamp_millis()})),
        "auth.directory" => {
            if std::env::var("MESHKEEPER_DEMO_LOGIN").as_deref() == Ok("1") {
                auth_directory(conn)
            } else {
                Err(ApiError::not_found("Процедура отключена"))
            }
        }
        "auth.options" => auth_options(conn),
        "auth.login" => auth_login(conn, input),
        "auth.register" => auth_register(conn, input),
        "auth.join" => auth_join(conn, input, user_id),
        "auth.joinRegister" => auth_join_register(conn, input),
        "auth.logout" => Ok(json!({"ok": true})),
        "auth.me" => {
            Ok(jsn::user_public(conn, require_user(conn, user_id)?).unwrap_or(Value::Null))
        }
        "auth.registerDevice" => {
            let uid = require_user(conn, user_id)?;
            crate::device::register(conn, uid, input)
                .map_err(|e| ApiError::bad(format!("Не удалось зарегистрировать устройство: {e}")))
        }
        "auth.devices" => {
            let uid = require_user(conn, user_id)?;
            crate::device::list(conn, uid).map_err(|e| ApiError::internal(e.to_string()))
        }
        "auth.revokeDevice" => {
            let uid = require_user(conn, user_id)?;
            let device_id = s(input, "deviceId").ok_or_else(|| ApiError::bad("deviceId"))?;
            let revoked = crate::device::revoke(conn, uid, &device_id)
                .map_err(|e| ApiError::internal(e.to_string()))?;
            Ok(json!({"ok":revoked}))
        }
        "auth.inviteInfo" => invite_info(conn, input),
        "meta.currentUser" => {
            Ok(jsn::user_public(conn, require_user(conn, user_id)?).unwrap_or(Value::Null))
        }
        "meta.transferCounts" => transfer_counts(conn, require_user(conn, user_id)?),
        "meta.workspaces" => workspaces_list(conn),
        "items.list" => items_list(conn, input, user_id),
        "items.byId" => items_by_id(conn, input, user_id),
        "items.byCode" => items_by_code(conn, input, user_id),
        "items.nextInternalId" => items_next_id(conn, input),
        "items.create" => items_create(conn, input, user_id),
        "items.update" => items_update(conn, input, user_id),
        "items.remove" => items_remove(conn, input, user_id),
        "items.addPhoto" => items_add_photo(conn, input, user_id),
        "items.addDocument" => items_add_document(conn, input, user_id),
        "items.addComment" => items_add_comment(conn, input, user_id),
        "items.reportFault" => report_fault(conn, input, user_id),
        "items.faults" => list_faults(conn, input),
        "items.resolveFault" => resolve_fault(conn, input, user_id),
        "items.requestChange" => request_change(conn, input, user_id),
        "items.changeRequests" => list_changes(conn, input),
        "items.decideChange" => decide_change(conn, input, user_id),
        "chat.list" => chat_list(conn, input, user_id),
        "chat.send" => chat_send(conn, input, user_id),
        "sync.status" => Ok(crate::sync::status(conn)),
        "sync.audit" => Ok(crate::sync::integrity_audit(conn)),
        "sync.peers" => Ok(crate::sync::list_peers(conn)),
        "sync.nodeKeys" => Ok(crate::sync::node_keys(conn)),
        "sync.diagnostics" => Ok(crate::diagnostics::list(conn)),
        "sync.clearDiagnostics" => Ok(crate::diagnostics::clear_resolved(conn)),
        "sync.reportTransportStatus" => {
            require_user(conn, user_id)?;
            let transport = s(input, "transport").ok_or_else(|| ApiError::bad("transport"))?;
            if transport != "ble" {
                return Err(ApiError::bad("Неизвестный локальный транспорт"));
            }
            let failed = input.get("error").and_then(Value::as_bool).unwrap_or(false);
            if failed {
                let message = s(input, "message")
                    .unwrap_or_else(|| "Ошибка локального BLE-транспорта".to_string());
                crate::diagnostics::record(
                    conn,
                    "warning",
                    "transport",
                    "ble_transport",
                    &message,
                    None,
                );
            } else {
                crate::diagnostics::resolve(conn, "transport", "ble_transport", None);
            }
            Ok(json!({"ok":true,"active":failed}))
        }
        "sync.exportBundle" => {
            let workspace_guid = s(input, "workspaceGuid");
            let (token, scope) = crate::sync_bundle_export_capability(workspace_guid.as_deref())
                .map_err(ApiError::bad)?;
            let result =
                crate::sync::export_transport_bundle_scoped(conn, Some(&token), scope.as_ref());
            if result.get("ok").and_then(Value::as_bool) == Some(false) {
                crate::diagnostics::record(
                    conn,
                    "error",
                    "transport",
                    "bundle_export_failed",
                    result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Transport bundle отклонён"),
                    None,
                );
                return Err(ApiError::bad(
                    result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Не удалось зашифровать transport bundle"),
                ));
            }
            crate::diagnostics::resolve(conn, "transport", "bundle_export_failed", None);
            Ok(result)
        }
        "sync.importBundle" => {
            let bundle = input
                .get("bundle")
                .ok_or_else(|| ApiError::bad("Нет transport bundle"))?;
            let capabilities = crate::sync_bundle_import_capabilities();
            let mut result =
                json!({"ok":false,"error":"Ни одна capability не расшифровала transport bundle"});
            for (token, scope) in capabilities {
                let candidate = crate::sync::import_transport_bundle_scoped(
                    conn,
                    bundle,
                    Some(&token),
                    scope.as_ref(),
                );
                if candidate.get("ok").and_then(Value::as_bool) == Some(true) {
                    result = candidate;
                    break;
                }
                result = candidate;
            }
            if result.get("ok").and_then(Value::as_bool) == Some(false) {
                crate::diagnostics::record(
                    conn,
                    "error",
                    "transport",
                    "bundle_rejected",
                    result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Transport bundle отклонён"),
                    None,
                );
                return Err(ApiError::bad(
                    result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Transport bundle отклонён"),
                ));
            }
            crate::diagnostics::resolve(conn, "transport", "bundle_rejected", None);
            Ok(result)
        }
        "sync.approveNodeKey" => {
            let uid = require_user(conn, user_id)?;
            let key = s(input, "publicKey").ok_or_else(|| ApiError::bad("publicKey"))?;
            crate::sync::approve_node_key(conn, &key, s(input, "label").as_deref(), uid)
                .map_err(|e| ApiError::bad(e.to_string()))?;
            Ok(crate::sync::node_keys(conn))
        }
        "sync.revokeNodeKey" => {
            let key = s(input, "publicKey").ok_or_else(|| ApiError::bad("publicKey"))?;
            crate::sync::revoke_node_key(conn, &key).map_err(|e| ApiError::bad(e.to_string()))?;
            Ok(crate::sync::node_keys(conn))
        }
        "content.status" => Ok(crate::content::status(conn)),
        "content.setMode" => {
            let mode = s(input, "mode").ok_or_else(|| ApiError::bad("mode"))?;
            crate::content::set_mode(conn, &mode)
                .map_err(|error| ApiError::bad(error.to_string()))?;
            Ok(crate::content::status(conn))
        }
        "content.pin" => {
            let hash = s(input, "hash").ok_or_else(|| ApiError::bad("hash"))?;
            crate::content::pin(conn, &hash, "explicit")
                .map_err(|error| ApiError::bad(error.to_string()))?;
            Ok(crate::content::status(conn))
        }
        "content.unpin" => {
            let hash = s(input, "hash").ok_or_else(|| ApiError::bad("hash"))?;
            crate::content::unpin(conn, &hash).map_err(|error| ApiError::bad(error.to_string()))?;
            Ok(crate::content::status(conn))
        }
        "bit.balance" => bit_balance(conn, input, user_id),
        "bit.transactions" => bit_transactions(conn, input),
        "bit.transfer" => bit_transfer(conn, input, user_id),
        "bit.sale" => bit_sale(conn, input, user_id),
        "bit.mint" => bit_mint(conn, input, user_id),
        "knowledge.list" => knowledge_list(conn, input, user_id),
        "knowledge.bySlug" => knowledge_by_slug(conn, input, user_id),
        "knowledge.save" => knowledge_save(conn, input, user_id),
        "sync.addPeer" => {
            let url = s(input, "url").ok_or_else(|| ApiError::bad("Укажите адрес узла"))?;
            crate::validate_peer_url(&url).map_err(ApiError::bad)?;
            if crate::sync_token().is_none() {
                return Err(ApiError::bad(
                    "Для P2P-обмена задайте MESHKEEPER_SYNC_TOKEN не короче 32 символов",
                ));
            }
            let normalized = url.trim().trim_end_matches('/');
            if [
                crate::sync::local_http_base(),
                crate::sync::guess_lan_base(),
            ]
            .iter()
            .any(|local| local == normalized)
            {
                return Err(ApiError::bad(
                    "Нельзя добавить этот узел в peers самого себя",
                ));
            }
            let added = crate::sync::add_peer(conn, &url, s(input, "name").as_deref(), None);
            if added.get("ok").and_then(Value::as_bool) == Some(false) {
                return Err(ApiError::bad(
                    added
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Не удалось добавить peer"),
                ));
            }
            Ok(added)
        }
        "sync.removePeer" => {
            let url = s(input, "url").ok_or_else(|| ApiError::bad("Укажите адрес узла"))?;
            if std::env::var("MESHKEEPER_UPSTREAM")
                .ok()
                .map(|value| value.trim().trim_end_matches('/').to_string())
                .as_deref()
                == Some(url.trim().trim_end_matches('/'))
            {
                return Err(ApiError::bad(
                    "Постоянный upstream удаляется только из конфигурации запуска",
                ));
            }
            Ok(crate::sync::remove_peer(conn, &url))
        }
        "sync.conflicts" => Ok(crate::sync::list_conflicts(conn)),
        "sync.resolveConflict" => {
            let uid = require_user(conn, user_id)?;
            require_can(conn, uid, "editItems")?;
            crate::sync::resolve_conflict(
                conn,
                i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?,
                i64v(input, "responsibleUserId"),
                uid,
            )
            .map_err(|e| ApiError::bad(e.to_string()))
        }
        "sync.pullNow" => {
            let no_upstream = std::env::var("MESHKEEPER_UPSTREAM")
                .ok()
                .filter(|u| !u.trim().is_empty())
                .is_none();
            if no_upstream && crate::sync::peer_urls(conn).is_empty() {
                return Err(ApiError::bad(
                    "Нет узлов для обмена: добавьте локальный peer или задайте MESHKEEPER_UPSTREAM",
                ));
            }
            crate::sync::request_sync_now();
            Ok(json!({"ok": true, "queued": true}))
        }
        "backup.export" => backup_export(conn, input, user_id),
        "backup.import" => backup_import(conn, input, user_id),
        "transfers.outgoing" => transfers_list(conn, user_id, true),
        "transfers.incoming" => transfers_list(conn, user_id, false),
        "transfers.byId" => transfer_by_id(conn, input, user_id),
        "transfers.prepare" => transfers_prepare(conn, input, user_id),
        "transfers.accept" => transfers_accept(conn, input, user_id, true),
        "transfers.reject" => transfers_accept(conn, input, user_id, false),
        "transfers.acceptAll" => transfers_accept_all(conn, user_id),
        "transfers.take" => transfers_take(conn, input, user_id),
        "transfers.takeMany" => transfers_take_many(conn, input, user_id),
        "transfers.returnItem" => transfers_return(conn, input, user_id),
        "history.movements" => {
            history_list(conn, input, &["move", "transfer_send", "transfer_receive"])
        }
        "history.quantityOps" => history_list(conn, input, &["write_off", "replenish"]),
        "history.all" => history_list(conn, input, &[]),
        "history.writeOff" => history_write_off(conn, input, user_id),
        "history.replenish" => history_replenish(conn, input, user_id),
        "history.move" => history_move(conn, input, user_id),
        "inventory.sessions" => inv_sessions(conn, input),
        "inventory.byId" => inv_by_id(conn, input, user_id),
        "inventory.results" => inv_results(conn, input, user_id),
        "inventory.create" => inv_create(conn, input, user_id),
        "inventory.checkItem" => inv_check(conn, input, user_id),
        "inventory.complete" => inv_complete(conn, input, user_id),
        "notifications.list" => notif_list(conn, user_id),
        "notifications.unreadCount" => notif_unread(conn, user_id),
        "notifications.markRead" => notif_mark(conn, input, false, user_id),
        "notifications.markAllRead" => notif_mark(conn, input, true, user_id),
        "reports.byUsers" => reports_by_users(conn, input),
        "reports.quantityTransactions" => history_list(conn, input, &["write_off", "replenish"]),
        "reports.allItems" => reports_all(conn, input),
        "profile.get" => profile_get(conn, user_id),
        "profile.update" => profile_update(conn, input, user_id),
        "profile.changePassword" => profile_password(conn, input, user_id),
        "admin.users.list" => admin_users(conn, input),
        "admin.users.create" => admin_user_create(conn, input, user_id),
        "admin.users.update" => admin_user_update(conn, input, user_id),
        "admin.users.remove" => admin_user_remove(conn, input, user_id),
        "admin.users.invite" => admin_user_invite(conn, input, user_id),
        "admin.users.defaultRights" => Ok(db::default_rights()),
        "admin.workspaces.list" => workspaces_list(conn),
        "admin.workspaces.create" => ws_create(conn, input, user_id),
        "admin.workspaces.update" => ws_update(conn, input, user_id),
        "admin.workspaces.remove" => Err(ApiError::conflict(
            "Неизменяемую летопись организации нельзя удалить; используйте отзыв доступа",
        )),
        "admin.workspaces.createInvite" => ws_create_invite(conn, input, user_id),
        "admin.workspaces.invites" => ws_invites(conn, input),
        "admin.storages.list" => storages_list(conn, input),
        "admin.storages.create" => storage_create(conn, input, user_id),
        "admin.storages.update" => storage_update(conn, input, user_id),
        "admin.storages.remove" => storage_remove(conn, input, user_id),
        "admin.buildingSites.list" => sites_list(conn, input),
        "admin.buildingSites.create" => site_create(conn, input, user_id),
        "admin.buildingSites.update" => site_update(conn, input, user_id),
        "admin.buildingSites.remove" => site_remove(conn, input, user_id),
        "admin.organizationNodes.list" => organization_nodes_list(conn, input),
        "admin.organizationNodes.create" => organization_node_create(conn, input, user_id),
        "admin.organizationNodes.update" => organization_node_update(conn, input, user_id),
        "admin.organizationNodes.remove" => organization_node_remove(conn, input, user_id),
        "admin.dictionaries.list" => dict_list(conn, input),
        "admin.dictionaries.create" => dict_create(conn, input, user_id),
        "admin.dictionaries.update" => dict_update(conn, input, user_id),
        "admin.dictionaries.remove" => dict_remove(conn, input, user_id),
        _ => Err(ApiError::not_found(format!("Нет процедуры {procedure}"))),
    }
}

fn auth_directory(conn: &Connection) -> ApiResult {
    let mut stmt = conn.prepare("SELECT id FROM users WHERE status!='disabled' ORDER BY id")?;
    let ids: Vec<i64> = stmt
        .query_map([], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    let mut out = Vec::new();
    for id in ids {
        if let Some(mut u) = jsn::user_public(conn, id) {
            let has: Option<String> = conn
                .query_row(
                    "SELECT password_hash FROM users WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .ok()
                .flatten();
            u["hasPassword"] = json!(has.filter(|s| !s.is_empty()).is_some());
            out.push(u);
        }
    }
    Ok(Value::Array(out))
}

/// Что экран входа может предложить прямо сейчас. Регистрация владельца
/// доступна, пока база пуста (bootstrap) либо если она открыта явно.
fn auth_options(conn: &Connection) -> ApiResult {
    let users: i64 = conn
        .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
        .unwrap_or(0);
    let open = std::env::var("MESHKEEPER_OPEN_REGISTRATION").as_deref() == Ok("1");
    Ok(json!({
        "registrationOpen": users == 0 || open,
        "bootstrap": users == 0,
        "demoLogin": std::env::var("MESHKEEPER_DEMO_LOGIN").as_deref() == Ok("1"),
    }))
}

/// Сколько неудачных попыток проходит без задержки.
const LOGIN_FREE_ATTEMPTS: i64 = 5;
/// Базовая пауза после исчерпания попыток; дальше удваивается.
const LOGIN_LOCK_BASE_SECS: i64 = 30;
const LOGIN_LOCK_MAX_SECS: i64 = 900;
/// Через столько тишины счётчик неудач обнуляется.
const LOGIN_FAILURE_TTL_SECS: i64 = 3600;

/// Ключ троттлинга — только цифры номера, чтобы «+7 900…» и «8900…»
/// считались одной учётной записью.
fn throttle_key(phone: &str) -> String {
    db::digits_only(phone)
}

/// Отказ, если по этому номеру уже перебирали пароль.
fn check_login_allowed(conn: &Connection, key: &str) -> Result<(), ApiError> {
    let locked: Option<String> = conn
        .query_row(
            "SELECT locked_until FROM login_throttle WHERE key=?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(raw) = locked else { return Ok(()) };
    let Ok(until) = chrono::DateTime::parse_from_rfc3339(&raw) else {
        return Ok(());
    };
    let left = (until.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds();
    if left <= 0 {
        return Ok(());
    }
    Err(ApiError::new(
        "TOO_MANY_REQUESTS",
        429,
        format!("Слишком много попыток входа. Повторите через {left} с."),
    ))
}

fn note_login_failure(conn: &Connection, key: &str) {
    let now_ts = chrono::Utc::now();
    let previous: Option<(i64, Option<String>)> = conn
        .query_row(
            "SELECT failures, last_failure_at FROM login_throttle WHERE key=?1",
            params![key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    // Давние неудачи не должны копиться месяцами.
    let stale = previous
        .as_ref()
        .and_then(|(_, at)| at.as_deref())
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .is_none_or(|at| {
            (now_ts - at.with_timezone(&chrono::Utc)).num_seconds() > LOGIN_FAILURE_TTL_SECS
        });
    let failures = if stale {
        1
    } else {
        previous.map(|(n, _)| n).unwrap_or(0) + 1
    };

    let locked_until = if failures >= LOGIN_FREE_ATTEMPTS {
        let steps = (failures - LOGIN_FREE_ATTEMPTS).min(20) as u32;
        let secs = LOGIN_LOCK_BASE_SECS
            .saturating_mul(1_i64 << steps.min(10))
            .min(LOGIN_LOCK_MAX_SECS);
        Some((now_ts + chrono::Duration::seconds(secs)).to_rfc3339())
    } else {
        None
    };
    let _ = conn.execute(
        "INSERT INTO login_throttle (key, failures, last_failure_at, locked_until)
         VALUES (?1,?2,?3,?4)
         ON CONFLICT(key) DO UPDATE SET
           failures=excluded.failures,
           last_failure_at=excluded.last_failure_at,
           locked_until=excluded.locked_until",
        params![key, failures, now_ts.to_rfc3339(), locked_until],
    );
}

fn clear_login_failures(conn: &Connection, key: &str) {
    let _ = conn.execute("DELETE FROM login_throttle WHERE key=?1", params![key]);
}

fn auth_login(conn: &Connection, input: &Value) -> ApiResult {
    let id = if let Some(uid) = i64v(input, "userId") {
        if std::env::var("MESHKEEPER_DEMO_LOGIN").as_deref() != Ok("1") {
            return Err(ApiError::unauth("Вход по идентификатору отключён"));
        }
        uid
    } else if let Some(phone) = s(input, "phone") {
        validate_phone(&phone).map_err(|_| ApiError::unauth("Неверный телефон или пароль"))?;
        // Проверяем до обращения к базе паролей: перебор не должен
        // получать даже ответ «есть такой аккаунт или нет».
        check_login_allowed(conn, &throttle_key(&phone))?;
        find_user_phone(conn, &phone).ok_or_else(|| {
            note_login_failure(conn, &throttle_key(&phone));
            // Одна Argon2-операция выравнивает время ответа с существующим
            // аккаунтом и затрудняет перебор зарегистрированных телефонов.
            let _ = hash_password("invalid-login-placeholder");
            ApiError::unauth("Неверный телефон или пароль")
        })?
    } else {
        return Err(ApiError::unauth("Укажите телефон"));
    };
    let (status, hash, name): (String, Option<String>, String) = conn
        .query_row(
            "SELECT status, password_hash, full_name FROM users WHERE id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| ApiError::unauth("Аккаунт не найден"))?;
    if status == "disabled" {
        if let Some(k) = s(input, "phone").map(|p| throttle_key(&p)) {
            note_login_failure(conn, &k);
        }
        let _ = hash_password("disabled-login-placeholder");
        return Err(ApiError::unauth("Неверный телефон или пароль"));
    }
    let key = s(input, "phone").map(|p| throttle_key(&p));
    if let Some(h) = hash.filter(|x| !x.is_empty()) {
        let pw = s(input, "password").unwrap_or_default();
        if pw.chars().count() > 128 {
            if let Some(k) = &key {
                note_login_failure(conn, k);
            }
            return Err(ApiError::unauth("Неверный телефон или пароль"));
        }
        if !verify_password(&pw, &h) {
            if let Some(k) = &key {
                note_login_failure(conn, k);
            }
            return Err(ApiError::unauth("Неверный телефон или пароль"));
        }
        if !h.starts_with("$argon2") {
            conn.execute(
                "UPDATE users SET password_hash=?1 WHERE id=?2",
                params![hash_password(&pw), id],
            )?;
        }
    } else if std::env::var("MESHKEEPER_DEMO_LOGIN").as_deref() != Ok("1") {
        return Err(ApiError::unauth("Неверный телефон или пароль"));
    }
    if status == "invited" {
        let _ = conn.execute("UPDATE users SET status='active' WHERE id=?1", params![id]);
    }
    if let Some(k) = &key {
        clear_login_failures(conn, k);
    }
    let mut u = jsn::user_public(conn, id).ok_or_else(|| ApiError::unauth("Аккаунт не найден"))?;
    u["fullName"] = json!(name);
    Ok(u)
}

/// Базовые статусы и склад нового пространства. Без них у предметов не будет
/// ни «В работе», ни «На проверке» (ТЗ §9), а конфликты синхронизации не смогут
/// пометить предмет как требующий проверки.
fn seed_workspace_defaults(conn: &Connection, ws: i64, owner: i64) -> Result<(), ApiError> {
    db::ensure_workspace_statuses(conn, ws).map_err(|error| ApiError::bad(error.to_string()))?;
    conn.execute(
        "INSERT INTO storages (name, responsible_user_id, workspace_id, address)
         SELECT 'Основной склад', ?2, ?1, ''
         WHERE NOT EXISTS (SELECT 1 FROM storages WHERE workspace_id=?1)",
        params![ws, owner],
    )?;
    Ok(())
}

fn auth_register(conn: &Connection, input: &Value) -> ApiResult {
    let users: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    if users > 0 && std::env::var("MESHKEEPER_OPEN_REGISTRATION").as_deref() != Ok("1") {
        return Err(ApiError::new(
            "FORBIDDEN",
            403,
            "Открытая регистрация отключена; используйте приглашение",
        ));
    }
    let full_name = s(input, "fullName").ok_or_else(|| ApiError::bad("Введите имя"))?;
    let phone = s(input, "phone").ok_or_else(|| ApiError::bad("Введите телефон"))?;
    let password = s(input, "password").ok_or_else(|| ApiError::bad("Введите пароль"))?;
    if full_name.chars().count() > 200 {
        return Err(ApiError::bad("Имя слишком длинное"));
    }
    validate_phone(&phone)?;
    validate_new_password(&password)?;
    if find_user_phone(conn, &phone).is_some() {
        return Err(ApiError::conflict(
            "Этот телефон уже зарегистрирован. Войдите с тем же номером и паролем.",
        ));
    }
    let ws_name = s(input, "workspaceName").unwrap_or_else(|| "Моя группа".into());
    if ws_name.chars().count() > 200 {
        return Err(ApiError::bad("Название группы слишком длинное"));
    }
    let sync_url = s(input, "syncUrl");
    conn.execute(
        "INSERT INTO workspaces (name, timezone, internal_id_prefix, comment, created_at, sync_url, guid) VALUES (?1,?2,'ВН-',?3,?4,?5,?6)",
        params![ws_name, "Europe/Moscow", "Создано при регистрации", now(), sync_url, Uuid::new_v4().to_string()],
    ).map_err(|e| ApiError::bad(e.to_string()))?;
    let ws = conn.last_insert_rowid();
    if let Some(url) = sync_url {
        crate::sync::add_peer(conn, &url, Some("relay"), None);
    }
    conn.execute(
        "INSERT INTO users (full_name, position, phone, status, password_hash, role_rights, created_at)
         VALUES (?1,'Владелец',?2,'active',?3,?4,?5)",
        params![full_name, phone, hash_password(&password), db::owner_rights().to_string(), now()],
    ).map_err(|e| ApiError::conflict(e.to_string()))?;
    let uid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO user_workspaces (user_id, workspace_id, rights_json) VALUES (?1,?2,?3)",
        params![uid, ws, db::owner_rights().to_string()],
    )?;
    crate::sync::record_membership_version(conn, ws, uid, true, None, true)
        .map_err(|error| ApiError::internal(format!("Ошибка версии членства: {error}")))?;
    seed_workspace_defaults(conn, ws, uid)?;
    Ok(jsn::user_public(conn, uid).unwrap())
}

struct Invite {
    id: i64,
    workspace_id: i64,
    role: String,
    max_uses: i64,
    used_count: i64,
    revoked: i64,
    expires_at: Option<String>,
}

impl Invite {
    fn is_expired(&self) -> bool {
        let Some(raw) = self.expires_at.as_deref().filter(|s| !s.is_empty()) else {
            return false;
        };
        match chrono::DateTime::parse_from_rfc3339(raw) {
            Ok(ts) => ts.with_timezone(&chrono::Utc) <= chrono::Utc::now(),
            // Нечитаемый срок считаем истёкшим: приглашение не должно «оживать» из-за битой даты.
            Err(_) => true,
        }
    }
}

fn invite_by_token(conn: &Connection, token: &str) -> Result<Invite, ApiError> {
    // Mesh-журнал никогда не раскрывает bearer-токен приглашения. На принимающей
    // ноде хранится только его SHA-256; введённый QR-токен остаётся доказательством
    // владения и сопоставляется локально.
    let token_digest = format!("sha256:{}", hex::encode(Sha256::digest(token.as_bytes())));
    conn.query_row(
        "SELECT id, workspace_id, role, max_uses, used_count, revoked, expires_at FROM invites WHERE token=?1 OR token=?2",
        params![token, token_digest],
        |r| {
            Ok(Invite {
                id: r.get(0)?,
                workspace_id: r.get(1)?,
                role: r.get(2)?,
                max_uses: r.get(3)?,
                used_count: r.get(4)?,
                revoked: r.get(5)?,
                expires_at: r.get(6)?,
            })
        },
    )
    .map_err(|_| ApiError::not_found("Приглашение недействительно или истекло"))
}

/// Общая проверка пригодности приглашения: отзыв, исчерпание и срок действия.
fn ensure_invite_usable(invite: &Invite) -> Result<(), ApiError> {
    if invite.revoked != 0 {
        return Err(ApiError::bad("Приглашение отозвано"));
    }
    if invite.used_count >= invite.max_uses {
        return Err(ApiError::bad("Приглашение уже использовано"));
    }
    if invite.is_expired() {
        return Err(ApiError::bad("Срок действия приглашения истёк"));
    }
    Ok(())
}

fn consume_invite(conn: &Connection, token: &str, user_id: i64) -> ApiResult {
    let invite = invite_by_token(conn, token)?;
    ensure_invite_usable(&invite)?;
    let (id, ws) = (invite.id, invite.workspace_id);
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
        params![user_id, ws],
        |r| r.get(0),
    )?;
    if exists == 0 {
        conn.execute(
            "INSERT INTO user_workspaces (user_id, workspace_id, rights_json) VALUES (?1,?2,?3)",
            params![user_id, ws, db::rights_for_role(&invite.role).to_string()],
        )?;
        conn.execute(
            "UPDATE users SET status='active' WHERE id=?1 AND status='disabled'",
            [user_id],
        )?;
        let workspace_guid = ledger::guid(conn, "workspaces", ws)
            .map_err(|error| ApiError::internal(format!("Ошибка GUID: {error}")))?;
        let previous: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM membership_versions WHERE workspace_guid=?1 AND user_guid=(SELECT guid FROM users WHERE id=?2)",
                params![workspace_guid,user_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let event = ledger::append(
            conn,
            ws,
            user_id,
            None,
            "membership_join",
            None,
            Some(&invite.role),
            None,
            Some("Вступление по capability-приглашению"),
        )
        .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
        crate::sync::record_membership_version(
            conn,
            ws,
            user_id,
            true,
            event["opId"].as_str(),
            previous == 0,
        )
        .map_err(|error| ApiError::internal(format!("Ошибка версии членства: {error}")))?;
    }
    conn.execute(
        "UPDATE invites SET used_count=used_count+1 WHERE id=?1",
        params![id],
    )?;
    Ok(jsn::workspace_json(conn, ws).unwrap_or(json!({"id": ws})))
}

fn auth_join(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let token = s(input, "token").ok_or_else(|| ApiError::bad("Нет токена приглашения"))?;
    consume_invite(conn, &token, uid)
}

fn auth_join_register(conn: &Connection, input: &Value) -> ApiResult {
    let token = s(input, "token").ok_or_else(|| ApiError::bad("Нет токена приглашения"))?;
    let invite = invite_by_token(conn, &token)?;
    ensure_invite_usable(&invite)?;
    let ws = invite.workspace_id;
    let full_name = s(input, "fullName").ok_or_else(|| ApiError::bad("Введите имя"))?;
    let phone = s(input, "phone").ok_or_else(|| ApiError::bad("Введите телефон"))?;
    let password = s(input, "password").unwrap_or_default();
    if full_name.chars().count() > 200 {
        return Err(ApiError::bad("Имя слишком длинное"));
    }
    validate_phone(&phone)?;
    let uid = if let Some(existing) = find_user_phone(conn, &phone) {
        let h: Option<String> = conn.query_row(
            "SELECT password_hash FROM users WHERE id=?1",
            params![existing],
            |r| r.get(0),
        )?;
        let h = h.unwrap_or_default();
        if !h.is_empty() && !verify_password(&password, &h) {
            return Err(ApiError::unauth("Неверный пароль для этого телефона"));
        }
        if h.is_empty() {
            let invited_here: i64 = conn.query_row(
                "SELECT COUNT(*) FROM user_workspaces uw JOIN users u ON u.id=uw.user_id WHERE uw.user_id=?1 AND uw.workspace_id=?2 AND u.status='invited'",
                params![existing, ws], |r| r.get(0),
            )?;
            if invited_here == 0 || validate_new_password(&password).is_err() {
                return Err(ApiError::unauth("Аккаунт требует персональной активации"));
            }
            conn.execute(
                "UPDATE users SET password_hash=?1,status='active' WHERE id=?2",
                params![hash_password(&password), existing],
            )?;
        }
        existing
    } else {
        validate_new_password(&password)?;
        conn.execute(
            "INSERT INTO users (full_name, position, phone, status, password_hash, role_rights, created_at)
             VALUES (?1,?2,?3,'active',?4,?5,?6)",
            params![
                full_name,
                invite_position(&invite.role),
                phone,
                hash_password(&password),
                db::rights_for_role(&invite.role).to_string(),
                now()
            ],
        ).map_err(|e| ApiError::bad(e.to_string()))?;
        conn.last_insert_rowid()
    };
    let wsj = consume_invite(conn, &token, uid)?;
    let mut u = jsn::user_public(conn, uid).unwrap();
    u["joinedWorkspace"] = wsj;
    u["workspaceId"] = json!(ws);
    Ok(u)
}

fn invite_info(conn: &Connection, input: &Value) -> ApiResult {
    let token = s(input, "token").ok_or_else(|| ApiError::bad("token"))?;
    let invite = invite_by_token(conn, &token)?;
    ensure_invite_usable(&invite)?;
    let wsj = jsn::workspace_json(conn, invite.workspace_id).unwrap_or(json!({}));
    Ok(json!({
        "workspace": wsj,
        "role": invite.role,
        "token": token,
        "expiresAt": invite.expires_at,
    }))
}

fn workspaces_list(conn: &Connection) -> ApiResult {
    let ids: Vec<i64> = CURRENT_UID.with(|c| {
        if let Some(uid) = c.get() {
            conn.prepare("SELECT workspace_id FROM user_workspaces WHERE user_id=?1 ORDER BY id")
                .ok()
                .and_then(|mut stmt| {
                    stmt.query_map(params![uid], |r| r.get(0))
                        .ok()
                        .map(|rows| rows.filter_map(|x| x.ok()).collect())
                })
                .unwrap_or_default()
        } else {
            conn.prepare("SELECT id FROM workspaces ORDER BY id")
                .ok()
                .and_then(|mut stmt| {
                    stmt.query_map([], |r| r.get(0))
                        .ok()
                        .map(|rows| rows.filter_map(|x| x.ok()).collect())
                })
                .unwrap_or_default()
        }
    });
    Ok(Value::Array(
        ids.into_iter()
            .filter_map(|id| jsn::workspace_json(conn, id))
            .collect(),
    ))
}

fn transfer_counts(conn: &Connection, uid: i64) -> ApiResult {
    let outgoing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transfers WHERE from_user_id=?1 AND status IN ('draft','pending')",
        params![uid],
        |r| r.get(0),
    )?;
    let incoming: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transfers WHERE to_user_id=?1 AND status='pending'",
        params![uid],
        |r| r.get(0),
    )?;
    Ok(json!({"outgoing": outgoing, "incoming": incoming}))
}

/// Карточка для списка: без оригиналов снимков.
fn item_for_list(conn: &Connection, id: i64) -> Option<Value> {
    let mut item = jsn::item_json(conn, id, false)?;
    jsn::strip_full_photos(&mut item);
    Some(item)
}

fn items_list(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let page = i64v(input, "page").unwrap_or(1).max(1);
    let limit = i64v(input, "limit").unwrap_or(20).clamp(1, 500);
    let search = s(input, "search").map(|q| q.to_lowercase());
    let only_mine = b(input, "onlyMine").unwrap_or(false);
    let mut stmt = conn.prepare("SELECT id, title, internal_id, serial_number, responsible_user_id FROM items WHERE workspace_id=?1 AND archived=0 ORDER BY created_at DESC, id DESC")?;
    let mut ids: Vec<i64> = Vec::new();
    let rows = stmt.query_map(params![ws], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })?;
    for row in rows.flatten() {
        let (id, title, internal, serial, resp) = row;
        if only_mine && resp != user_id {
            continue;
        }
        if let Some(ref q) = search {
            let blob = format!(
                "{} {} {}",
                title,
                internal,
                serial.clone().unwrap_or_default()
            )
            .to_lowercase();
            if !blob.contains(q) {
                continue;
            }
        }
        ids.push(id);
    }
    let total = ids.len() as i64;
    let start = ((page - 1) * limit) as usize;
    let has_more = total > start as i64 + limit;
    let rows: Vec<Value> = ids
        .into_iter()
        .skip(start)
        .take(limit as usize)
        .filter_map(|id| item_for_list(conn, id))
        .collect();
    Ok(json!({"rows": rows, "page": page, "limit": limit, "hasMore": has_more, "total": total}))
}

fn items_by_id(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    require_item_access(conn, uid, id)?;
    jsn::item_json(conn, id, true).ok_or_else(|| ApiError::not_found("Инструмент не найден"))
}

fn items_by_code(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let code = s(input, "code").ok_or_else(|| ApiError::bad("code"))?;
    let canonical_guid = code
        .trim()
        .strip_prefix("everyday:item:")
        .filter(|guid| Uuid::parse_str(guid).is_ok());
    if code.trim().starts_with("everyday:item:") && canonical_guid.is_none() {
        return Err(ApiError::bad("Некорректный GUID в QR-коде Everyday"));
    }
    let id: Option<i64> = conn.query_row(
        "SELECT i.id
         FROM items i
         JOIN user_workspaces m ON m.workspace_id=i.workspace_id
         JOIN users u ON u.id=m.user_id
         WHERE m.user_id=?2 AND u.status='active' AND i.archived=0
           AND ((?3 IS NOT NULL AND i.guid=?3) OR (?3 IS NULL AND (i.qr_code=?1 OR i.internal_id=?1 OR UPPER(i.qr_code)=UPPER(?1) OR UPPER(i.internal_id)=UPPER(?1))))
         ORDER BY CASE WHEN ?3 IS NOT NULL THEN 0 WHEN i.qr_code=?1 THEN 1 WHEN i.internal_id=?1 THEN 2 ELSE 3 END, i.id
         LIMIT 1",
        params![code, uid, canonical_guid], |r| r.get(0),
    ).optional().ok().flatten();
    let id = id.ok_or_else(|| ApiError::not_found("Инструмент с таким QR/номером не найден"))?;
    require_item_access(conn, uid, id)?;
    jsn::item_json(conn, id, false).ok_or_else(|| ApiError::not_found("Инструмент не найден"))
}

fn items_next_id(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let prefix: String = conn
        .query_row(
            "SELECT internal_id_prefix FROM workspaces WHERE id=?1",
            params![ws],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "ВН-".into());
    let mut stmt = conn.prepare("SELECT internal_id FROM items WHERE workspace_id=?1")?;
    let ids: Vec<String> = stmt
        .query_map(params![ws], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    let mut max = 0i64;
    for id in ids {
        if let Some(n) = id.strip_prefix(&prefix).and_then(|x| x.parse::<i64>().ok()) {
            if n > max {
                max = n;
            }
        }
    }
    Ok(json!(format!("{prefix}{:04}", max + 1)))
}

fn item_state_fields(conn: &Connection, id: i64) -> Result<(i64, String, Value), ApiError> {
    conn.query_row(
        "SELECT i.workspace_id,i.guid,i.internal_id,i.title,i.category_id,i.brand_id,i.status_id,i.responsible_user_id,i.building_site_id,i.storage_id,i.serial_number,i.qr_code,i.calibrated_until,i.min_quantity,i.quantitative,i.quantity,i.unit,i.cost,i.comment,i.source_system,i.external_id,i.metadata_json,i.organization_node_id
         FROM items i WHERE i.id=?1",
        [id],
        |r| {
            let ws:i64=r.get(0)?;
            let reference=|table:&str,value:Option<i64>| value.and_then(|value| ledger::guid(conn,table,value).ok());
            let metadata:Option<String>=r.get(21)?;
            Ok((ws,r.get(1)?,json!({
                "internalId":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,
                "categoryGuid":reference("categories",r.get(4)?),"brandGuid":reference("brands",r.get(5)?),
                "statusSlug":r.get::<_,Option<i64>>(6)?.and_then(|id|conn.query_row("SELECT slug FROM statuses WHERE id=?1",[id],|row|row.get::<_,String>(0)).ok()),"responsibleGuid":reference("users",r.get(7)?),
                "buildingSiteGuid":reference("building_sites",r.get(8)?),"storageGuid":reference("storages",r.get(9)?),
                "serialNumber":r.get::<_,Option<String>>(10)?,"qrCode":r.get::<_,Option<String>>(11)?,
                "calibratedUntil":r.get::<_,Option<String>>(12)?,"minQuantity":r.get::<_,Option<f64>>(13)?,
                "quantitative":r.get::<_,i64>(14)?!=0,"quantity":r.get::<_,Option<f64>>(15)?,
                "unit":r.get::<_,Option<String>>(16)?,"cost":r.get::<_,Option<f64>>(17)?,
                "comment":r.get::<_,Option<String>>(18)?,"sourceSystem":r.get::<_,Option<String>>(19)?,
                "externalId":r.get::<_,Option<String>>(20)?,
                "metadata":metadata.and_then(|raw|serde_json::from_str::<Value>(&raw).ok()).unwrap_or_else(||json!({})),
                "organizationNodeGuid":reference("organization_nodes",r.get(22)?),
            })))
        },
    ).map_err(Into::into)
}

fn item_state_payload_hash(
    item_guid: &str,
    parent: Option<&str>,
    depth: i64,
    workspace_guid: &str,
    actor_guid: &str,
    fields: &Value,
    updated_at: &str,
) -> String {
    let payload = json!({"domain":"everyday/item-state/v1","itemGuid":item_guid,"parentHash":parent,"depth":depth,"workspaceGuid":workspace_guid,"actorGuid":actor_guid,"fields":fields,"updatedAt":updated_at});
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload).expect("JSON serialization"))
    )
}

fn item_state_version_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/item-state-ledger/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}

fn record_item_state_version(
    conn: &Connection,
    id: i64,
    uid: i64,
    operation: &str,
    note: &str,
) -> Result<Value, ApiError> {
    ledger::guid(conn, "items", id).map_err(|error| ApiError::internal(error.to_string()))?;
    let (ws, item_guid, fields) = item_state_fields(conn, id)?;
    let parent:Option<(String,i64)>=conn.query_row("SELECT version_hash,depth FROM item_state_versions WHERE item_guid=?1 ORDER BY depth DESC,version_hash DESC LIMIT 1",[&item_guid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let (parent_hash, depth, event_type) = match (parent, operation) {
        (Some((hash, depth)), _) => (Some(hash), depth + 1, "item_state_update"),
        (None, "create") => (None, 0, "item_state_create"),
        (None, _) => (None, 0, "item_state_adopt"),
    };
    let workspace_guid =
        ledger::guid(conn, "workspaces", ws).map_err(|e| ApiError::internal(e.to_string()))?;
    let actor_guid =
        ledger::guid(conn, "users", uid).map_err(|e| ApiError::internal(e.to_string()))?;
    let updated_at = now();
    let payload_hash = item_state_payload_hash(
        &item_guid,
        parent_hash.as_deref(),
        depth,
        &workspace_guid,
        &actor_guid,
        &fields,
        &updated_at,
    );
    let event = ledger::append(
        conn,
        ws,
        uid,
        Some(id),
        event_type,
        Some(&item_guid),
        Some(&payload_hash),
        None,
        Some(note),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    let ledger_hash = event["opId"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?;
    let version_hash = item_state_version_hash(&payload_hash, ledger_hash);
    conn.execute("INSERT INTO item_state_versions(version_hash,item_guid,parent_hash,depth,workspace_guid,actor_guid,fields_json,payload_hash,ledger_hash,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![version_hash,item_guid,parent_hash,depth,workspace_guid,actor_guid,fields.to_string(),payload_hash,ledger_hash,updated_at])?;
    Ok(json!({"versionHash":version_hash,"ledgerHash":ledger_hash}))
}

fn adopt_item_config_references(conn: &Connection, id: i64, uid: i64) -> Result<(), ApiError> {
    let refs: (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    ) = conn.query_row(
        "SELECT category_id,brand_id,status_id,building_site_id,storage_id FROM items WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    for (kind, value) in [
        ("category", refs.0),
        ("brand", refs.1),
        ("status", refs.2),
        ("site", refs.3),
        ("storage", refs.4),
    ] {
        let Some(value) = value else { continue };
        let table = config_kind_table(kind)?;
        let guid = ledger::guid(conn, table, value)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        let exists: i64 = conn.query_row(
            "SELECT count(*) FROM config_versions WHERE kind=?1 AND entity_guid=?2",
            params![kind, guid],
            |r| r.get(0),
        )?;
        if exists == 0 {
            record_config_version(conn, kind, value, uid, "adopt")?;
        }
    }
    Ok(())
}

fn items_create(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| items_create_atomic(conn, input, user_id))
}

fn items_create_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    require_member(conn, uid, ws)?;
    require_can_in_workspace(conn, uid, ws, "createItems")?;
    validate_item_references(conn, input, ws)?;
    let title = s(input, "title").ok_or_else(|| ApiError::bad("Название обязательно"))?;
    let internal = if let Some(v) = s(input, "internalId") {
        v
    } else {
        items_next_id(conn, &json!({"workspaceId": ws}))?
            .as_str()
            .unwrap_or("ВН-0001")
            .to_string()
    };
    let qr = s(input, "qrCode").or(Some(internal.clone()));
    let metadata = item_metadata(input)?;
    conn.execute(
        "INSERT INTO items (internal_id, title, category_id, brand_id, status_id, responsible_user_id, building_site_id, storage_id, workspace_id, serial_number, cost, quantitative, quantity, unit, comment, qr_code, source_system, external_id, metadata_json, created_at, organization_node_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
        params![
            internal, title, i64v(input,"categoryId"), i64v(input,"brandId"), i64v(input,"statusId"),
            i64v(input,"responsibleUserId"), i64v(input,"buildingSiteId"), i64v(input,"storageId"), ws,
            s(input,"serialNumber"), f64v(input,"cost"), b(input,"quantitative").unwrap_or(false) as i64,
            f64v(input,"quantity"), s(input,"unit"), s(input,"comment"), qr,
            s(input,"sourceSystem"), s(input,"externalId"), metadata, now(), i64v(input,"organizationNodeId")
        ],
    ).map_err(|e| ApiError::bad(e.to_string()))?;
    let id = conn.last_insert_rowid();
    if let Some(arr) = g(input, "photos").as_array() {
        for (i, p) in arr.iter().enumerate() {
            // Принимаем и строку с оригиналом, и пару {url, thumbUrl}.
            let (url, thumb) = match p {
                Value::String(url) => (Some(url.clone()), None),
                Value::Object(_) => (
                    p.get("url").and_then(Value::as_str).map(str::to_owned),
                    p.get("thumbUrl").and_then(Value::as_str).map(str::to_owned),
                ),
                _ => (None, None),
            };
            if let Some(url) = url {
                insert_photo(conn, id, &url, thumb.as_deref(), i == 0)?;
            }
        }
    }
    adopt_item_config_references(conn, id, uid)?;
    record_item_state_version(conn, id, uid, "create", "Инструмент добавлен в каталог")?;
    jsn::item_json(conn, id, true).ok_or_else(|| ApiError::bad("не создан"))
}

fn items_update(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| items_update_atomic(conn, input, user_id))
}

fn items_update_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    items_update_atomic_bound(conn, input, user_id, None)
}

fn items_update_atomic_bound(
    conn: &Connection,
    input: &Value,
    user_id: Option<i64>,
    ledger_binding: Option<(&str, &str, &str)>,
) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "editItems")?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    require_item_access(conn, uid, id)?;
    let before = jsn::item_json(conn, id, false)
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    let ws = before["workspaceId"]
        .as_i64()
        .ok_or_else(|| ApiError::bad("Некорректный item"))?;
    validate_item_references(conn, input, ws)?;
    let before_status = before["statusId"].as_i64();
    let next_status = if input.get("statusId").is_some() {
        i64v(input, "statusId")
    } else {
        before_status
    };
    let reason = status_change_reason(conn, input, before_status, next_status)?;
    let metadata = item_metadata(input)?;
    conn.execute(
        "UPDATE items SET title=COALESCE(?2,title),
         category_id=CASE WHEN ?3 THEN ?4 ELSE category_id END,
         brand_id=CASE WHEN ?5 THEN ?6 ELSE brand_id END,
         status_id=CASE WHEN ?7 THEN ?8 ELSE status_id END,
         responsible_user_id=CASE WHEN ?9 THEN ?10 ELSE responsible_user_id END,
         building_site_id=CASE WHEN ?11 THEN ?12 ELSE building_site_id END,
         storage_id=CASE WHEN ?13 THEN ?14 ELSE storage_id END,
         serial_number=CASE WHEN ?15 THEN ?16 ELSE serial_number END,
         cost=CASE WHEN ?17 THEN ?18 ELSE cost END,
         comment=CASE WHEN ?19 THEN ?20 ELSE comment END,
         qr_code=CASE WHEN ?21 THEN ?22 ELSE qr_code END,
         calibrated_until=CASE WHEN ?23 THEN ?24 ELSE calibrated_until END,
         min_quantity=CASE WHEN ?25 THEN ?26 ELSE min_quantity END,
         source_system=CASE WHEN ?27 THEN ?28 ELSE source_system END,
         external_id=CASE WHEN ?29 THEN ?30 ELSE external_id END,
         metadata_json=CASE WHEN ?31 THEN ?32 ELSE metadata_json END,
         organization_node_id=CASE WHEN ?33 THEN ?34 ELSE organization_node_id END
         WHERE id=?1",
        params![
            id,
            s(input, "title"),
            input.get("categoryId").is_some(),
            i64v(input, "categoryId"),
            input.get("brandId").is_some(),
            i64v(input, "brandId"),
            input.get("statusId").is_some(),
            i64v(input, "statusId"),
            input.get("responsibleUserId").is_some(),
            i64v(input, "responsibleUserId"),
            input.get("buildingSiteId").is_some(),
            i64v(input, "buildingSiteId"),
            input.get("storageId").is_some(),
            i64v(input, "storageId"),
            input.get("serialNumber").is_some(),
            s(input, "serialNumber"),
            input.get("cost").is_some(),
            f64v(input, "cost"),
            input.get("comment").is_some(),
            s(input, "comment"),
            input.get("qrCode").is_some(),
            s(input, "qrCode"),
            input.get("calibratedUntil").is_some(),
            s(input, "calibratedUntil"),
            input.get("minQuantity").is_some(),
            f64v(input, "minQuantity"),
            input.get("sourceSystem").is_some(),
            s(input, "sourceSystem"),
            input.get("externalId").is_some(),
            s(input, "externalId"),
            input.get("metadata").is_some(),
            metadata,
            input.get("organizationNodeId").is_some(),
            i64v(input, "organizationNodeId")
        ],
    )?;
    // Смена статуса и места хранения не должна выглядеть как безымянная правка:
    // журнал фиксирует переход и причину.
    let mut note = String::from("Данные инструмента обновлены");
    if before_status != next_status {
        let (from_name, _) = status_label(conn, before_status);
        let (to_name, _) = status_label(conn, next_status);
        note = format!(
            "Статус: {} → {}",
            from_name.unwrap_or_else(|| "—".into()),
            to_name.unwrap_or_else(|| "—".into())
        );
        if let Some(text) = &reason {
            note.push_str(&format!(". Причина: {text}"));
        }
    }
    let before_storage = before["storageId"].as_i64();
    let next_storage = if input.get("storageId").is_some() {
        i64v(input, "storageId")
    } else {
        before_storage
    };
    if before_storage != next_storage {
        let name: Option<String> = next_storage.and_then(|sid| {
            conn.query_row("SELECT name FROM storages WHERE id=?1", params![sid], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .ok()
            .flatten()
        });
        note.push_str(&format!(
            ". Место хранения: {}",
            name.unwrap_or_else(|| "не указано".into())
        ));
    }
    let before_node = before["organizationNodeId"].as_i64();
    let next_node = if input.get("organizationNodeId").is_some() {
        i64v(input, "organizationNodeId")
    } else {
        before_node
    };
    if before_node != next_node {
        let name: Option<String> = next_node.and_then(|node_id| {
            conn.query_row(
                "SELECT name FROM organization_nodes WHERE id=?1",
                [node_id],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
        });
        note.push_str(&format!(
            ". Раздел структуры: {}",
            name.unwrap_or_else(|| "не указан".into())
        ));
    }
    let (event_type, from_label, to_label) = ledger_binding
        .map_or(("update", None, None), |(kind, from, to)| {
            (kind, Some(from), Some(to))
        });
    let event = if ledger_binding.is_none() {
        adopt_item_config_references(conn, id, uid)?;
        record_item_state_version(conn, id, uid, "update", &note)?
    } else {
        let bound_event = ledger::append(
            conn,
            ws,
            uid,
            Some(id),
            event_type,
            from_label,
            to_label,
            None,
            Some(&note),
        )
        .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
        // Решение коммитит portable patch, а отдельная производная запись —
        // полное получившееся master-состояние. Обе операции авторизованы тем
        // же атомарным запросом и откатываются вместе.
        adopt_item_config_references(conn, id, uid)?;
        record_item_state_version(conn, id, uid, "update", "Master-состояние после решения")?;
        bound_event
    };
    let mut item = jsn::item_json(conn, id, true).ok_or_else(|| ApiError::not_found("нет"))?;
    if let Some(object) = item.as_object_mut() {
        object.insert("ledgerHash".into(), event["opId"].clone());
    }
    Ok(item)
}

fn items_remove(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| items_remove_atomic(conn, input, user_id))
}

fn items_remove_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "deleteItems")?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let ws: i64 = conn
        .query_row("SELECT workspace_id FROM items WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()?
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    require_member(conn, uid, ws)?;
    let guid =
        ledger::guid(conn, "items", id).map_err(|error| ApiError::internal(error.to_string()))?;
    let (title, responsible, archived): (String, Option<i64>, bool) = conn
        .query_row(
            "SELECT title,responsible_user_id,archived!=0 FROM items WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| ApiError::not_found("Инструмент не найден"))?;
    if archived {
        return Ok(json!({"ok":true,"archived":true,"id":id,"guid":guid,"duplicate":true}));
    }
    let active_holding: i64 = conn.query_row(
        "SELECT count(*) FROM item_holdings WHERE item_id=?1 AND returned_at IS NULL",
        [id],
        |row| row.get(0),
    )?;
    let pending_transfer: i64 = conn.query_row(
        "SELECT count(*) FROM transfers WHERE item_id=?1 AND status IN ('draft','pending')",
        [id],
        |row| row.get(0),
    )?;
    if responsible.is_some() || active_holding > 0 || pending_transfer > 0 {
        return Err(ApiError::conflict(
            "Нельзя архивировать выданный или передаваемый инструмент; сначала верните его",
        ));
    }
    let event = ledger::append(
        conn,
        ws,
        uid,
        Some(id),
        "item_archive",
        Some(&guid),
        None,
        None,
        Some(&format!("Карточка ТМЦ архивирована: {title}")),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    let tombstone = crate::sync::record_item_tombstone(conn, ws, id, uid, &event)
        .map_err(|error| ApiError::internal(format!("Ошибка tombstone: {error}")))?;
    Ok(json!({"ok":true,"archived":true,"id":id,"guid":guid,"tombstone":tombstone}))
}

/// Контрольная сумма вложения (ТЗ §5): по ней видно подмену снимка.
fn photo_checksum(url: &str) -> String {
    hex::encode(Sha256::digest(url.as_bytes()))
}

fn insert_photo(
    conn: &Connection,
    item_id: i64,
    url: &str,
    thumb: Option<&str>,
    is_title: bool,
) -> Result<i64, ApiError> {
    let stored_url = crate::content::ingest_data_url(conn, url)
        .map_err(|error| ApiError::bad(format!("Некорректное фото: {error}")))?
        .unwrap_or_else(|| url.to_string());
    let source_thumb = thumb.unwrap_or(url);
    let stored_thumb = crate::content::ingest_data_url(conn, source_thumb)
        .map_err(|error| ApiError::bad(format!("Некорректная миниатюра: {error}")))?
        .unwrap_or_else(|| source_thumb.to_string());
    let checksum = stored_url
        .strip_prefix("cas:")
        .map(str::to_string)
        .unwrap_or_else(|| photo_checksum(url));
    conn.execute(
        "INSERT INTO item_photos (item_id, url, thumb_url, sha256, is_title, guid) VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            item_id,
            stored_url,
            stored_thumb,
            checksum,
            is_title as i64,
            uuid::Uuid::new_v4().to_string()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn items_add_photo(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "editItems")?;
    let item_id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    require_item_access(conn, uid, item_id)?;
    let url = s(input, "url").ok_or_else(|| ApiError::bad("url"))?;
    let is_title = b(input, "isTitle").unwrap_or(false);
    let thumb = s(input, "thumbUrl");
    let id = insert_photo(conn, item_id, &url, thumb.as_deref(), is_title)?;
    Ok(json!({
        "id": id,
        "itemId": item_id,
        "url": url,
        "thumbUrl": thumb.unwrap_or(url.clone()),
        "sha256": photo_checksum(&url),
        "isTitle": is_title
    }))
}

fn items_add_document(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| items_add_document_atomic(conn, input, user_id))
}

fn items_add_document_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let item_id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    require_item_access(conn, uid, item_id)?;
    require_can(conn, uid, "manageDocuments")?;
    let name = s(input, "name").ok_or_else(|| ApiError::bad("Название документа обязательно"))?;
    if name.chars().count() > 200 {
        return Err(ApiError::bad("Название документа длиннее 200 символов"));
    }
    let source =
        s(input, "url").ok_or_else(|| ApiError::bad("Содержимое документа обязательно"))?;
    let access = s(input, "accessLevel").unwrap_or_else(|| "members".into());
    if !matches!(access.as_str(), "members" | "accounting" | "managers") {
        return Err(ApiError::bad(
            "accessLevel: members, accounting или managers",
        ));
    }
    let stored = crate::content::ingest_data_url(conn, &source)
        .map_err(|error| ApiError::bad(format!("Некорректный документ: {error}")))?
        .unwrap_or_else(|| source.clone());
    let checksum = stored
        .strip_prefix("cas:")
        .map(str::to_string)
        .unwrap_or_else(|| photo_checksum(&source));
    let mime = s(input, "mime").or_else(|| {
        source
            .strip_prefix("data:")
            .and_then(|value| value.split_once(';'))
            .map(|(mime, _)| mime.to_string())
    });
    let guid = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO item_documents(item_id,name,url,guid,mime,sha256,author_id,access_level)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![item_id, name, stored, guid, mime, checksum, uid, access],
    )?;
    let item = jsn::item_json(conn, item_id, false)
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    let ws = item["workspaceId"].as_i64().unwrap_or(1);
    ledger::append(
        conn,
        ws,
        uid,
        Some(item_id),
        "document_add",
        Some(&guid),
        Some(&checksum),
        None,
        Some(&format!("Документ добавлен: {name}; доступ: {access}")),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    Ok(json!({
        "id":conn.last_insert_rowid(),"guid":guid,"itemId":item_id,"name":name,
        "url":source,"mime":mime,"sha256":checksum,"authorId":uid,"accessLevel":access
    }))
}

fn item_comment_payload_hash(
    workspace_guid: &str,
    item_guid: &str,
    author_guid: &str,
    guid: &str,
    text: &str,
    created_at: &str,
) -> String {
    let payload = json!({
        "domain":"everyday/item-comment/v1","workspaceGuid":workspace_guid,
        "itemGuid":item_guid,"authorGuid":author_guid,"guid":guid,
        "text":text,"createdAt":created_at,
    });
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload).expect("JSON serialization"))
    )
}

fn items_add_comment(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| items_add_comment_atomic(conn, input, user_id))
}

fn items_add_comment_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let item_id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let ws = require_item_access(conn, uid, item_id)?;
    let text = s(input, "text").ok_or_else(|| ApiError::bad("text"))?;
    if text.chars().count() > 8_000 {
        return Err(ApiError::bad("Комментарий длиннее 8000 символов"));
    }
    let guid = Uuid::new_v4().to_string();
    let created_at = now();
    let workspace_guid = ledger::guid(conn, "workspaces", ws)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let item_guid = ledger::guid(conn, "items", item_id)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let author_guid =
        ledger::guid(conn, "users", uid).map_err(|error| ApiError::internal(error.to_string()))?;
    let payload_hash = item_comment_payload_hash(
        &workspace_guid,
        &item_guid,
        &author_guid,
        &guid,
        &text,
        &created_at,
    );
    let event = ledger::append(
        conn,
        ws,
        uid,
        Some(item_id),
        "item_comment",
        Some(&guid),
        Some(&payload_hash),
        None,
        Some(&text),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    let ledger_hash = event["opId"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?;
    let record_hash = format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/item-comment-record/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    );
    conn.execute(
        "INSERT INTO item_comments (item_id, user_id, text, created_at) VALUES (?1,?2,?3,?4)",
        params![item_id, uid, text, created_at],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO item_comment_records(record_hash,guid,workspace_guid,item_guid,author_guid,text,payload_hash,ledger_hash,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![record_hash,guid,workspace_guid,item_guid,author_guid,text,payload_hash,ledger_hash,created_at],
    )?;
    Ok(
        json!({"id": id, "guid":guid, "recordHash":record_hash, "ledgerHash":ledger_hash,
            "itemId": item_id, "userId": uid, "text": text, "user": jsn::user_public(conn, uid)}),
    )
}

fn transfers_list(conn: &Connection, user_id: Option<i64>, outgoing: bool) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let sql = if outgoing {
        "SELECT id FROM transfers WHERE from_user_id=?1 AND status IN ('draft','pending') ORDER BY id DESC"
    } else {
        "SELECT id FROM transfers WHERE to_user_id=?1 AND status='pending' ORDER BY id DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let ids: Vec<i64> = stmt
        .query_map(params![uid], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Value::Array(
        ids.into_iter()
            .filter_map(|id| jsn::transfer_json(conn, id))
            .collect(),
    ))
}

fn transfer_by_id(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let transfer =
        jsn::transfer_json(conn, id).ok_or_else(|| ApiError::not_found("Передача не найдена"))?;
    let ws = transfer["workspaceId"]
        .as_i64()
        .ok_or_else(|| ApiError::bad("Некорректная передача"))?;
    require_member(conn, uid, ws)?;
    let party =
        transfer["fromUserId"].as_i64() == Some(uid) || transfer["toUserId"].as_i64() == Some(uid);
    if !party && !user_can(conn, uid, "manageUsers") {
        return Err(ApiError::new(
            "FORBIDDEN",
            403,
            "Передача доступна только её участникам",
        ));
    }
    Ok(transfer)
}

fn next_transfer_code(conn: &Connection, ws: i64) -> String {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transfers WHERE workspace_id=?1",
            params![ws],
            |r| r.get(0),
        )
        .unwrap_or(0);
    format!("ПП-{:04}", n + 1)
}

fn checkout_policy(conn: &Connection, uid: i64) -> Value {
    jsn::user_public(conn, uid)
        .and_then(|u| u.get("checkoutPolicy").cloned())
        .unwrap_or_else(db::default_checkout_policy)
}

/// Списанный, отправленный в ремонт или на проверку предмет не участвует
/// в обороте — ни выдача, ни передача другому сотруднику.
fn ensure_item_circulates(conn: &Connection, item: &Value, item_id: i64) -> Result<(), ApiError> {
    match item["status"]["slug"].as_str() {
        Some("written-off") => {
            return Err(ApiError::bad("Списанный инструмент недоступен для выдачи"))
        }
        Some("in-repair") | Some("needs-check") => {
            return Err(ApiError::bad(
                "Инструмент на проверке или в ремонте, выдача запрещена",
            ))
        }
        _ => {}
    }
    let open_faults: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM faults WHERE item_id=?1 AND status IN ('open','repair')",
            params![item_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if open_faults > 0 {
        return Err(ApiError::bad(
            "По предмету есть неисправность, выдача запрещена",
        ));
    }
    Ok(())
}

fn take_one(
    conn: &mut Connection,
    uid: i64,
    item_id: i64,
    comment: Option<&str>,
    due_at: Option<&str>,
    photo_url: Option<&str>,
    qty: Option<f64>,
) -> ApiResult {
    atomic(conn, |conn| {
        take_one_atomic(conn, uid, item_id, comment, due_at, photo_url, qty)
    })
}

fn take_one_atomic(
    conn: &Connection,
    uid: i64,
    item_id: i64,
    comment: Option<&str>,
    due_at: Option<&str>,
    photo_url: Option<&str>,
    qty: Option<f64>,
) -> ApiResult {
    let item_ws = require_item_access(conn, uid, item_id)?;
    require_can_in_workspace(conn, uid, item_ws, "transferItems")?;
    let stored_photo = photo_url
        .map(|source| crate::content::ingest_data_url(conn, source))
        .transpose()
        .map_err(|error| ApiError::bad(format!("Некорректное фото выдачи: {error}")))?
        .flatten();
    let photo_url = stored_photo.as_deref().or(photo_url);
    if photo_url.is_some_and(|value| value.len() > 512) {
        return Err(ApiError::bad("Ссылка на фото выдачи слишком длинная"));
    }
    let item = jsn::item_json(conn, item_id, false)
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    ensure_item_circulates(conn, &item, item_id)?;
    if item["responsibleUserId"].as_i64() == Some(uid)
        && !item["quantitative"].as_bool().unwrap_or(false)
    {
        return Err(ApiError::bad("Инструмент уже у вас"));
    }
    let policy = checkout_policy(conn, uid);
    if let Some(cats) = policy.get("allowedCategoryIds").and_then(|v| v.as_array()) {
        if !cats.is_empty() {
            let cat = item["categoryId"].as_i64();
            let ok = cat
                .map(|c| cats.iter().any(|x| x.as_i64() == Some(c)))
                .unwrap_or(false);
            if !ok {
                return Err(ApiError::bad("Вам не разрешено брать эту категорию"));
            }
        }
    }
    let allow_none = policy
        .get("allowNoDueDate")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if due_at.is_none() && !allow_none {
        return Err(ApiError::bad("Укажите срок возврата"));
    }
    if let (Some(due), Some(max_h)) = (due_at, policy.get("maxHours").and_then(|v| v.as_f64())) {
        if let Ok(due_ts) = chrono::DateTime::parse_from_rfc3339(due) {
            let hours =
                (due_ts.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_hours() as f64;
            if hours > max_h + 0.1 {
                return Err(ApiError::bad(format!(
                    "Срок больше разрешённого ({max_h} ч)"
                )));
            }
        }
    }
    let ws = item["workspaceId"].as_i64().unwrap_or(1);
    let from = item["responsibleUserId"]
        .as_i64()
        .or_else(|| item["storage"]["responsibleUserId"].as_i64())
        .unwrap_or(uid);
    let need_admin = policy
        .get("requireApproval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let code = next_transfer_code(conn, ws);
    if item["quantitative"].as_bool().unwrap_or(false) {
        let take_qty = qty.unwrap_or(1.0);
        if take_qty <= 0.0 {
            return Err(ApiError::bad("Укажите количество"));
        }
        let stock = item["quantity"].as_f64().unwrap_or(0.0);
        if take_qty > stock + 1e-9 {
            return Err(ApiError::bad(format!("На складе только {stock}")));
        }
        if need_admin {
            conn.execute(
                "INSERT INTO transfers (code, item_id, from_user_id, to_user_id, to_storage_id, building_site_id, workspace_id, quantity, status, comment, no_confirmation, needs_admin, photo_url, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9,0,1,?10,?11)",
                params![code, item_id, from, uid, item["storageId"].as_i64(), item["buildingSiteId"].as_i64(), ws, take_qty, comment, photo_url, now()],
            )?;
            return Ok(
                json!({"pending": true, "code": code, "itemId": item_id, "quantity": take_qty, "message": "Заявка отправлена администратору"}),
            );
        }
        let changed = conn.execute(
            "UPDATE items SET quantity=quantity-?1 WHERE id=?2 AND quantity>=?1",
            params![take_qty, item_id],
        )?;
        if changed != 1 {
            return Err(ApiError::conflict("Остаток изменился; повторите операцию"));
        }
        conn.execute(
            "INSERT INTO item_holdings (item_id, user_id, quantity, due_at, comment, photo_url, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![item_id, uid, take_qty, due_at, comment, photo_url, now()],
        )?;
        conn.execute(
            "INSERT INTO transfers (code, item_id, from_user_id, to_user_id, to_storage_id, building_site_id, workspace_id, quantity, status, comment, no_confirmation, photo_url, created_at, completed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'accepted',?9,1,?10,?11,?11)",
            params![code, item_id, from, uid, item["storageId"].as_i64(), item["buildingSiteId"].as_i64(), ws, take_qty, comment, photo_url, now()],
        )?;
        let title = item["title"].as_str().unwrap_or("");
        let to_name = jsn::user_public(conn, uid)
            .and_then(|u| u["fullName"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let event = ledger::append(
            conn,
            ws,
            uid,
            Some(item_id),
            "transfer_receive",
            Some("Склад"),
            Some(&to_name),
            Some(take_qty),
            Some(&format!("Выдача {code}: {take_qty} × {title}")),
        )
        .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
        crate::sync::record_custody_entry(
            conn, ws, item_id, uid, take_qty, due_at, comment, photo_url, &event,
        )
        .map_err(|e| ApiError::internal(format!("Ошибка custody-летописи: {e}")))?;
        return jsn::item_json(conn, item_id, false).ok_or_else(|| ApiError::bad("ошибка"));
    }
    if need_admin {
        conn.execute(
            "INSERT INTO transfers (code, item_id, from_user_id, to_user_id, to_storage_id, building_site_id, workspace_id, status, comment, no_confirmation, needs_admin, photo_url, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8,0,1,?9,?10)",
            params![code, item_id, from, uid, item["storageId"].as_i64(), item["buildingSiteId"].as_i64(), ws, comment, photo_url, now()],
        )?;
        notify_admins(
            conn,
            ws,
            item_id,
            "Заявка на выдачу",
            &format!(
                "{} просит {} ({})",
                jsn::user_public(conn, uid)
                    .and_then(|u| u["fullName"].as_str().map(|s| s.to_string()))
                    .unwrap_or_default(),
                item["title"].as_str().unwrap_or(""),
                code
            ),
        );
        return Ok(
            json!({"pending": true, "code": code, "itemId": item_id, "message": "Заявка отправлена администратору"}),
        );
    }
    let in_work: Option<i64> = conn
        .query_row(
            "SELECT id FROM statuses WHERE workspace_id=?1 AND slug='in-work'",
            params![ws],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    conn.execute(
        "INSERT INTO transfers (code, item_id, from_user_id, to_user_id, to_storage_id, building_site_id, workspace_id, status, comment, no_confirmation, photo_url, created_at, completed_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'accepted',?8,1,?9,?10,?10)",
        params![code, item_id, from, uid, item["storageId"].as_i64(), item["buildingSiteId"].as_i64(), ws, comment, photo_url, now()],
    )?;
    conn.execute(
        "UPDATE items SET responsible_user_id=?1, status_id=COALESCE(?2,status_id), due_at=?4 WHERE id=?3",
        params![uid, in_work, item_id, due_at],
    )?;
    let title = item["title"].as_str().unwrap_or("");
    let from_name = jsn::user_public(conn, from)
        .and_then(|u| u["fullName"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "Склад".into());
    let to_name = jsn::user_public(conn, uid)
        .and_then(|u| u["fullName"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    ledger::append(
        conn,
        ws,
        from,
        Some(item_id),
        "transfer_send",
        Some(&from_name),
        Some(&to_name),
        None,
        Some(&format!("Выдача {code}: {title}")),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    let receive_event = ledger::append(
        conn,
        ws,
        uid,
        Some(item_id),
        "transfer_receive",
        Some(&from_name),
        Some(&to_name),
        None,
        Some(&format!("Получение {code}")),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    crate::sync::record_custody_entry(
        conn,
        ws,
        item_id,
        uid,
        1.0,
        due_at,
        comment,
        photo_url,
        &receive_event,
    )
    .map_err(|e| ApiError::internal(format!("Ошибка custody-летописи: {e}")))?;
    jsn::item_json(conn, item_id, false).ok_or_else(|| ApiError::bad("ошибка"))
}

fn transfers_take(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let qty = f64v(input, "quantity");
    let item = jsn::item_json(conn, id, false)
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    let want = qty.unwrap_or(1.0).max(1.0);
    if !item["quantitative"].as_bool().unwrap_or(false) && want > 1.0 {
        let mut taken = Vec::new();
        let mut failed = Vec::new();
        match take_one(
            conn,
            uid,
            id,
            s(input, "comment").as_deref(),
            s(input, "dueAt").as_deref(),
            s(input, "photoUrl").as_deref(),
            None,
        ) {
            Ok(_) => taken.push(id),
            Err(e) => failed.push(json!({"itemId": id, "message": e.message})),
        }
        if let Some(members) = item
            .get("family")
            .and_then(|f| f.get("members"))
            .and_then(|v| v.as_array())
        {
            for m in members {
                if taken.len() as f64 >= want {
                    break;
                }
                let sid = m.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                if sid == id || m.get("inStock").and_then(|v| v.as_bool()) != Some(true) {
                    continue;
                }
                match take_one(
                    conn,
                    uid,
                    sid,
                    s(input, "comment").as_deref(),
                    s(input, "dueAt").as_deref(),
                    s(input, "photoUrl").as_deref(),
                    None,
                ) {
                    Ok(_) => taken.push(sid),
                    Err(e) => failed.push(json!({"itemId": sid, "message": e.message})),
                }
            }
        }
        return Ok(
            json!({"takenCount": taken.len(), "taken": taken, "failed": failed, "itemId": id}),
        );
    }
    take_one(
        conn,
        uid,
        id,
        s(input, "comment").as_deref(),
        s(input, "dueAt").as_deref(),
        s(input, "photoUrl").as_deref(),
        qty,
    )
}

fn transfers_take_many(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let ids = g(input, "itemIds").as_array().cloned().unwrap_or_default();
    let mut taken = Vec::new();
    let mut failed = Vec::new();
    for v in ids {
        let id = v.as_i64().unwrap_or(0);
        match take_one(
            conn,
            uid,
            id,
            s(input, "comment").as_deref(),
            s(input, "dueAt").as_deref(),
            s(input, "photoUrl").as_deref(),
            f64v(input, "quantity"),
        ) {
            Ok(_) => taken.push(id),
            Err(e) => failed.push(json!({"itemId": id, "message": e.message})),
        }
    }
    Ok(json!({"takenCount": taken.len(), "taken": taken, "failed": failed}))
}

fn transfers_return(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| transfers_return_atomic(conn, input, user_id))
}

fn transfers_return_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let item = jsn::item_json(conn, id, false)
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    if item["quantitative"].as_bool().unwrap_or(false) {
        let held: f64 = conn.query_row(
            "SELECT COALESCE(SUM(quantity),0) FROM item_holdings WHERE item_id=?1 AND user_id=?2 AND returned_at IS NULL",
            params![id, uid], |r| r.get(0),
        ).unwrap_or(0.0);
        if held <= 0.0 {
            return Err(ApiError::bad("У вас нет этого материала"));
        }
        let give = f64v(input, "quantity").unwrap_or(held).min(held);
        conn.execute(
            "UPDATE item_holdings SET returned_at=?1 WHERE item_id=?2 AND user_id=?3 AND returned_at IS NULL",
            params![now(), id, uid],
        )?;
        if give + 1e-9 < held {
            conn.execute(
                "INSERT INTO item_holdings (item_id, user_id, quantity, created_at) VALUES (?1,?2,?3,?4)",
                params![id, uid, held - give, now()],
            )?;
        }
        conn.execute(
            "UPDATE items SET quantity=COALESCE(quantity,0)+?1 WHERE id=?2",
            params![give, id],
        )?;
        let vn = item["internalId"].as_str().unwrap_or("");
        let name = jsn::user_public(conn, uid)
            .and_then(|u| u["fullName"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let event = ledger::append(
            conn,
            item["workspaceId"].as_i64().unwrap_or(1),
            uid,
            Some(id),
            "transfer_send",
            Some(&name),
            Some("Склад"),
            Some(give),
            Some(&format!("Возврат {give} × {vn}")),
        )
        .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
        crate::sync::record_custody_entry(
            conn,
            item["workspaceId"].as_i64().unwrap_or(1),
            id,
            uid,
            -give,
            None,
            s(input, "comment").as_deref(),
            None,
            &event,
        )
        .map_err(|e| ApiError::internal(format!("Ошибка custody-летописи: {e}")))?;
        return jsn::item_json(conn, id, false).ok_or_else(|| ApiError::bad("ошибка"));
    }
    if item["responsibleUserId"].as_i64() != Some(uid) {
        if item["responsibleUserId"].is_null() {
            return Err(ApiError::bad("Инструмент уже на складе"));
        }
        return Err(ApiError::bad("Инструмент на другом сотруднике"));
    }
    let ws = item["workspaceId"].as_i64().unwrap_or(1);
    let in_stock: Option<i64> = conn
        .query_row(
            "SELECT id FROM statuses WHERE workspace_id=?1 AND slug='in-stock'",
            params![ws],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    conn.execute("UPDATE items SET responsible_user_id=NULL, building_site_id=NULL, status_id=COALESCE(?1,status_id), due_at=NULL WHERE id=?2", params![in_stock, id])?;
    let vn = item["internalId"].as_str().unwrap_or("");
    let name = jsn::user_public(conn, uid)
        .and_then(|u| u["fullName"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let event = ledger::append(
        conn,
        ws,
        uid,
        Some(id),
        "transfer_send",
        Some(&name),
        Some("Склад"),
        None,
        Some(&format!("Возврат {vn} на склад")),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    crate::sync::record_custody_entry(
        conn,
        ws,
        id,
        uid,
        -1.0,
        None,
        s(input, "comment").as_deref(),
        None,
        &event,
    )
    .map_err(|e| ApiError::internal(format!("Ошибка custody-летописи: {e}")))?;
    jsn::item_json(conn, id, false).ok_or_else(|| ApiError::bad("ошибка"))
}

fn transfers_prepare(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| transfers_prepare_atomic(conn, input, user_id))
}

fn transfers_prepare_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let item_id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let to = i64v(input, "toUserId").ok_or_else(|| ApiError::bad("toUserId"))?;
    let item = jsn::item_json(conn, item_id, false)
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    let ws = item["workspaceId"].as_i64().unwrap_or(1);
    require_member(conn, uid, ws)?;
    require_can_in_workspace(conn, uid, ws, "transferItems")?;
    require_member(conn, to, ws)
        .map_err(|_| ApiError::bad("Получатель не состоит в этом рабочем пространстве"))?;
    ensure_item_circulates(conn, &item, item_id)?;
    if !item["quantitative"].as_bool().unwrap_or(false)
        && item["responsibleUserId"].as_i64().is_some()
        && item["responsibleUserId"].as_i64() != Some(uid)
    {
        return Err(ApiError::new(
            "FORBIDDEN",
            403,
            "Передать инструмент может только ответственный сотрудник",
        ));
    }
    let quantity = f64v(input, "quantity");
    let source_custody = if item["quantitative"].as_bool().unwrap_or(false) {
        let quantity = quantity
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| ApiError::bad("Для материала укажите количество"))?;
        let held: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(quantity),0) FROM item_holdings
             WHERE item_id=?1 AND user_id=?2 AND returned_at IS NULL",
                params![item_id, uid],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        if held > 1e-9 && held + 1e-9 < quantity {
            return Err(ApiError::bad(
                "У отправителя недостаточно выданного материала",
            ));
        }
        if held <= 1e-9 && item["quantity"].as_f64().unwrap_or(0.0) + 1e-9 < quantity {
            return Err(ApiError::bad("На складе недостаточно материала"));
        }
        held >= quantity - 1e-9
    } else {
        item["responsibleUserId"].as_i64() == Some(uid)
    };
    let status = if b(input, "asDraft").unwrap_or(false) {
        "draft"
    } else {
        "pending"
    };
    let code = next_transfer_code(conn, ws);
    conn.execute(
        "INSERT INTO transfers (code, item_id, from_user_id, to_user_id, to_storage_id, building_site_id, workspace_id, quantity, status, comment, no_confirmation,source_custody,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![code, item_id, uid, to, i64v(input,"toStorageId"), i64v(input,"buildingSiteId"), ws, quantity, status, s(input,"comment"), b(input,"noConfirmation").unwrap_or(false) as i64,source_custody as i64,now()],
    )?;
    let tid = conn.last_insert_rowid();
    let from_guid = ledger::guid(conn, "users", uid)
        .map_err(|error| ApiError::internal(format!("Ошибка GUID отправителя: {error}")))?;
    let to_guid = ledger::guid(conn, "users", to)
        .map_err(|error| ApiError::internal(format!("Ошибка GUID получателя: {error}")))?;
    let event = ledger::append(
        conn,
        ws,
        uid,
        Some(item_id),
        "transfer_send",
        Some(&from_guid),
        Some(&to_guid),
        quantity,
        Some(&format!("Передача {code} оформлена")),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    conn.execute(
        "UPDATE transfers SET prepare_ledger_hash=?1 WHERE id=?2",
        params![event["opId"].as_str(), tid],
    )?;
    if to != uid {
        let title = item["title"].as_str().unwrap_or("");
        let from_name = jsn::user_public(conn, uid)
            .and_then(|u| u["fullName"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO notifications (user_id, item_id, type, title, text, created_at) VALUES (?1,?2,'transfer','Ожидает приёма',?3,?4)",
            params![to, item_id, format!("Передача {code}: {title} от {from_name}"), now()],
        )?;
    }
    jsn::transfer_json(conn, tid).ok_or_else(|| ApiError::bad("ошибка"))
}

fn transfers_accept(
    conn: &mut Connection,
    input: &Value,
    user_id: Option<i64>,
    accept: bool,
) -> ApiResult {
    atomic(conn, |conn| {
        transfers_accept_atomic(conn, input, user_id, accept)
    })
}

fn transfers_accept_atomic(
    conn: &Connection,
    input: &Value,
    user_id: Option<i64>,
    accept: bool,
) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let t =
        jsn::transfer_json(conn, id).ok_or_else(|| ApiError::not_found("Передача не найдена"))?;
    let ws = t["workspaceId"]
        .as_i64()
        .ok_or_else(|| ApiError::bad("Некорректная передача"))?;
    require_member(conn, uid, ws)?;
    let (needs_admin, source_custody, prepare_ledger_hash): (bool, bool, Option<String>) = conn.query_row(
        "SELECT needs_admin != 0,source_custody != 0,prepare_ledger_hash FROM transfers WHERE id=?1",
        params![id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    if needs_admin {
        require_can(conn, uid, "manageUsers")?;
    } else if t["toUserId"].as_i64() != Some(uid) {
        return Err(ApiError::new(
            "FORBIDDEN",
            403,
            "Подтвердить передачу может только получатель",
        ));
    }
    let st = t["status"].as_str().unwrap_or("");
    if st != "pending" && st != "draft" {
        return Err(ApiError::bad("Передача уже завершена"));
    }
    let new_st = if accept { "accepted" } else { "rejected" };
    conn.execute(
        "UPDATE transfers SET status=?1, completed_at=?2, comment=COALESCE(?3,comment) WHERE id=?4",
        params![new_st, now(), s(input, "comment"), id],
    )?;
    if accept {
        let item_id = t["itemId"]
            .as_i64()
            .ok_or_else(|| ApiError::bad("В передаче нет инструмента"))?;
        let item = jsn::item_json(conn, item_id, false)
            .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
        if item["quantitative"].as_bool().unwrap_or(false) {
            let quantity = t["quantity"]
                .as_f64()
                .filter(|q| *q > 0.0)
                .ok_or_else(|| ApiError::bad("В передаче не указано количество"))?;
            if source_custody {
                let held: f64 = conn
                    .query_row(
                        "SELECT COALESCE(SUM(quantity),0) FROM item_holdings
                     WHERE item_id=?1 AND user_id=?2 AND returned_at IS NULL",
                        params![item_id, t["fromUserId"].as_i64()],
                        |row| row.get(0),
                    )
                    .unwrap_or(0.0);
                if held + 1e-9 < quantity {
                    return Err(ApiError::conflict(
                        "Выданная партия отправителя уже изменилась",
                    ));
                }
                conn.execute(
                    "UPDATE item_holdings SET returned_at=?1 WHERE item_id=?2 AND user_id=?3 AND returned_at IS NULL",
                    params![now(),item_id,t["fromUserId"].as_i64()],
                )?;
                if quantity + 1e-9 < held {
                    conn.execute(
                        "INSERT INTO item_holdings(item_id,user_id,quantity,created_at) VALUES(?1,?2,?3,?4)",
                        params![item_id,t["fromUserId"].as_i64(),held-quantity,now()],
                    )?;
                }
            } else {
                let changed = conn.execute(
                    "UPDATE items SET quantity=quantity-?1 WHERE id=?2 AND quantity>=?1",
                    params![quantity, item_id],
                )?;
                if changed != 1 {
                    return Err(ApiError::conflict("Недостаточное количество на складе"));
                }
            }
            conn.execute(
                "INSERT INTO item_holdings (item_id, user_id, quantity, created_at) VALUES (?1,?2,?3,?4)",
                params![item_id, t["toUserId"].as_i64(), quantity, now()],
            )?;
        } else {
            conn.execute("UPDATE items SET responsible_user_id=?1, storage_id=COALESCE(?2,storage_id), building_site_id=COALESCE(?3,building_site_id) WHERE id=?4",
                params![t["toUserId"].as_i64(), t["toStorageId"].as_i64(), t["buildingSiteId"].as_i64(), item_id])?;
        }
    }
    let from_user = t["fromUserId"].as_i64();
    let to_user = t["toUserId"].as_i64();
    let from_guid = from_user
        .map(|user| ledger::guid(conn, "users", user))
        .transpose()
        .map_err(|error| ApiError::internal(format!("Ошибка GUID отправителя: {error}")))?;
    let to_guid = to_user
        .map(|user| ledger::guid(conn, "users", user))
        .transpose()
        .map_err(|error| ApiError::internal(format!("Ошибка GUID получателя: {error}")))?;
    let event = ledger::append(
        conn,
        ws,
        uid,
        t["itemId"].as_i64(),
        if accept {
            "transfer_receive"
        } else {
            "transfer_reject"
        },
        from_guid.as_deref(),
        to_guid.as_deref(),
        t["quantity"].as_f64(),
        Some(if accept {
            "Принята"
        } else {
            "Отклонена"
        }),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    conn.execute(
        "UPDATE transfers SET accept_ledger_hash=?1 WHERE id=?2",
        params![event["opId"].as_str(), id],
    )?;
    if accept {
        let item_id = t["itemId"]
            .as_i64()
            .ok_or_else(|| ApiError::bad("В передаче нет инструмента"))?;
        let quantity = t["quantity"].as_f64().unwrap_or(1.0);
        let recipient = to_user.ok_or_else(|| ApiError::bad("В передаче нет получателя"))?;
        if source_custody {
            let sender = from_user.ok_or_else(|| ApiError::bad("В передаче нет отправителя"))?;
            let prepare_hash = prepare_ledger_hash
                .as_deref()
                .ok_or_else(|| ApiError::bad("Передача не связана с намерением отправителя"))?;
            let prepare_created: String = conn
                .query_row(
                    "SELECT created_at FROM history_entries WHERE hash=?1",
                    [prepare_hash],
                    |row| row.get(0),
                )
                .map_err(|_| ApiError::bad("Не найдено намерение отправителя"))?;
            crate::sync::record_custody_entry(
                conn,
                ws,
                item_id,
                sender,
                -quantity,
                None,
                t["comment"].as_str(),
                None,
                &json!({"opId":prepare_hash,"createdAt":prepare_created}),
            )
            .map_err(|error| ApiError::internal(format!("Ошибка custody отправителя: {error}")))?;
        }
        crate::sync::record_custody_entry(
            conn,
            ws,
            item_id,
            recipient,
            quantity,
            None,
            t["comment"].as_str(),
            None,
            &event,
        )
        .map_err(|error| ApiError::internal(format!("Ошибка custody получателя: {error}")))?;
    }
    jsn::transfer_json(conn, id).ok_or_else(|| ApiError::bad("ошибка"))
}

fn transfers_accept_all(conn: &mut Connection, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let mut stmt =
        conn.prepare("SELECT id FROM transfers WHERE to_user_id=?1 AND status='pending'")?;
    let ids: Vec<i64> = stmt
        .query_map(params![uid], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    drop(stmt);
    let mut accepted = Vec::new();
    for id in ids {
        if let Ok(v) = transfers_accept(conn, &json!({"id": id}), Some(uid), true) {
            accepted.push(v);
        }
    }
    Ok(json!({"acceptedCount": accepted.len(), "accepted": accepted}))
}

fn history_list(conn: &Connection, input: &Value, types: &[&str]) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut sql = String::from("SELECT id FROM history_entries WHERE workspace_id=?1");
    if !types.is_empty() {
        sql.push_str(" AND type IN (");
        sql.push_str(
            &types
                .iter()
                .map(|t| format!("'{t}'"))
                .collect::<Vec<_>>()
                .join(","),
        );
        sql.push(')');
    }
    if let Some(id) = i64v(input, "itemId") {
        sql.push_str(&format!(" AND item_id={id}"));
    }
    sql.push_str(" ORDER BY id DESC LIMIT 500");
    let mut stmt = conn.prepare(&sql)?;
    let ids: Vec<i64> = stmt
        .query_map(params![ws], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    let mut out = Vec::new();
    for id in ids {
        if let Ok(v) = conn.query_row(
            "SELECT id, workspace_id, item_id, type, actor_user_id, from_label, to_label, quantity_delta, comment, hash, created_at, photo_url,event_version,request_device_id,request_nonce,request_hash FROM history_entries WHERE id=?1",
            params![id],
            |r| {
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
                    "actor": jsn::user_public(conn, actor),
                    "item": item_id.and_then(|i| jsn::item_json(conn, i, false)),
                }))
            },
        ) { out.push(v); }
    }
    Ok(Value::Array(out))
}

/// Требует ли группа фото при списании.
fn requires_writeoff_photo(conn: &Connection, ws: i64) -> bool {
    conn.query_row(
        "SELECT require_writeoff_photo FROM workspaces WHERE id=?1",
        params![ws],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(0)
        != 0
}

/// Привязывает фото к уже созданной записи журнала. Отдельным шагом, чтобы
/// не расширять и без того длинную сигнатуру `ledger::append`.
fn attach_photo(conn: &Connection, entry: &Value, photo: Option<&str>) -> Result<(), ApiError> {
    let (Some(url), Some(id)) = (photo, entry.get("id").and_then(Value::as_i64)) else {
        return Ok(());
    };
    conn.execute(
        "UPDATE history_entries SET photo_url=?1 WHERE id=?2",
        params![url, id],
    )?;
    Ok(())
}

fn history_write_off(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| history_write_off_atomic(conn, input, user_id))
}

fn history_write_off_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "writeOff")?;
    if s(input, "comment").is_none() {
        return Err(ApiError::bad("Укажите причину списания"));
    }
    let id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let item_ws = require_item_access(conn, uid, id)?;
    // ТЗ §8: если группа так настроена, списание без фото не принимается.
    let photo = s(input, "photoUrl");
    if requires_writeoff_photo(conn, item_ws) && photo.is_none() {
        return Err(ApiError::bad(
            "В этой группе списание требует фото-подтверждения",
        ));
    }
    let item = jsn::item_json(conn, id, false).ok_or_else(|| ApiError::not_found("нет"))?;
    let ws = item["workspaceId"].as_i64().unwrap_or(1);
    if item["quantitative"].as_bool().unwrap_or(false) {
        let qty = f64v(input, "quantity").unwrap_or(1.0);
        if qty <= 0.0 {
            return Err(ApiError::bad("Количество должно быть больше нуля"));
        }
        let stock = item["quantity"].as_f64().unwrap_or(0.0);
        if qty > stock + 1e-9 {
            return Err(ApiError::bad(format!(
                "Нельзя списать {qty}: доступно {stock}"
            )));
        }
        let changed = conn.execute(
            "UPDATE items SET quantity=quantity-?1 WHERE id=?2 AND quantity>=?1",
            params![qty, id],
        )?;
        if changed != 1 {
            return Err(ApiError::conflict("Остаток изменился; повторите операцию"));
        }
        let entry = ledger::append(
            conn,
            ws,
            uid,
            Some(id),
            "write_off",
            None,
            None,
            Some(-qty),
            s(input, "comment").as_deref(),
        )
        .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
        attach_photo(conn, &entry, photo.as_deref())?;
    } else {
        let st = conn
            .query_row(
                "SELECT id FROM statuses WHERE workspace_id=?1 AND slug='written-off'",
                params![ws],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .ok_or_else(|| ApiError::bad("В рабочем пространстве нет статуса списания"))?;
        conn.execute("UPDATE items SET status_id=?1 WHERE id=?2", params![st, id])?;
        let entry = ledger::append(
            conn,
            ws,
            uid,
            Some(id),
            "write_off",
            None,
            None,
            None,
            s(input, "comment").as_deref(),
        )
        .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
        attach_photo(conn, &entry, photo.as_deref())?;
    }
    jsn::item_json(conn, id, false).ok_or_else(|| ApiError::bad("ошибка"))
}

fn history_replenish(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| history_replenish_atomic(conn, input, user_id))
}

fn history_replenish_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "replenish")?;
    let id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let qty = f64v(input, "quantity").ok_or_else(|| ApiError::bad("quantity"))?;
    if qty <= 0.0 {
        return Err(ApiError::bad("Количество должно быть больше нуля"));
    }
    require_item_access(conn, uid, id)?;
    let item = jsn::item_json(conn, id, false).ok_or_else(|| ApiError::not_found("нет"))?;
    if !item["quantitative"].as_bool().unwrap_or(false) {
        return Err(ApiError::bad("Инструмент не количественный"));
    }
    conn.execute(
        "UPDATE items SET quantity=COALESCE(quantity,0)+?1 WHERE id=?2",
        params![qty, id],
    )?;
    ledger::append(
        conn,
        item["workspaceId"].as_i64().unwrap_or(1),
        uid,
        Some(id),
        "replenish",
        None,
        None,
        Some(qty),
        s(input, "comment").as_deref(),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    jsn::item_json(conn, id, false).ok_or_else(|| ApiError::bad("ошибка"))
}

fn history_move(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| history_move_atomic(conn, input, user_id))
}

fn history_move_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "editItems")?;
    let id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    require_item_access(conn, uid, id)?;
    conn.execute(
        "UPDATE items SET storage_id=COALESCE(?1,storage_id), building_site_id=?2 WHERE id=?3",
        params![
            i64v(input, "toStorageId"),
            i64v(input, "toBuildingSiteId"),
            id
        ],
    )?;
    let item = jsn::item_json(conn, id, false).ok_or_else(|| ApiError::not_found("нет"))?;
    ledger::append(
        conn,
        item["workspaceId"].as_i64().unwrap_or(1),
        uid,
        Some(id),
        "move",
        None,
        None,
        None,
        s(input, "comment").as_deref(),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    Ok(item)
}

fn inv_sessions(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut stmt = conn.prepare("SELECT id, number, workspace_id, status, started_by, created_at, completed_at FROM inventory_sessions WHERE workspace_id=?1 ORDER BY id DESC")?;
    let rows: Vec<Value> = stmt
        .query_map(params![ws], |r| {
            let id: i64 = r.get(0)?;
            let total: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM inventory_results WHERE session_id=?1",
                    params![id],
                    |x| x.get(0),
                )
                .unwrap_or(0);
            let checked: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM inventory_results WHERE session_id=?1 AND checked=1",
                    params![id],
                    |x| x.get(0),
                )
                .unwrap_or(0);
            Ok(json!({
                "id": id, "number": r.get::<_, String>(1)?, "workspaceId": r.get::<_, i64>(2)?,
                "status": r.get::<_, String>(3)?, "startedBy": r.get::<_, i64>(4)?,
                "createdAt": r.get::<_, String>(5)?, "completedAt": r.get::<_, Option<String>>(6)?,
                "totalItems": total, "checkedItems": checked,
                "starter": jsn::user_public(conn, r.get(4)?),
            }))
        })?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Value::Array(rows))
}

fn inv_session_full(conn: &Connection, id: i64) -> Option<Value> {
    conn.query_row(
        "SELECT id, number, workspace_id, status, started_by, created_at, completed_at FROM inventory_sessions WHERE id=?1",
        params![id],
        |r| {
            let mut results = Vec::new();
            let mut stmt = conn.prepare("SELECT id, session_id, item_id, expected_qty, actual_qty, checked FROM inventory_results WHERE session_id=?1").unwrap();
            for row in stmt.query_map(params![id], |x| {
                let item_id: i64 = x.get(2)?;
                Ok(json!({
                    "id": x.get::<_, i64>(0)?, "sessionId": x.get::<_, i64>(1)?, "itemId": item_id,
                    "expectedQty": x.get::<_, Option<f64>>(3)?, "actualQty": x.get::<_, Option<f64>>(4)?,
                    "checked": x.get::<_, i64>(5)? != 0,
                    "item": jsn::item_json(conn, item_id, false)
                }))
            }).unwrap().flatten() { results.push(row); }
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "number": r.get::<_, String>(1)?, "workspaceId": r.get::<_, i64>(2)?,
                "status": r.get::<_, String>(3)?, "startedBy": r.get::<_, i64>(4)?,
                "createdAt": r.get::<_, String>(5)?, "completedAt": r.get::<_, Option<String>>(6)?,
                "starter": jsn::user_public(conn, r.get(4)?),
                "results": results
            }))
        },
    ).ok()
}

fn inventory_record_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/inventory-ledger/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}

fn record_inventory_event(
    conn: &Connection,
    session_id: i64,
    actor_id: i64,
    kind: &str,
    item_id: Option<i64>,
    mut fields: Value,
) -> Result<Value, ApiError> {
    let (session_guid, workspace_id): (Option<String>, i64) = conn.query_row(
        "SELECT guid,workspace_id FROM inventory_sessions WHERE id=?1",
        [session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let session_guid = session_guid
        .filter(|guid| !guid.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    conn.execute(
        "UPDATE inventory_sessions SET guid=?1 WHERE id=?2 AND (guid IS NULL OR guid='')",
        params![session_guid, session_id],
    )?;
    let roots:i64=conn.query_row("SELECT count(*) FROM inventory_records WHERE session_guid=?1 AND kind IN ('create','adopt_check','adopt_complete')",[&session_guid],|r|r.get(0))?;
    let effective_kind = if roots == 0 && kind != "create" {
        format!("adopt_{kind}")
    } else {
        kind.to_string()
    };
    if effective_kind.starts_with("adopt_") {
        let number: String = conn.query_row(
            "SELECT number FROM inventory_sessions WHERE id=?1",
            [session_id],
            |r| r.get(0),
        )?;
        let mut results = Vec::new();
        let mut statement=conn.prepare("SELECT i.guid,r.expected_qty,r.actual_qty,r.checked FROM inventory_results r JOIN items i ON i.id=r.item_id WHERE r.session_id=?1 ORDER BY i.guid")?;
        let rows=statement.query_map([session_id],|r|Ok(json!({"itemGuid":r.get::<_,String>(0)?,"expectedQty":r.get::<_,Option<f64>>(1)?,"actualQty":r.get::<_,Option<f64>>(2)?,"checked":r.get::<_,i64>(3)?!=0})))?;
        results.extend(rows.flatten());
        fields["number"] = json!(number);
        fields["results"] = Value::Array(results);
    }
    let workspace_guid = ledger::guid(conn, "workspaces", workspace_id)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let actor_guid = ledger::guid(conn, "users", actor_id)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let item_guid = item_id
        .map(|id| ledger::guid(conn, "items", id))
        .transpose()
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let created_at = now();
    let payload = json!({
        "domain":"everyday/inventory/v1", "sessionGuid":session_guid,
        "workspaceGuid":workspace_guid, "actorGuid":actor_guid, "kind":effective_kind,
        "itemGuid":item_guid, "fields":fields, "createdAt":created_at
    });
    let payload_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload).expect("JSON serialization"))
    );
    let event = ledger::append(
        conn,
        workspace_id,
        actor_id,
        item_id,
        &format!("inventory_{effective_kind}"),
        Some(&session_guid),
        Some(&payload_hash),
        None,
        Some("Подписанная летопись инвентаризации"),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    let ledger_hash = event["opId"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?;
    let record_hash = inventory_record_hash(&payload_hash, ledger_hash);
    conn.execute(
        "INSERT INTO inventory_records(record_hash,session_guid,workspace_guid,actor_guid,kind,item_guid,number,expected_qty,actual_qty,checked,fields_json,payload_hash,ledger_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![record_hash,session_guid,workspace_guid,actor_guid,effective_kind,item_guid,fields.get("number").and_then(Value::as_str),fields.get("expectedQty").and_then(Value::as_f64),fields.get("actualQty").and_then(Value::as_f64),fields.get("checked").and_then(Value::as_bool).map(i64::from),fields.to_string(),payload_hash,ledger_hash,created_at],
    )?;
    Ok(json!({"recordHash":record_hash,"ledgerHash":ledger_hash}))
}

fn inv_by_id(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let ws: i64 = conn.query_row(
        "SELECT workspace_id FROM inventory_sessions WHERE id=?1",
        params![id],
        |r| r.get(0),
    )?;
    require_member(conn, uid, ws)?;
    inv_session_full(conn, id).ok_or_else(|| ApiError::not_found("Сессия не найдена"))
}
fn inv_results(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let sid = i64v(input, "sessionId").ok_or_else(|| ApiError::bad("sessionId"))?;
    let ws: i64 = conn.query_row(
        "SELECT workspace_id FROM inventory_sessions WHERE id=?1",
        params![sid],
        |r| r.get(0),
    )?;
    require_member(conn, uid, ws)?;
    let s = inv_session_full(conn, sid).ok_or_else(|| ApiError::not_found("Сессия не найдена"))?;
    Ok(s.get("results").cloned().unwrap_or(json!([])))
}
fn inv_create(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| inv_create_atomic(conn, input, user_id))
}

fn inv_create_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "inventory")?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM inventory_sessions WHERE workspace_id=?1",
        params![ws],
        |r| r.get(0),
    )?;
    let number = format!("ИНВ-{:03}", n + 1);
    conn.execute("INSERT INTO inventory_sessions (guid,number, workspace_id, started_by, created_at) VALUES (?1,?2,?3,?4,?5)", params![Uuid::new_v4().to_string(),number, ws, uid, now()])?;
    let sid = conn.last_insert_rowid();
    let mut sql = String::from(
        "SELECT id, quantity, quantitative FROM items WHERE workspace_id=?1 AND archived=0",
    );
    if let Some(st) = i64v(input, "storageId") {
        sql.push_str(&format!(" AND storage_id={st}"));
    }
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(i64, Option<f64>, i64)> = stmt
        .query_map(params![ws], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .filter_map(|x| x.ok())
        .collect();
    for (id, qty, qnt) in rows {
        let exp = if qnt != 0 { qty.unwrap_or(0.0) } else { 1.0 };
        conn.execute("INSERT INTO inventory_results (session_id, item_id, expected_qty, checked) VALUES (?1,?2,?3,0)", params![sid, id, exp])?;
    }
    let session = inv_session_full(conn, sid).ok_or_else(|| ApiError::bad("ошибка"))?;
    let results = session["results"].as_array().cloned().unwrap_or_default().into_iter().filter_map(|result| Some(json!({"itemGuid":ledger::guid(conn,"items",result["itemId"].as_i64()?).ok()?,"expectedQty":result["expectedQty"]}))).collect::<Vec<_>>();
    let proof = record_inventory_event(
        conn,
        sid,
        uid,
        "create",
        None,
        json!({"number":number,"results":results}),
    )?;
    let mut session = session;
    session["recordHash"] = proof["recordHash"].clone();
    Ok(session)
}
fn inv_check(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| inv_check_atomic(conn, input, user_id))
}
fn inv_check_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "inventory")?;
    let sid = i64v(input, "sessionId").ok_or_else(|| ApiError::bad("sessionId"))?;
    let iid = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let (ws, status): (i64, String) = conn.query_row(
        "SELECT workspace_id,status FROM inventory_sessions WHERE id=?1",
        params![sid],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    require_member(conn, uid, ws)?;
    if status != "in_progress" {
        return Err(ApiError::conflict("Инвентаризация уже завершена"));
    }
    require_item_access(conn, uid, iid)?;
    let checked = b(input, "checked").unwrap_or(true);
    if checked {
        conn.execute("UPDATE inventory_results SET checked=1, actual_qty=COALESCE(?3, actual_qty) WHERE session_id=?1 AND item_id=?2",
            params![sid, iid, f64v(input,"actualQty")])?;
    } else {
        conn.execute(
            "UPDATE inventory_results SET checked=0 WHERE session_id=?1 AND item_id=?2",
            params![sid, iid],
        )?;
    }
    let expected: Option<f64> = conn
        .query_row(
            "SELECT expected_qty FROM inventory_results WHERE session_id=?1 AND item_id=?2",
            params![sid, iid],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let proof = record_inventory_event(
        conn,
        sid,
        uid,
        "check",
        Some(iid),
        json!({"expectedQty":expected,"actualQty":f64v(input,"actualQty"),"checked":checked}),
    )?;
    let mut session = inv_session_full(conn, sid).ok_or_else(|| ApiError::not_found("нет"))?;
    session["recordHash"] = proof["recordHash"].clone();
    Ok(session)
}
/// Расхождения инвентаризации не затирают историю: каждое оформляется
/// отдельной корректирующей записью журнала, а для количественных позиций
/// остаток приводится к фактическому (ТЗ §4, «Инвентаризация»).
fn apply_inventory_corrections(
    conn: &Connection,
    session_id: i64,
    ws: i64,
    uid: i64,
    number: &str,
) -> Result<usize, ApiError> {
    let mut stmt = conn.prepare(
        "SELECT r.item_id, r.expected_qty, r.actual_qty, i.quantitative, i.title, i.internal_id
         FROM inventory_results r JOIN items i ON i.id = r.item_id
         WHERE r.session_id = ?1 AND r.checked = 1 AND r.actual_qty IS NOT NULL",
    )?;
    let rows: Vec<(i64, f64, f64, i64, String, String)> = stmt
        .query_map(params![session_id], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .filter_map(|x| x.ok())
        .collect();
    drop(stmt);

    let mut corrections = 0usize;
    for (item_id, expected, actual, quantitative, title, internal_id) in rows {
        let delta = actual - expected;
        if delta.abs() < 1e-9 {
            continue;
        }
        if quantitative != 0 {
            conn.execute(
                "UPDATE items SET quantity=?1 WHERE id=?2",
                params![actual, item_id],
            )?;
        }
        ledger::append(
            conn,
            ws,
            uid,
            Some(item_id),
            "inventory",
            None,
            None,
            Some(delta),
            Some(&format!(
                "Корректировка по {number}: {internal_id} {title}, учтено {expected}, фактически {actual}"
            )),
        )
        .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
        corrections += 1;
    }
    Ok(corrections)
}

fn inv_complete(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| inv_complete_atomic(conn, input, user_id))
}

fn inv_complete_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "inventory")?;
    let sid = i64v(input, "sessionId").ok_or_else(|| ApiError::bad("sessionId"))?;
    let ws: i64 = conn.query_row(
        "SELECT workspace_id FROM inventory_sessions WHERE id=?1",
        params![sid],
        |r| r.get(0),
    )?;
    require_member(conn, uid, ws)?;
    let number: String = conn.query_row(
        "SELECT number FROM inventory_sessions WHERE id=?1",
        params![sid],
        |r| r.get(0),
    )?;
    let changed = conn.execute(
        "UPDATE inventory_sessions SET status='completed', completed_at=?1 WHERE id=?2 AND status='in_progress'",
        params![now(), sid],
    )?;
    if changed != 1 {
        return Err(ApiError::conflict("Инвентаризация уже завершена"));
    }
    let checked: i64 = conn.query_row(
        "SELECT count(*) FROM inventory_results WHERE session_id=?1 AND checked=1",
        [sid],
        |r| r.get(0),
    )?;
    let total: i64 = conn.query_row(
        "SELECT count(*) FROM inventory_results WHERE session_id=?1",
        [sid],
        |r| r.get(0),
    )?;
    let proof = record_inventory_event(
        conn,
        sid,
        uid,
        "complete",
        None,
        json!({"number":number,"checkedItems":checked,"totalItems":total}),
    )?;
    let corrections = apply_inventory_corrections(conn, sid, ws, uid, &number)?;
    let mut session = inv_session_full(conn, sid).ok_or_else(|| ApiError::not_found("нет"))?;
    session["corrections"] = json!(corrections);
    session["recordHash"] = proof["recordHash"].clone();
    Ok(session)
}

fn emit_overdue_and_stock(conn: &Connection) {
    let nows = now();
    if let Ok(mut stmt) = conn.prepare("SELECT id, workspace_id, title, responsible_user_id, due_at FROM items WHERE archived=0 AND due_at IS NOT NULL AND responsible_user_id IS NOT NULL") {
        let rows: Vec<(i64, i64, String, i64, String)> = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        }).ok().map(|x| x.filter_map(|y| y.ok()).collect()).unwrap_or_default();
        for (id, _ws, title, resp, due) in rows {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&due) {
                if dt.with_timezone(&chrono::Utc) < chrono::Utc::now() {
                    let exists: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM notifications WHERE item_id=?1 AND type='reminder' AND text LIKE '%просроч%'",
                        params![id], |r| r.get(0),
                    ).unwrap_or(0);
                    if exists == 0 {
                        let _ = conn.execute(
                            "INSERT INTO notifications (user_id, item_id, type, title, text, created_at) VALUES (?1,?2,'reminder','Просроченный возврат',?3,?4)",
                            params![resp, id, format!("Просрочен возврат: {title}"), nows.clone()],
                        );
                    }
                }
            }
        }
    }
    if let Ok(mut stmt) = conn.prepare("SELECT id, workspace_id, title, quantity, min_quantity FROM items WHERE archived=0 AND quantitative=1 AND min_quantity IS NOT NULL AND quantity IS NOT NULL AND quantity < min_quantity") {
        let rows: Vec<(i64, i64, String, f64, f64)> = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        }).ok().map(|x| x.filter_map(|y| y.ok()).collect()).unwrap_or_default();
        for (id, ws, title, qty, minq) in rows {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM notifications WHERE item_id=?1 AND title='Мало на складе' AND read=0",
                params![id], |r| r.get(0),
            ).unwrap_or(0);
            if exists == 0 {
                notify_admins(conn, ws, id, "Мало на складе", &format!("{title}: {qty} (мин. {minq})"));
            }
        }
    }
    if let Ok(mut stmt) = conn.prepare("SELECT id, workspace_id, title, calibrated_until FROM items WHERE archived=0 AND calibrated_until IS NOT NULL") {
        let rows: Vec<(i64, i64, String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .ok().map(|x| x.filter_map(|y| y.ok()).collect()).unwrap_or_default();
        for (id, ws, title, until) in rows {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&until) {
                if dt.with_timezone(&chrono::Utc) < chrono::Utc::now() {
                    let exists: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM notifications WHERE item_id=?1 AND title='Истекла поверка' AND read=0",
                        params![id], |r| r.get(0),
                    ).unwrap_or(0);
                    if exists == 0 {
                        notify_admins(conn, ws, id, "Истекла поверка", &format!("{title}: срок поверки прошёл"));
                    }
                }
            }
        }
    }
}

fn notif_list(conn: &Connection, user_id: Option<i64>) -> ApiResult {
    emit_overdue_and_stock(conn);
    let uid = require_user(conn, user_id)?;
    let mut stmt = conn.prepare("SELECT id, user_id, item_id, type, title, text, read, created_at FROM notifications WHERE user_id=?1 ORDER BY id DESC LIMIT 100")?;
    let rows: Vec<Value> = stmt
        .query_map(params![uid], |r| {
            let item_id: Option<i64> = r.get(2)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "userId": r.get::<_, i64>(1)?, "itemId": item_id,
                "type": r.get::<_, String>(3)?, "title": r.get::<_, Option<String>>(4)?,
                "text": r.get::<_, String>(5)?, "read": r.get::<_, i64>(6)? != 0,
                "createdAt": r.get::<_, String>(7)?,
                "item": item_id.and_then(|i| jsn::item_json(conn, i, false))
            }))
        })?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Value::Array(rows))
}
fn notif_unread(conn: &Connection, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE user_id=?1 AND read=0",
        params![uid],
        |r| r.get(0),
    )?;
    Ok(json!({"count": n}))
}
fn notif_mark(conn: &Connection, input: &Value, all: bool, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    if all {
        conn.execute(
            "UPDATE notifications SET read=1 WHERE user_id=?1 AND read=0",
            params![uid],
        )?;
    } else if let Some(id) = i64v(input, "id") {
        conn.execute(
            "UPDATE notifications SET read=1 WHERE id=?1 AND user_id=?2",
            params![id, uid],
        )?;
    }
    Ok(json!({"ok": true}))
}

fn reports_by_users(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut stmt = conn.prepare(
        "SELECT DISTINCT responsible_user_id FROM items WHERE workspace_id=?1 AND archived=0",
    )?;
    let uids: Vec<Option<i64>> = stmt
        .query_map(params![ws], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    let mut out = Vec::new();
    for uid in uids {
        let mut st = conn.prepare("SELECT id FROM items WHERE workspace_id=?1 AND archived=0 AND ((?2 IS NULL AND responsible_user_id IS NULL) OR responsible_user_id=?2)")?;
        let ids: Vec<i64> = st
            .query_map(params![ws, uid], |r| r.get(0))?
            .filter_map(|x| x.ok())
            .collect();
        let items: Vec<Value> = ids
            .iter()
            .filter_map(|id| jsn::item_json(conn, *id, false))
            .collect();
        let total: f64 = items
            .iter()
            .map(|i| i["cost"].as_f64().unwrap_or(0.0))
            .sum();
        out.push(json!({
            "userId": uid,
            "user": uid.and_then(|i| jsn::user_public(conn, i)),
            "itemsCount": items.len(),
            "totalCost": total,
            "items": items
        }));
    }
    Ok(Value::Array(out))
}
fn reports_all(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut stmt = conn.prepare(
        "SELECT id FROM items WHERE workspace_id=?1 AND archived=0 ORDER BY created_at DESC",
    )?;
    let ids: Vec<i64> = stmt
        .query_map(params![ws], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Value::Array(
        ids.into_iter()
            .filter_map(|id| item_for_list(conn, id))
            .collect(),
    ))
}

fn profile_get(conn: &Connection, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let mut u = jsn::user_public(conn, uid).ok_or_else(|| ApiError::unauth("нет"))?;
    let mut st = conn.prepare("SELECT workspace_id FROM user_workspaces WHERE user_id=?1")?;
    let wids: Vec<i64> = st
        .query_map(params![uid], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    u["workspaces"] = Value::Array(
        wids.into_iter()
            .filter_map(|id| jsn::workspace_json(conn, id))
            .collect(),
    );
    Ok(u)
}

fn bit_balance(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let target = i64v(input, "userId").unwrap_or(uid);
    if target != uid {
        require_can_in_workspace(conn, uid, ws, "viewAccounting")?;
    }
    let balance = crate::accounting::balance(conn, ws, target)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(json!({"workspaceId":ws,"userId":target,"currency":"BIT","minorUnit":1,"balance":balance}))
}

fn bit_transactions(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    Ok(crate::accounting::list(conn, ws))
}

fn bit_transfer(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        let uid = require_user(conn, user_id)?;
        let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
        let recipient =
            i64v(input, "recipientUserId").ok_or_else(|| ApiError::bad("recipientUserId"))?;
        let amount = i64v(input, "amount")
            .ok_or_else(|| ApiError::bad("amount должен быть целым числом Bit"))?;
        let posted = crate::accounting::post(
            conn,
            ws,
            uid,
            "transfer",
            Some(uid),
            recipient,
            amount,
            s(input, "memo").as_deref(),
            s(input, "reference").as_deref(),
        )
        .map_err(|e| ApiError::bad(e.to_string()))?;
        if posted["status"] != "posted" {
            return Err(ApiError::conflict(
                "Недостаточно подтверждённых Bit; перевод сохранён не будет",
            ));
        }
        ledger::append(
            conn,
            ws,
            uid,
            None,
            "bit_transfer",
            posted["senderAccountGuid"].as_str(),
            posted["txHash"].as_str(),
            Some(amount as f64),
            s(input, "memo").as_deref(),
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(posted)
    })
}

fn bit_mint(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        let uid = require_user(conn, user_id)?;
        let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
        require_can_in_workspace(conn, uid, ws, "manageAccounting")?;
        let recipient =
            i64v(input, "recipientUserId").ok_or_else(|| ApiError::bad("recipientUserId"))?;
        let amount = i64v(input, "amount")
            .ok_or_else(|| ApiError::bad("amount должен быть целым числом Bit"))?;
        let posted = crate::accounting::post(
            conn,
            ws,
            uid,
            "mint",
            None,
            recipient,
            amount,
            s(input, "memo").as_deref(),
            s(input, "reference").as_deref(),
        )
        .map_err(|e| ApiError::bad(e.to_string()))?;
        ledger::append(
            conn,
            ws,
            uid,
            None,
            "bit_mint",
            Some("BIT-ISSUANCE"),
            posted["txHash"].as_str(),
            Some(amount as f64),
            s(input, "memo").as_deref(),
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(posted)
    })
}

fn bit_sale(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        let buyer = require_user(conn, user_id)?;
        let item_id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
        require_item_access(conn, buyer, item_id)?;
        db::fill_guids(conn).map_err(|e| ApiError::internal(e.to_string()))?;
        let item = jsn::item_json(conn, item_id, false)
            .ok_or_else(|| ApiError::not_found("Товар не найден"))?;
        let ws = item["workspaceId"]
            .as_i64()
            .ok_or_else(|| ApiError::bad("У товара нет организации"))?;
        let seller = i64v(input, "sellerUserId").ok_or_else(|| ApiError::bad("sellerUserId"))?;
        let amount = i64v(input, "amount")
            .ok_or_else(|| ApiError::bad("amount должен быть целым числом Bit"))?;
        let item_guid = item["guid"]
            .as_str()
            .ok_or_else(|| ApiError::bad("У товара нет GUID"))?;
        let posted = crate::accounting::post(
            conn,
            ws,
            buyer,
            "sale",
            Some(buyer),
            seller,
            amount,
            s(input, "memo").as_deref(),
            Some(item_guid),
        )
        .map_err(|e| ApiError::bad(e.to_string()))?;
        if posted["status"] != "posted" {
            return Err(ApiError::conflict(
                "Недостаточно подтверждённых Bit для покупки",
            ));
        }
        ledger::append(
            conn,
            ws,
            buyer,
            Some(item_id),
            "bit_sale",
            posted["senderAccountGuid"].as_str(),
            posted["txHash"].as_str(),
            Some(amount as f64),
            s(input, "memo").as_deref(),
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(posted)
    })
}

fn knowledge_visible(rights: &Value, page: &Value) -> bool {
    match page
        .get("visibility")
        .and_then(Value::as_str)
        .unwrap_or("members")
    {
        "accounting" => rights
            .get("viewAccounting")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "managers" => rights
            .get("editKnowledge")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        _ => true,
    }
}

fn knowledge_list(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let rights = merged_rights(conn, uid, ws);
    let mut pages = crate::knowledge::list(conn, ws);
    if let Some(items) = pages.as_array_mut() {
        items.retain(|page| knowledge_visible(&rights, page));
    }
    Ok(pages)
}

fn knowledge_by_slug(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let slug = s(input, "slug").ok_or_else(|| ApiError::bad("slug"))?;
    let page = crate::knowledge::page(conn, ws, &slug)
        .ok_or_else(|| ApiError::not_found("Страница не найдена"))?;
    if !knowledge_visible(&merged_rights(conn, uid, ws), &page) {
        return Err(ApiError::not_found("Страница не найдена"));
    }
    Ok(page)
}

fn knowledge_save(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        let uid = require_user(conn, user_id)?;
        let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
        require_can_in_workspace(conn, uid, ws, "editKnowledge")?;
        db::fill_guids(conn).map_err(|e| ApiError::internal(e.to_string()))?;
        let slug = s(input, "slug").ok_or_else(|| ApiError::bad("slug"))?;
        let title = s(input, "title").ok_or_else(|| ApiError::bad("title"))?;
        let content = s(input, "content").unwrap_or_default();
        let visibility = s(input, "visibility").unwrap_or_else(|| "members".into());
        let page = crate::knowledge::save(
            conn,
            ws,
            uid,
            &slug,
            &title,
            &content,
            &visibility,
            s(input, "parentRevisionGuid").as_deref(),
            input.get("attachments").unwrap_or(&Value::Null),
        )
        .map_err(|e| ApiError::bad(e.to_string()))?;
        let page_guid = page["guid"]
            .as_str()
            .ok_or_else(|| ApiError::internal("нет GUID страницы"))?;
        let revision_hash = page["savedRevisionHash"]
            .as_str()
            .ok_or_else(|| ApiError::internal("нет hash ревизии"))?;
        ledger::append(
            conn,
            ws,
            uid,
            None,
            "knowledge_revision",
            Some(page_guid),
            Some(revision_hash),
            None,
            Some(&format!("База знаний: {title}")),
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(page)
    })
}
fn profile_update(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    if let Some(phone) = s(input, "phone") {
        validate_phone(&phone)?;
    }
    if s(input, "fullName").is_some_and(|name| name.chars().count() > 200) {
        return Err(ApiError::bad("Имя слишком длинное"));
    }
    conn.execute("UPDATE users SET full_name=COALESCE(?2,full_name), position=COALESCE(?3,position), phone=COALESCE(?4,phone), avatar_url=COALESCE(?5,avatar_url) WHERE id=?1",
        params![uid, s(input,"fullName"), s(input,"position"), s(input,"phone"), s(input,"avatarUrl")])?;
    jsn::user_public(conn, uid).ok_or_else(|| ApiError::not_found("нет"))
}
fn profile_password(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let newp = s(input, "newPassword").ok_or_else(|| ApiError::bad("newPassword"))?;
    validate_new_password(&newp)?;
    let old = conn
        .query_row(
            "SELECT password_hash FROM users WHERE id=?1",
            params![uid],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();
    if let Some(h) = old.filter(|x| !x.is_empty()) {
        let cur = s(input, "currentPassword").unwrap_or_default();
        if !verify_password(&cur, &h) {
            return Err(ApiError::unauth("Неверный текущий пароль"));
        }
    }
    conn.execute(
        "UPDATE users SET password_hash=?1 WHERE id=?2",
        params![hash_password(&newp), uid],
    )?;
    conn.execute(
        "UPDATE sessions SET revoked_at=?1 WHERE user_id=?2 AND revoked_at IS NULL",
        params![now(), uid],
    )?;
    Ok(json!({"ok": true, "message": "Пароль изменён"}))
}

fn admin_users(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut stmt = conn.prepare("SELECT user_id,position,role_name,personnel_number,rights_json FROM user_workspaces WHERE workspace_id=?1")?;
    let rows = stmt.query_map(params![ws], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for (id, position, role_name, personnel_number, rights) in rows.flatten() {
        if let Some(mut user) = jsn::user_public(conn, id) {
            user["globalPosition"] = user.get("position").cloned().unwrap_or(Value::Null);
            if position.is_some() {
                user["position"] = json!(position);
            }
            user["organizationRole"] = json!(role_name);
            user["personnelNumber"] = json!(personnel_number);
            if let Some(rights) = rights.and_then(|value| serde_json::from_str(&value).ok()) {
                user["roleRights"] = rights;
            }
            out.push(user);
        }
    }
    Ok(Value::Array(out))
}
fn admin_user_create(conn: &mut Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    atomic(conn, |conn| admin_user_create_atomic(conn, input, actor))
}

fn admin_user_create_atomic(conn: &Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    let actor = require_user(conn, actor)?;
    let name = s(input, "fullName").ok_or_else(|| ApiError::bad("fullName"))?;
    let phone = s(input, "phone").ok_or_else(|| ApiError::bad("phone"))?;
    conn.execute(
        "INSERT INTO users (full_name, position, phone, status, role_rights, created_at) VALUES (?1,?2,?3,'invited',?4,?5)",
        params![name, s(input,"position"), phone, db::default_rights().to_string(), now()],
    ).map_err(|e| ApiError::conflict(e.to_string()))?;
    let uid = conn.last_insert_rowid();
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    conn.execute(
        "INSERT INTO user_workspaces(user_id,workspace_id,rights_json,position,role_name,personnel_number) VALUES (?1,?2,?3,?4,?5,?6)",
        params![uid,ws,db::default_rights().to_string(),s(input,"position"),s(input,"organizationRole"),s(input,"personnelNumber")],
    )?;
    let user_guid = ledger::guid(conn, "users", uid)
        .map_err(|error| ApiError::internal(format!("Ошибка GUID: {error}")))?;
    let event = ledger::append(
        conn,
        ws,
        actor,
        None,
        "membership_create",
        None,
        Some(&user_guid),
        None,
        Some(&format!("Добавлен участник: {name}")),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    crate::sync::record_membership_version(conn, ws, uid, true, event["opId"].as_str(), true)
        .map_err(|error| ApiError::internal(format!("Ошибка версии членства: {error}")))?;
    jsn::user_public(conn, uid).ok_or_else(|| ApiError::bad("ошибка"))
}
fn admin_user_update(conn: &mut Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    atomic(conn, |conn| admin_user_update_atomic(conn, input, actor))
}

fn admin_user_update_atomic(conn: &Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    let actor = require_user(conn, actor)?;
    require_can(conn, actor, "manageUsers")?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let before = jsn::user_public(conn, id).ok_or_else(|| ApiError::not_found("нет"))?;
    conn.execute("UPDATE users SET full_name=COALESCE(?2,full_name), position=COALESCE(?3,position), phone=COALESCE(?4,phone), status=COALESCE(?5,status) WHERE id=?1",
        params![id, s(input,"fullName"), s(input,"position"), s(input,"phone"), s(input,"status")])?;
    if let Some(rr) = input.get("roleRights") {
        if !rr.is_null() {
            let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
            conn.execute(
                "UPDATE user_workspaces SET rights_json=?1 WHERE user_id=?2 AND workspace_id=?3",
                params![rr.to_string(), id, ws],
            )?;
        }
    }
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    conn.execute(
        "UPDATE user_workspaces SET position=COALESCE(?1,position),role_name=COALESCE(?2,role_name),personnel_number=COALESCE(?3,personnel_number) WHERE user_id=?4 AND workspace_id=?5",
        params![s(input,"position"),s(input,"organizationRole"),s(input,"personnelNumber"),id,ws],
    )?;
    if let Some(cp) = input.get("checkoutPolicy") {
        if !cp.is_null() {
            conn.execute(
                "UPDATE users SET checkout_policy=?1 WHERE id=?2",
                params![cp.to_string(), id],
            )?;
        }
    }
    let updated = jsn::user_public(conn, id).ok_or_else(|| ApiError::not_found("нет"))?;
    let target_guid = ledger::guid(conn, "users", id)
        .map_err(|error| ApiError::internal(format!("Ошибка GUID: {error}")))?;
    let event = ledger::append(
        conn,
        ws,
        actor,
        None,
        "membership_update",
        Some(&target_guid),
        Some(&target_guid),
        None,
        Some(&format!(
            "Изменёны права/профиль: {}",
            updated["fullName"]
                .as_str()
                .or_else(|| before["fullName"].as_str())
                .unwrap_or("участник")
        )),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    crate::sync::record_membership_version(conn, ws, id, true, event["opId"].as_str(), false)
        .map_err(|error| ApiError::internal(format!("Ошибка версии членства: {error}")))?;
    Ok(updated)
}
/// Исключение участника. Историю и подписанные блоки трогать нельзя (ТЗ §7—8):
/// если за человеком что-то числится, он блокируется и выводится из пространства,
/// а не стирается вместе со следами своих операций.
fn admin_user_remove(conn: &mut Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    atomic(conn, |conn| admin_user_remove_atomic(conn, input, actor))
}

fn admin_user_remove_atomic(conn: &Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    let uid = require_user(conn, actor)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    if id == uid {
        return Err(ApiError::bad("Нельзя удалить собственную учётную запись"));
    }
    let exists: i64 =
        conn.query_row("SELECT COUNT(*) FROM users WHERE id=?1", params![id], |r| {
            r.get(0)
        })?;
    if exists == 0 {
        return Err(ApiError::not_found("Пользователь не найден"));
    }
    let traces: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM history_entries WHERE actor_user_id=?1)
              + (SELECT COUNT(*) FROM items WHERE responsible_user_id=?1)
              + (SELECT COUNT(*) FROM item_holdings WHERE user_id=?1 AND returned_at IS NULL)
              + (SELECT COUNT(*) FROM transfers WHERE from_user_id=?1 OR to_user_id=?1)",
        params![id],
        |r| r.get(0),
    )?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let target_guid = ledger::guid(conn, "users", id)
        .map_err(|error| ApiError::internal(format!("Ошибка GUID: {error}")))?;
    let removal_comment = if traces == 0 {
        "Участник исключён; учётная запись не имеет операционных следов"
    } else {
        "Участник исключён из организации; история сохранена"
    };
    let event = ledger::append(
        conn,
        ws,
        uid,
        None,
        "membership_remove",
        Some(&target_guid),
        None,
        None,
        Some(removal_comment),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    crate::sync::record_membership_version(conn, ws, id, false, event["opId"].as_str(), false)
        .map_err(|error| ApiError::internal(format!("Ошибка tombstone членства: {error}")))?;
    conn.execute(
        "DELETE FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
        params![id, ws],
    )?;
    let other_workspaces: i64 = conn.query_row(
        "SELECT COUNT(*) FROM user_workspaces WHERE user_id=?1",
        params![id],
        |r| r.get(0),
    )?;
    if other_workspaces == 0 {
        conn.execute(
            "UPDATE users SET status='disabled' WHERE id=?1",
            params![id],
        )?;
        conn.execute(
            "UPDATE sessions SET revoked_at=?1 WHERE user_id=?2 AND revoked_at IS NULL",
            params![now(), id],
        )?;
    }
    if traces == 0 && other_workspaces == 0 {
        conn.execute("DELETE FROM users WHERE id=?1", params![id])?;
        return Ok(json!({"ok": true, "deleted": true}));
    }
    Ok(json!({
        "ok": true,
        "deleted": false,
        "disabled": other_workspaces == 0,
        "message": "Участник исключён из пространства; история его операций сохранена"
    }))
}

fn admin_user_invite(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| admin_user_invite_atomic(conn, input, user_id))
}

fn admin_user_invite_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let created = admin_user_create_atomic(conn, input, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let token = Uuid::new_v4().to_string().replace('-', "");
    let expires_at = invite_expiry(input);
    conn.execute(
        "INSERT INTO invites (workspace_id, token, role, created_by, max_uses, expires_at, created_at) VALUES (?1,?2,'member',?3,20,?4,?5)",
        params![ws, token, user_id, expires_at, now()],
    )?;
    Ok(json!({"user": created, "token": token, "expiresAt": expires_at}))
}

fn ws_create(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| ws_create_atomic(conn, input, user_id))
}

fn ws_create_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    conn.execute(
        "INSERT INTO workspaces (name, timezone, internal_id_prefix, comment, created_at, sync_url, guid) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![s(input,"name").unwrap_or("Группа".into()), s(input,"timezone").unwrap_or("Europe/Moscow".into()), s(input,"internalIdPrefix").unwrap_or("ВН-".into()), s(input,"comment"), now(), s(input,"syncUrl"), Uuid::new_v4().to_string()],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO user_workspaces (user_id, workspace_id, rights_json) VALUES (?1,?2,?3)",
        params![uid, id, db::owner_rights().to_string()],
    )?;
    seed_workspace_defaults(conn, id, uid)?;
    if let Some(url) = s(input, "syncUrl") {
        crate::sync::add_peer(conn, &url, Some("relay"), None);
    }
    let workspace = jsn::workspace_json(conn, id).ok_or_else(|| ApiError::bad("ошибка"))?;
    let event = ledger::append(
        conn,
        id,
        uid,
        None,
        "workspace_create",
        None,
        workspace["guid"].as_str(),
        None,
        Some(&format!(
            "Создана организация: {}",
            workspace["name"].as_str().unwrap_or("организация")
        )),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    crate::sync::record_membership_version(conn, id, uid, true, event["opId"].as_str(), true)
        .map_err(|error| ApiError::internal(format!("Ошибка версии членства: {error}")))?;
    Ok(workspace)
}
fn ws_update(conn: &mut Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    atomic(conn, |conn| ws_update_atomic(conn, input, actor))
}

fn ws_update_atomic(conn: &Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    let actor = require_user(conn, actor)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let before = jsn::workspace_json(conn, id).ok_or_else(|| ApiError::not_found("нет"))?;
    conn.execute("UPDATE workspaces SET name=COALESCE(?2,name), timezone=COALESCE(?3,timezone), internal_id_prefix=COALESCE(?4,internal_id_prefix), comment=?5, sync_url=COALESCE(?6,sync_url), require_writeoff_photo=CASE WHEN ?7 THEN ?8 ELSE require_writeoff_photo END WHERE id=?1",
        params![id, s(input,"name"), s(input,"timezone"), s(input,"internalIdPrefix"), s(input,"comment"), s(input,"syncUrl"),
                input.get("requireWriteoffPhoto").is_some(), b(input,"requireWriteoffPhoto").unwrap_or(false) as i64])?;
    if let Some(url) = s(input, "syncUrl") {
        crate::sync::add_peer(conn, &url, Some("relay"), None);
    }
    let updated = jsn::workspace_json(conn, id).ok_or_else(|| ApiError::not_found("нет"))?;
    ledger::append(
        conn,
        id,
        actor,
        None,
        "workspace_update",
        before["name"].as_str(),
        updated["name"].as_str(),
        None,
        Some("Изменены настройки организации"),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    Ok(updated)
}
/// Срок жизни приглашения по умолчанию — неделя (ТЗ: у приглашения есть срок действия).
const INVITE_DEFAULT_TTL_HOURS: i64 = 168;
const INVITE_MAX_TTL_HOURS: i64 = 24 * 365;

fn invite_expiry(input: &Value) -> String {
    let hours = i64v(input, "expiresInHours")
        .unwrap_or(INVITE_DEFAULT_TTL_HOURS)
        .clamp(1, INVITE_MAX_TTL_HOURS);
    (chrono::Utc::now() + chrono::Duration::hours(hours)).to_rfc3339()
}

/// Должность по умолчанию для участника, вступившего по приглашению с ролью.
fn invite_position(role: &str) -> &'static str {
    match role.trim().to_lowercase().as_str() {
        "owner" | "владелец" => "Владелец",
        "admin" | "администратор" => "Администратор",
        "viewer" | "observer" | "наблюдатель" => "Наблюдатель",
        _ => "Участник",
    }
}

fn ws_create_invite(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| ws_create_invite_atomic(conn, input, user_id))
}

fn ws_create_invite_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let actor = require_user(conn, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let token = Uuid::new_v4().to_string().replace('-', "");
    let role = s(input, "role").unwrap_or_else(|| "member".into());
    let expires_at = invite_expiry(input);
    conn.execute(
        "INSERT INTO invites (workspace_id, token, role, created_by, max_uses, expires_at, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![ws, token, role, user_id, i64v(input,"maxUses").unwrap_or(20), expires_at, now()],
    )?;
    ledger::append(
        conn,
        ws,
        actor,
        None,
        "invitation_create",
        None,
        Some(&role),
        None,
        Some(&format!(
            "Создано приглашение; maxUses={}, expiresAt={expires_at}",
            i64v(input, "maxUses").unwrap_or(20)
        )),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    let wsj = jsn::workspace_json(conn, ws).unwrap_or(json!({}));
    Ok(json!({
        "token": token,
        "workspaceId": ws,
        "role": role,
        "expiresAt": expires_at,
        "workspace": wsj,
        "payload": {
            "v": 1,
            "t": "join",
            "ws": ws,
            "token": token,
            "role": role,
            "exp": expires_at,
            "name": wsj.get("name"),
            "server": wsj.get("syncUrl")
        }
    }))
}
fn ws_invites(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut stmt = conn.prepare("SELECT id, token, role, max_uses, used_count, revoked, created_at, expires_at FROM invites WHERE workspace_id=?1 AND revoked=0 ORDER BY id DESC")?;
    let rows: Vec<Value> = stmt
        .query_map(params![ws], |r| {
            let invite = Invite {
                id: r.get(0)?,
                workspace_id: ws,
                role: r.get(2)?,
                max_uses: r.get(3)?,
                used_count: r.get(4)?,
                revoked: r.get(5)?,
                expires_at: r.get(7)?,
            };
            Ok(json!({
                "id": invite.id, "token": r.get::<_, String>(1)?, "role": invite.role,
                "maxUses": invite.max_uses, "usedCount": invite.used_count,
                "revoked": invite.revoked != 0, "createdAt": r.get::<_, String>(6)?,
                "expiresAt": invite.expires_at,
                "expired": invite.is_expired(),
                "usable": ensure_invite_usable(&invite).is_ok(),
            }))
        })?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Value::Array(rows))
}

fn config_kind_table(kind: &str) -> Result<&'static str, ApiError> {
    match kind {
        "storage" => Ok("storages"),
        "site" => Ok("building_sites"),
        "category" => Ok("categories"),
        "brand" => Ok("brands"),
        "status" => Ok("statuses"),
        _ => Err(ApiError::bad("Некорректный тип конфигурации")),
    }
}

fn config_fields(
    conn: &Connection,
    kind: &str,
    id: i64,
) -> Result<(i64, String, Value, bool), ApiError> {
    let table = config_kind_table(kind)?;
    let common = format!("SELECT workspace_id,guid,name,archived FROM {table} WHERE id=?1");
    let (ws, guid, name, archived): (i64, String, String, i64) = conn
        .query_row(&common, [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(|_| ApiError::not_found("Элемент конфигурации не найден"))?;
    let fields = match kind {
        "storage" => {
            let (responsible, address): (Option<i64>, Option<String>) = conn.query_row(
                "SELECT responsible_user_id,address FROM storages WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            json!({"name":name,"responsibleGuid":responsible.map(|uid|ledger::guid(conn,"users",uid)).transpose().map_err(|e|ApiError::internal(e.to_string()))?,"address":address})
        }
        "site" => {
            let responsible: Option<i64> = conn.query_row(
                "SELECT responsible_user_id FROM building_sites WHERE id=?1",
                [id],
                |r| r.get(0),
            )?;
            json!({"name":name,"responsibleGuid":responsible.map(|uid|ledger::guid(conn,"users",uid)).transpose().map_err(|e|ApiError::internal(e.to_string()))?})
        }
        "category" | "brand" => {
            let description: Option<String> = conn.query_row(
                &format!("SELECT description FROM {table} WHERE id=?1"),
                [id],
                |r| r.get(0),
            )?;
            json!({"name":name,"description":description})
        }
        "status" => {
            let (description, slug, color, bg): (
                Option<String>,
                String,
                Option<String>,
                Option<String>,
            ) = conn.query_row(
                "SELECT description,slug,color,bg FROM statuses WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            json!({"name":name,"description":description,"slug":slug,"color":color,"bg":bg})
        }
        _ => unreachable!(),
    };
    Ok((ws, guid, fields, archived != 0))
}

fn config_payload_hash(
    kind: &str,
    guid: &str,
    parent: Option<&str>,
    depth: i64,
    workspace: &str,
    actor: &str,
    active: bool,
    fields: &Value,
    updated_at: &str,
) -> String {
    let payload = json!({"domain":"everyday/config-version/v1","kind":kind,"entityGuid":guid,"parentHash":parent,"depth":depth,"workspaceGuid":workspace,"actorGuid":actor,"active":active,"fields":fields,"updatedAt":updated_at});
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload).expect("JSON serialization"))
    )
}

fn config_version_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/config-version-ledger/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}

fn validate_config_responsible(
    conn: &Connection,
    workspace_id: i64,
    responsible_user_id: Option<i64>,
) -> Result<(), ApiError> {
    let Some(user_id) = responsible_user_id else {
        return Ok(());
    };
    let member: i64 = conn.query_row(
        "SELECT count(*) FROM user_workspaces WHERE workspace_id=?1 AND user_id=?2 AND removed_at IS NULL",
        params![workspace_id,user_id], |row| row.get(0),
    )?;
    if member == 0 {
        return Err(ApiError::bad("Ответственный не состоит в этой организации"));
    }
    Ok(())
}

fn record_config_version(
    conn: &Connection,
    kind: &str,
    id: i64,
    uid: i64,
    operation: &str,
) -> Result<Value, ApiError> {
    let (ws, guid, fields, archived) = config_fields(conn, kind, id)?;
    let parent:Option<(String,i64)>=conn.query_row("SELECT version_hash,depth FROM config_versions WHERE kind=?1 AND entity_guid=?2 ORDER BY depth DESC,version_hash DESC LIMIT 1",params![kind,guid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let (parent_hash, depth, event_type) = match (parent, operation) {
        (Some((hash, depth)), "archive") => (Some(hash), depth + 1, "config_archive"),
        (Some((hash, depth)), _) => (Some(hash), depth + 1, "config_update"),
        (None, "create") => (None, 0, "config_create"),
        (None, _) => (None, 0, "config_adopt"),
    };
    let workspace_guid =
        ledger::guid(conn, "workspaces", ws).map_err(|e| ApiError::internal(e.to_string()))?;
    let actor_guid =
        ledger::guid(conn, "users", uid).map_err(|e| ApiError::internal(e.to_string()))?;
    let updated_at = now();
    let payload_hash = config_payload_hash(
        kind,
        &guid,
        parent_hash.as_deref(),
        depth,
        &workspace_guid,
        &actor_guid,
        !archived,
        &fields,
        &updated_at,
    );
    let event = ledger::append(
        conn,
        ws,
        uid,
        None,
        event_type,
        Some(&guid),
        Some(&payload_hash),
        None,
        Some(&format!(
            "Конфигурация {kind}: {}",
            fields["name"].as_str().unwrap_or("элемент")
        )),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    let ledger_hash = event["opId"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?;
    let version_hash = config_version_hash(&payload_hash, ledger_hash);
    conn.execute("INSERT INTO config_versions(version_hash,entity_guid,kind,parent_hash,depth,workspace_guid,actor_guid,active,fields_json,payload_hash,ledger_hash,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![version_hash,guid,kind,parent_hash,depth,workspace_guid,actor_guid,!archived,fields.to_string(),payload_hash,ledger_hash,updated_at])?;
    Ok(
        json!({"guid":guid,"versionHash":version_hash,"ledgerHash":ledger_hash,"depth":depth,"active":!archived}),
    )
}

fn storages_list(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut stmt = conn.prepare("SELECT id FROM storages WHERE workspace_id=?1 AND archived=0")?;
    let ids: Vec<i64> = stmt
        .query_map(params![ws], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    let mut out = Vec::new();
    for id in ids {
        let mut v = jsn::storage_obj(conn, Some(id));
        if let Some(uid) = v.get("responsibleUserId").and_then(|x| x.as_i64()) {
            v["responsible"] = jsn::user_public(conn, uid).unwrap_or(Value::Null);
        }
        out.push(v);
    }
    Ok(Value::Array(out))
}
fn storage_create(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| storage_create_atomic(conn, input, user_id))
}
fn storage_create_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let name = s(input, "name").unwrap_or("Склад".into());
    validate_node_text(&name, "Название", 120)?;
    validate_config_responsible(conn, ws, i64v(input, "responsibleUserId"))?;
    conn.execute("INSERT INTO storages(name,responsible_user_id,workspace_id,address,guid) VALUES(?1,?2,?3,?4,?5)",params![name,i64v(input,"responsibleUserId"),ws,s(input,"address"),Uuid::new_v4().to_string()])?;
    let id = conn.last_insert_rowid();
    let proof = record_config_version(conn, "storage", id, uid, "create")?;
    let mut result = jsn::storage_obj(conn, Some(id));
    result["guid"] = proof["guid"].clone();
    result["versionHash"] = proof["versionHash"].clone();
    Ok(result)
}
fn storage_update(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| storage_update_atomic(conn, input, user_id))
}
fn storage_update_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    if let Some(name) = s(input, "name") {
        validate_node_text(&name, "Название", 120)?;
    }
    let ws: i64 = conn.query_row(
        "SELECT workspace_id FROM storages WHERE id=?1 AND archived=0",
        [id],
        |r| r.get(0),
    )?;
    if input.get("responsibleUserId").is_some() {
        validate_config_responsible(conn, ws, i64v(input, "responsibleUserId"))?;
    }
    conn.execute("UPDATE storages SET name=COALESCE(?2,name),responsible_user_id=CASE WHEN ?3 THEN ?4 ELSE responsible_user_id END,address=CASE WHEN ?5 THEN ?6 ELSE address END WHERE id=?1 AND archived=0",params![id,s(input,"name"),input.get("responsibleUserId").is_some(),i64v(input,"responsibleUserId"),input.get("address").is_some(),s(input,"address")])?;
    let proof = record_config_version(conn, "storage", id, uid, "update")?;
    let mut result = jsn::storage_obj(conn, Some(id));
    result["guid"] = proof["guid"].clone();
    result["versionHash"] = proof["versionHash"].clone();
    Ok(result)
}
fn storage_remove(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        let uid = require_user(conn, user_id)?;
        let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
        let used: i64 = conn.query_row(
            "SELECT count(*) FROM items WHERE storage_id=?1 AND archived=0",
            [id],
            |r| r.get(0),
        )?;
        if used > 0 {
            return Err(ApiError::conflict("Склад используется в карточках ТМЦ"));
        }
        conn.execute("UPDATE storages SET archived=1 WHERE id=?1", [id])?;
        let proof = record_config_version(conn, "storage", id, uid, "archive")?;
        Ok(
            json!({"ok":true,"archived":true,"id":id,"guid":proof["guid"],"versionHash":proof["versionHash"]}),
        )
    })
}
fn sites_list(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut stmt=conn.prepare("SELECT id,name,responsible_user_id,workspace_id,guid FROM building_sites WHERE workspace_id=?1 AND archived=0")?;
    let rows: Vec<Value> = stmt
        .query_map(params![ws], |r| {
            let uid: Option<i64> = r.get(2)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?,
                "responsibleUserId": uid, "workspaceId": r.get::<_, i64>(3)?,
                "responsible":uid.and_then(|i|jsn::user_public(conn,i)),"guid":r.get::<_,Option<String>>(4)?
            }))
        })?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Value::Array(rows))
}
fn site_create(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| site_create_atomic(conn, input, user_id))
}
fn site_create_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let name = s(input, "name").unwrap_or("Объект".into());
    validate_node_text(&name, "Название", 120)?;
    validate_config_responsible(conn, ws, i64v(input, "responsibleUserId"))?;
    conn.execute(
        "INSERT INTO building_sites(name,responsible_user_id,workspace_id,guid) VALUES(?1,?2,?3,?4)",
        params![
            name,
            i64v(input, "responsibleUserId"),
            ws,Uuid::new_v4().to_string()
        ],
    )?;
    let id = conn.last_insert_rowid();
    let proof = record_config_version(conn, "site", id, uid, "create")?;
    Ok(
        json!({"id":id,"guid":proof["guid"],"versionHash":proof["versionHash"],"name":s(input,"name"),"workspaceId":ws,"responsibleUserId":i64v(input,"responsibleUserId")}),
    )
}
fn site_update(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| site_update_atomic(conn, input, user_id))
}
fn site_update_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    if let Some(name) = s(input, "name") {
        validate_node_text(&name, "Название", 120)?;
    }
    let ws: i64 = conn.query_row(
        "SELECT workspace_id FROM building_sites WHERE id=?1 AND archived=0",
        [id],
        |r| r.get(0),
    )?;
    if input.get("responsibleUserId").is_some() {
        validate_config_responsible(conn, ws, i64v(input, "responsibleUserId"))?;
    }
    conn.execute(
        "UPDATE building_sites SET name=COALESCE(?2,name),responsible_user_id=CASE WHEN ?3 THEN ?4 ELSE responsible_user_id END WHERE id=?1 AND archived=0",
        params![id,s(input,"name"),input.get("responsibleUserId").is_some(),i64v(input,"responsibleUserId")],
    )?;
    let proof = record_config_version(conn, "site", id, uid, "update")?;
    Ok(
        json!({"id":id,"guid":proof["guid"],"versionHash":proof["versionHash"],"name":s(input,"name")}),
    )
}
fn site_remove(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        let uid = require_user(conn, user_id)?;
        let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
        let used: i64 = conn.query_row(
            "SELECT count(*) FROM items WHERE building_site_id=?1 AND archived=0",
            [id],
            |r| r.get(0),
        )?;
        if used > 0 {
            return Err(ApiError::conflict("Объект используется в карточках ТМЦ"));
        }
        conn.execute("UPDATE building_sites SET archived=1 WHERE id=?1", [id])?;
        let proof = record_config_version(conn, "site", id, uid, "archive")?;
        Ok(
            json!({"ok":true,"archived":true,"id":id,"guid":proof["guid"],"versionHash":proof["versionHash"]}),
        )
    })
}

fn organization_node_json(conn: &Connection, id: i64) -> Option<Value> {
    conn.query_row(
        "SELECT id,guid,workspace_id,parent_id,kind,name,tab_label,responsible_user_id,display_order,color,icon,archived,created_at,updated_at
         FROM organization_nodes WHERE id=?1",
        [id],
        |r| {
            let responsible: Option<i64> = r.get(7)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "guid": r.get::<_, String>(1)?,
                "workspaceId": r.get::<_, i64>(2)?, "parentId": r.get::<_, Option<i64>>(3)?,
                "kind": r.get::<_, String>(4)?, "name": r.get::<_, String>(5)?,
                "tabLabel": r.get::<_, Option<String>>(6)?, "responsibleUserId": responsible,
                "displayOrder": r.get::<_, i64>(8)?, "color": r.get::<_, Option<String>>(9)?,
                "icon": r.get::<_, Option<String>>(10)?, "archived": r.get::<_, i64>(11)? != 0,
                "createdAt": r.get::<_, String>(12)?, "updatedAt": r.get::<_, String>(13)?,
                "responsible": responsible.and_then(|uid| jsn::user_public(conn, uid)),
            }))
        },
    ).optional().ok().flatten()
}

fn organization_nodes_list(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let include_archived = input
        .get("includeArchived")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut stmt = conn.prepare(
        "SELECT id FROM organization_nodes WHERE workspace_id=?1 AND (?2=1 OR archived=0)
         ORDER BY COALESCE(parent_id,0),display_order,id",
    )?;
    let ids: Vec<i64> = stmt
        .query_map(params![ws, include_archived], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect();
    Ok(Value::Array(
        ids.into_iter()
            .filter_map(|id| organization_node_json(conn, id))
            .collect(),
    ))
}

fn validate_node_text(value: &str, field: &str, max: usize) -> Result<(), ApiError> {
    let len = value.trim().chars().count();
    if len == 0 || len > max {
        return Err(ApiError::bad(format!(
            "{field}: требуется от 1 до {max} символов"
        )));
    }
    Ok(())
}

fn validate_node_parent(
    conn: &Connection,
    ws: i64,
    id: Option<i64>,
    parent: Option<i64>,
) -> Result<(), ApiError> {
    let Some(parent) = parent else { return Ok(()) };
    if id == Some(parent) {
        return Err(ApiError::bad("Раздел не может быть родителем самого себя"));
    }
    let parent_ws: Option<i64> = conn
        .query_row(
            "SELECT workspace_id FROM organization_nodes WHERE id=?1 AND archived=0",
            [parent],
            |r| r.get(0),
        )
        .optional()?;
    if parent_ws != Some(ws) {
        return Err(ApiError::bad(
            "Родительский раздел не найден в этой организации",
        ));
    }
    if let Some(id) = id {
        let cycle: i64 = conn.query_row(
            "WITH RECURSIVE descendants(id) AS (
               SELECT id FROM organization_nodes WHERE parent_id=?1
               UNION ALL SELECT n.id FROM organization_nodes n JOIN descendants d ON n.parent_id=d.id
             ) SELECT COUNT(*) FROM descendants WHERE id=?2",
            params![id, parent], |r| r.get(0),
        )?;
        if cycle != 0 {
            return Err(ApiError::bad(
                "Нельзя переместить раздел внутрь его дочернего раздела",
            ));
        }
    }
    Ok(())
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
fn record_organization_node_version(
    conn: &Connection,
    id: i64,
    uid: i64,
    operation: &str,
) -> Result<Value, ApiError> {
    let node =
        organization_node_json(conn, id).ok_or_else(|| ApiError::not_found("Раздел не найден"))?;
    let ws = node["workspaceId"]
        .as_i64()
        .ok_or_else(|| ApiError::internal("Нет организации"))?;
    let guid = node["guid"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Нет GUID раздела"))?;
    validate_config_responsible(conn, ws, node["responsibleUserId"].as_i64())?;
    let parent_guid = node["parentId"]
        .as_i64()
        .and_then(|parent| ledger::guid(conn, "organization_nodes", parent).ok());
    let responsible_guid = node["responsibleUserId"]
        .as_i64()
        .and_then(|user| ledger::guid(conn, "users", user).ok());
    let fields = json!({"parentGuid":parent_guid,"kind":node["kind"],"name":node["name"],"tabLabel":node["tabLabel"],"responsibleGuid":responsible_guid,"displayOrder":node["displayOrder"],"color":node["color"],"icon":node["icon"]});
    let parent:Option<(String,i64)>=conn.query_row("SELECT version_hash,depth FROM organization_node_versions WHERE node_guid=?1 ORDER BY depth DESC,version_hash DESC LIMIT 1",[guid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let (parent_hash, depth, event_type) = match (parent, operation) {
        (Some((hash, depth)), "archive") => (Some(hash), depth + 1, "organization_node_archive"),
        (Some((hash, depth)), _) => (Some(hash), depth + 1, "organization_node_update"),
        (None, "create") => (None, 0, "organization_node_create"),
        (None, _) => (None, 0, "organization_node_adopt"),
    };
    let workspace_guid = ledger::guid(conn, "workspaces", ws)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let actor_guid =
        ledger::guid(conn, "users", uid).map_err(|error| ApiError::internal(error.to_string()))?;
    let updated_at = now();
    let payload = json!({"domain":"everyday/organization-node/v1","nodeGuid":guid,"parentHash":parent_hash,"depth":depth,"workspaceGuid":workspace_guid,"actorGuid":actor_guid,"active":!node["archived"].as_bool().unwrap_or(false),"fields":fields,"updatedAt":updated_at});
    let payload_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload).expect("JSON serialization"))
    );
    let event = ledger::append(
        conn,
        ws,
        uid,
        None,
        event_type,
        Some(guid),
        Some(&payload_hash),
        None,
        Some(&format!(
            "Структура: {}",
            node["name"].as_str().unwrap_or("раздел")
        )),
    )
    .map_err(|error| ApiError::internal(format!("Ошибка журнала: {error}")))?;
    let ledger_hash = event["opId"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?;
    let version_hash = organization_node_version_hash(&payload_hash, ledger_hash);
    conn.execute("INSERT INTO organization_node_versions(version_hash,node_guid,parent_hash,depth,workspace_guid,actor_guid,active,fields_json,payload_hash,ledger_hash,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![version_hash,guid,parent_hash,depth,workspace_guid,actor_guid,!node["archived"].as_bool().unwrap_or(false),fields.to_string(),payload_hash,ledger_hash,updated_at])?;
    Ok(json!({"versionHash":version_hash,"ledgerHash":ledger_hash}))
}

fn organization_node_create(conn: &mut Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        organization_node_create_atomic(conn, input, actor)
    })
}

fn organization_node_create_atomic(
    conn: &Connection,
    input: &Value,
    actor: Option<i64>,
) -> ApiResult {
    let uid = require_user(conn, actor)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let name = s(input, "name").ok_or_else(|| ApiError::bad("name"))?;
    let kind = s(input, "kind").unwrap_or_else(|| "section".into());
    validate_node_text(&name, "Название", 120)?;
    validate_node_text(&kind, "Тип", 40)?;
    let parent = i64v(input, "parentId");
    validate_node_parent(conn, ws, None, parent)?;
    let timestamp = now();
    conn.execute(
        "INSERT INTO organization_nodes(guid,workspace_id,parent_id,kind,name,tab_label,responsible_user_id,display_order,color,icon,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",
        params![uuid::Uuid::new_v4().to_string(),ws,parent,kind,name,s(input,"tabLabel"),i64v(input,"responsibleUserId"),i64v(input,"displayOrder").unwrap_or(0),s(input,"color"),s(input,"icon"),timestamp],
    )?;
    let id = conn.last_insert_rowid();
    let proof = record_organization_node_version(conn, id, uid, "create")?;
    let mut result =
        organization_node_json(conn, id).ok_or_else(|| ApiError::internal("Раздел не создан"))?;
    result["versionHash"] = proof["versionHash"].clone();
    Ok(result)
}

fn organization_node_update(conn: &mut Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        organization_node_update_atomic(conn, input, actor)
    })
}

fn organization_node_update_atomic(
    conn: &Connection,
    input: &Value,
    actor: Option<i64>,
) -> ApiResult {
    let uid = require_user(conn, actor)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let old =
        organization_node_json(conn, id).ok_or_else(|| ApiError::not_found("Раздел не найден"))?;
    let ws = old["workspaceId"].as_i64().unwrap_or(0);
    if let Some(name) = s(input, "name") {
        validate_node_text(&name, "Название", 120)?;
    }
    if let Some(kind) = s(input, "kind") {
        validate_node_text(&kind, "Тип", 40)?;
    }
    let parent = if input.get("parentId").is_some() {
        i64v(input, "parentId")
    } else {
        old["parentId"].as_i64()
    };
    validate_node_parent(conn, ws, Some(id), parent)?;
    conn.execute(
        "UPDATE organization_nodes SET parent_id=?2,name=COALESCE(?3,name),kind=COALESCE(?4,kind),tab_label=CASE WHEN ?5 THEN ?6 ELSE tab_label END,responsible_user_id=CASE WHEN ?7 THEN ?8 ELSE responsible_user_id END,display_order=COALESCE(?9,display_order),color=CASE WHEN ?10 THEN ?11 ELSE color END,icon=CASE WHEN ?12 THEN ?13 ELSE icon END,archived=COALESCE(?14,archived),updated_at=?15 WHERE id=?1",
        params![id,parent,s(input,"name"),s(input,"kind"),input.get("tabLabel").is_some(),s(input,"tabLabel"),input.get("responsibleUserId").is_some(),i64v(input,"responsibleUserId"),i64v(input,"displayOrder"),input.get("color").is_some(),s(input,"color"),input.get("icon").is_some(),s(input,"icon"),input.get("archived").and_then(Value::as_bool).map(i64::from),now()],
    )?;
    let updated =
        organization_node_json(conn, id).ok_or_else(|| ApiError::not_found("Раздел не найден"))?;
    let proof = record_organization_node_version(conn, id, uid, "update")?;
    let mut updated = updated;
    updated["versionHash"] = proof["versionHash"].clone();
    Ok(updated)
}

fn organization_node_remove(conn: &mut Connection, input: &Value, actor: Option<i64>) -> ApiResult {
    atomic(conn, |conn| {
        organization_node_remove_atomic(conn, input, actor)
    })
}

fn organization_node_remove_atomic(
    conn: &Connection,
    input: &Value,
    actor: Option<i64>,
) -> ApiResult {
    let uid = require_user(conn, actor)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    organization_node_json(conn, id).ok_or_else(|| ApiError::not_found("Раздел не найден"))?;
    let occupied: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM organization_nodes WHERE parent_id=?1 AND archived=0) + (SELECT COUNT(*) FROM items WHERE organization_node_id=?1)",
        [id], |r| r.get(0),
    )?;
    if occupied != 0 {
        return Err(ApiError::conflict(
            "Сначала перенесите дочерние разделы и оборудование",
        ));
    }
    conn.execute(
        "UPDATE organization_nodes SET archived=1,updated_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    let proof = record_organization_node_version(conn, id, uid, "archive")?;
    Ok(json!({"ok":true,"archived":true,"id":id,"versionHash":proof["versionHash"]}))
}

fn dict_table(kind: &str) -> Result<&'static str, ApiError> {
    match kind {
        "categories" => Ok("categories"),
        "brands" => Ok("brands"),
        "statuses" => Ok("statuses"),
        _ => Err(ApiError::bad("kind")),
    }
}
fn dict_list(conn: &Connection, input: &Value) -> ApiResult {
    let kind = s(input, "kind").unwrap_or_else(|| "categories".into());
    let table = dict_table(&kind)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let sql = if table == "statuses" {
        format!("SELECT id,name,description,workspace_id,type,slug,color,bg,guid FROM {table} WHERE workspace_id=?1 AND archived=0")
    } else {
        format!("SELECT id,name,description,workspace_id,type,NULL,NULL,NULL,guid FROM {table} WHERE workspace_id=?1 AND archived=0")
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<Value> = stmt
        .query_map(params![ws], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?,
                "description": r.get::<_, Option<String>>(2)?, "workspaceId": r.get::<_, i64>(3)?,
                "type": r.get::<_, String>(4)?, "slug": r.get::<_, Option<String>>(5)?,
                "color": r.get::<_, Option<String>>(6)?, "bg": r.get::<_, Option<String>>(7)?,
                "guid":r.get::<_,Option<String>>(8)?,
            }))
        })?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Value::Array(rows))
}
fn dict_kind(kind: &str) -> Result<&'static str, ApiError> {
    match kind {
        "categories" => Ok("category"),
        "brands" => Ok("brand"),
        "statuses" => Ok("status"),
        _ => Err(ApiError::bad("kind")),
    }
}
fn dict_create(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| dict_create_atomic(conn, input, user_id))
}
fn dict_create_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let kind = s(input, "kind").unwrap_or_else(|| "categories".into());
    let table = dict_table(&kind)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let name = s(input, "name").ok_or_else(|| ApiError::bad("name"))?;
    validate_node_text(&name, "Название", 120)?;
    let guid = Uuid::new_v4().to_string();
    if table == "statuses" {
        conn.execute("INSERT INTO statuses(name,description,workspace_id,type,slug,color,bg,guid) VALUES(?1,?2,?3,'status',?4,?5,?6,?7)",params![name,s(input,"description"),ws,s(input,"slug").unwrap_or_else(||format!("custom-{}",Uuid::new_v4())),s(input,"color").unwrap_or("#5E629B".into()),s(input,"bg").unwrap_or("#EDEDF7".into()),guid])?;
    } else {
        let ty = if table == "brands" {
            "brand"
        } else {
            "category"
        };
        conn.execute(
            &format!(
                "INSERT INTO {table}(name,description,workspace_id,type,guid) VALUES(?1,?2,?3,?4,?5)"
            ),
            params![name,s(input,"description"),ws,ty,guid],
        )?;
    }
    let id = conn.last_insert_rowid();
    let proof = record_config_version(conn, dict_kind(&kind)?, id, uid, "create")?;
    Ok(
        json!({"id":id,"guid":proof["guid"],"versionHash":proof["versionHash"],"name":name,"workspaceId":ws}),
    )
}
fn dict_update(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| dict_update_atomic(conn, input, user_id))
}
fn dict_update_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let kind = s(input, "kind").unwrap_or_else(|| "categories".into());
    let table = dict_table(&kind)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    conn.execute(
        &format!("UPDATE {table} SET name=COALESCE(?2,name),description=CASE WHEN ?3 THEN ?4 ELSE description END WHERE id=?1 AND archived=0"),
        params![id,s(input,"name"),input.get("description").is_some(),s(input,"description")],
    )?;
    let proof = record_config_version(conn, dict_kind(&kind)?, id, uid, "update")?;
    Ok(json!({"id":id,"guid":proof["guid"],"versionHash":proof["versionHash"],"ok":true}))
}
fn dict_remove(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| dict_remove_atomic(conn, input, user_id))
}
fn dict_remove_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let kind = s(input, "kind").unwrap_or_else(|| "categories".into());
    let table = dict_table(&kind)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let column = match table {
        "categories" => "category_id",
        "brands" => "brand_id",
        "statuses" => "status_id",
        _ => unreachable!(),
    };
    let used: i64 = conn.query_row(
        &format!("SELECT count(*) FROM items WHERE {column}=?1 AND archived=0"),
        [id],
        |r| r.get(0),
    )?;
    if used > 0 {
        return Err(ApiError::conflict("Элемент используется в карточках ТМЦ"));
    }
    conn.execute(
        &format!("UPDATE {table} SET archived=1 WHERE id=?1"),
        params![id],
    )?;
    let proof = record_config_version(conn, dict_kind(&kind)?, id, uid, "archive")?;
    Ok(
        json!({"ok":true,"archived":true,"id":id,"guid":proof["guid"],"versionHash":proof["versionHash"]}),
    )
}

fn notify_admins(conn: &Connection, ws: i64, item_id: i64, title: &str, text: &str) {
    let mut stmt = match conn.prepare("SELECT user_id FROM user_workspaces WHERE workspace_id=?1") {
        Ok(s) => s,
        Err(_) => return,
    };
    let ids: Vec<i64> = stmt
        .query_map(params![ws], |r| r.get(0))
        .ok()
        .map(|r| r.filter_map(|x| x.ok()).collect())
        .unwrap_or_default();
    for uid in ids {
        if user_can(conn, uid, "manageUsers") || user_can(conn, uid, "editItems") {
            let _ = conn.execute(
                "INSERT INTO notifications (user_id, item_id, type, title, text, created_at) VALUES (?1,?2,'system',?3,?4,?5)",
                params![uid, item_id, title, text, now()],
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fault_payload_hash(
    fault_guid: &str,
    parent_hash: Option<&str>,
    depth: i64,
    workspace_guid: &str,
    item_guid: &str,
    reporter_guid: &str,
    actor_guid: &str,
    severity: &str,
    description: &str,
    photo_url: Option<&str>,
    status: &str,
    resolution: Option<&str>,
    created_at: &str,
) -> String {
    let payload = json!({
        "domain":"everyday/fault-record/v1","faultGuid":fault_guid,"parentHash":parent_hash,
        "depth":depth,"workspaceGuid":workspace_guid,"itemGuid":item_guid,
        "reporterGuid":reporter_guid,"actorGuid":actor_guid,"severity":severity,
        "description":description,"photoUrl":photo_url,"status":status,
        "resolution":resolution,"createdAt":created_at,
    });
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload).expect("JSON serialization"))
    )
}

fn fault_record_hash(payload_hash: &str, ledger_hash: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("everyday/fault-record-ledger/v1\n{payload_hash}\n{ledger_hash}").as_bytes()
        )
    )
}

fn report_fault(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| report_fault_atomic(conn, input, user_id))
}

fn report_fault_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let item_id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let ws = require_item_access(conn, uid, item_id)?;
    require_can_in_workspace(conn, uid, ws, "reportFaults")?;
    let desc = s(input, "description").ok_or_else(|| ApiError::bad("Опишите неисправность"))?;
    if desc.chars().count() > 8_000 {
        return Err(ApiError::bad("Описание длиннее 8000 символов"));
    }
    let item = jsn::item_json(conn, item_id, false)
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    let severity = s(input, "severity").unwrap_or_else(|| "medium".into());
    if !matches!(severity.as_str(), "low" | "medium" | "high") {
        return Err(ApiError::bad("Некорректная важность"));
    }
    let source_photo = s(input, "photoUrl");
    let photo = source_photo
        .as_deref()
        .map(|value| crate::content::ingest_data_url(conn, value))
        .transpose()
        .map_err(|error| ApiError::bad(format!("Некорректное фото: {error}")))?
        .flatten()
        .or(source_photo);
    let fault_guid = Uuid::new_v4().to_string();
    let created_at = now();
    let workspace_guid =
        ledger::guid(conn, "workspaces", ws).map_err(|e| ApiError::internal(e.to_string()))?;
    let item_guid =
        ledger::guid(conn, "items", item_id).map_err(|e| ApiError::internal(e.to_string()))?;
    let actor_guid =
        ledger::guid(conn, "users", uid).map_err(|e| ApiError::internal(e.to_string()))?;
    let payload_hash = fault_payload_hash(
        &fault_guid,
        None,
        0,
        &workspace_guid,
        &item_guid,
        &actor_guid,
        &actor_guid,
        &severity,
        &desc,
        photo.as_deref(),
        "open",
        None,
        &created_at,
    );
    let event = ledger::append(
        conn,
        ws,
        uid,
        Some(item_id),
        "fault_report",
        Some(&fault_guid),
        Some(&payload_hash),
        None,
        Some(&desc),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    let ledger_hash = event["opId"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?;
    let record_hash = fault_record_hash(&payload_hash, ledger_hash);
    conn.execute(
        "INSERT INTO faults (item_id, workspace_id, author_id, severity, description, photo_url, created_at,guid) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![item_id,ws,uid,severity,desc,photo,created_at,fault_guid],
    )?;
    let fid = conn.last_insert_rowid();
    conn.execute("INSERT INTO fault_records(record_hash,fault_guid,parent_hash,depth,workspace_guid,item_guid,reporter_guid,actor_guid,severity,description,photo_url,status,resolution,payload_hash,ledger_hash,created_at)
        VALUES(?1,?2,NULL,0,?3,?4,?5,?5,?6,?7,?8,'open',NULL,?9,?10,?11)",params![record_hash,fault_guid,workspace_guid,item_guid,actor_guid,severity,desc,photo,payload_hash,ledger_hash,created_at])?;
    // Сообщение о неисправности переводит предмет в «На проверке»: решение о
    // ремонте принимает администратор (ТЗ §4, «Неисправность и ремонт»).
    if let Ok(st) = conn.query_row(
        "SELECT id FROM statuses WHERE workspace_id=?1 AND slug='needs-check'",
        params![ws],
        |r| r.get::<_, i64>(0),
    ) {
        conn.execute(
            "UPDATE items SET status_id=?1 WHERE id=?2",
            params![st, item_id],
        )?;
    }
    let title = item["title"].as_str().unwrap_or("");
    notify_admins(
        conn,
        ws,
        item_id,
        "Неисправность",
        &format!("{title}: {desc}"),
    );
    Ok(
        json!({"id":fid,"guid":fault_guid,"recordHash":record_hash,"ledgerHash":ledger_hash,"itemId":item_id,"status":"open"}),
    )
}

fn list_faults(conn: &Connection, input: &Value) -> ApiResult {
    let mut sql = String::from("SELECT id,item_id,workspace_id,author_id,severity,description,photo_url,status,resolution,resolver_id,created_at,resolved_at,guid FROM faults WHERE 1=1");
    if let Some(id) = i64v(input, "itemId") {
        sql.push_str(&format!(" AND item_id={id}"));
    } else {
        let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
        sql.push_str(&format!(" AND workspace_id={ws}"));
    }
    sql.push_str(" ORDER BY id DESC LIMIT 200");
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<Value> = stmt.query_map([], |r| {
        let author: i64 = r.get(3)?;
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "itemId": r.get::<_, i64>(1)?, "workspaceId": r.get::<_, i64>(2)?,
            "authorId": author, "severity": r.get::<_, String>(4)?, "description": r.get::<_, String>(5)?,
            "photoUrl": r.get::<_, Option<String>>(6)?, "status": r.get::<_, String>(7)?,
            "resolution": r.get::<_, Option<String>>(8)?, "resolverId": r.get::<_, Option<i64>>(9)?,
            "createdAt": r.get::<_, String>(10)?, "resolvedAt": r.get::<_, Option<String>>(11)?,
            "guid":r.get::<_,Option<String>>(12)?,
            "author": jsn::user_public(conn, author)
        }))
    })?.filter_map(|x| x.ok()).collect();
    Ok(Value::Array(rows))
}

fn resolve_fault(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| resolve_fault_atomic(conn, input, user_id))
}

fn resolve_fault_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let (item_id,ws,stored_guid,severity,description,photo,reporter_id):(i64,i64,Option<String>,String,String,Option<String>,i64)=conn
        .query_row(
            "SELECT item_id,workspace_id,guid,severity,description,photo_url,author_id FROM faults WHERE id=?1",
            params![id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Неисправность не найдена"))?;
    let fault_guid = stored_guid
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    conn.execute(
        "UPDATE faults SET guid=?1 WHERE id=?2 AND (guid IS NULL OR guid='')",
        params![fault_guid, id],
    )?;
    require_member(conn, uid, ws)?;
    require_can_in_workspace(conn, uid, ws, "editItems")?;
    let status = s(input, "status").unwrap_or_else(|| "resolved".into());
    if !matches!(status.as_str(), "open" | "repair" | "resolved") {
        return Err(ApiError::bad("Некорректный статус неисправности"));
    }
    let resolution = s(input, "comment");
    if resolution
        .as_deref()
        .is_some_and(|v| v.chars().count() > 8_000)
    {
        return Err(ApiError::bad("Решение длиннее 8000 символов"));
    }
    let parent:Option<(String,i64)>=conn.query_row("SELECT record_hash,depth FROM fault_records WHERE fault_guid=?1 ORDER BY depth DESC,record_hash DESC LIMIT 1",[&fault_guid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let (parent_hash, depth, event_type) = match parent {
        Some((hash, depth)) => (Some(hash), depth + 1, "fault_update"),
        None => (None, 0, "fault_adopt"),
    };
    let created_at = now();
    let workspace_guid =
        ledger::guid(conn, "workspaces", ws).map_err(|e| ApiError::internal(e.to_string()))?;
    let item_guid =
        ledger::guid(conn, "items", item_id).map_err(|e| ApiError::internal(e.to_string()))?;
    let reporter_guid =
        ledger::guid(conn, "users", reporter_id).map_err(|e| ApiError::internal(e.to_string()))?;
    let actor_guid =
        ledger::guid(conn, "users", uid).map_err(|e| ApiError::internal(e.to_string()))?;
    let payload_hash = fault_payload_hash(
        &fault_guid,
        parent_hash.as_deref(),
        depth,
        &workspace_guid,
        &item_guid,
        &reporter_guid,
        &actor_guid,
        &severity,
        &description,
        photo.as_deref(),
        &status,
        resolution.as_deref(),
        &created_at,
    );
    let event = ledger::append(
        conn,
        ws,
        uid,
        Some(item_id),
        event_type,
        Some(&fault_guid),
        Some(&payload_hash),
        None,
        resolution.as_deref().or(Some(&status)),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    let ledger_hash = event["opId"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?;
    let record_hash = fault_record_hash(&payload_hash, ledger_hash);
    conn.execute("INSERT INTO fault_records(record_hash,fault_guid,parent_hash,depth,workspace_guid,item_guid,reporter_guid,actor_guid,severity,description,photo_url,status,resolution,payload_hash,ledger_hash,created_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![record_hash,fault_guid,parent_hash,depth,workspace_guid,item_guid,reporter_guid,actor_guid,severity,description,photo,status,resolution,payload_hash,ledger_hash,created_at])?;
    conn.execute(
        "UPDATE faults SET status=?1, resolution=?2, resolver_id=?3, resolved_at=?4 WHERE id=?5",
        params![status, resolution, uid, created_at, id],
    )?;
    let slug = if status == "repair" || status == "open" {
        "in-repair"
    } else {
        "in-stock"
    };
    if let Ok(st) = conn.query_row(
        "SELECT id FROM statuses WHERE workspace_id=?1 AND slug=?2",
        params![ws, slug],
        |r| r.get::<_, i64>(0),
    ) {
        conn.execute(
            "UPDATE items SET status_id=?1 WHERE id=?2",
            params![st, item_id],
        )?;
    }
    Ok(
        json!({"ok":true,"id":id,"guid":fault_guid,"recordHash":record_hash,"ledgerHash":ledger_hash,"status":status}),
    )
}

fn request_change(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| request_change_atomic(conn, input, user_id))
}

fn request_change_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let item_id = i64v(input, "itemId").ok_or_else(|| ApiError::bad("itemId"))?;
    let ws = require_item_access(conn, uid, item_id)?;
    require_can_in_workspace(conn, uid, ws, "requestChanges")?;
    let payload = input
        .get("payload")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| ApiError::bad("payload должен быть объектом"))?;
    if payload.as_object().is_none_or(|o| o.is_empty()) {
        return Err(ApiError::bad("Заявка не содержит изменений"));
    }
    if payload.to_string().len() > 64 * 1024 {
        return Err(ApiError::bad("Заявка превышает 64 КиБ"));
    }
    if payload.as_object().is_some_and(|o| {
        o.keys().any(|key| {
            !CHANGEABLE_FIELDS
                .iter()
                .any(|(allowed, _, _)| allowed == key)
        })
    }) {
        return Err(ApiError::bad("Заявка содержит запрещённое поле"));
    }
    let comment = s(input, "comment");
    if comment
        .as_deref()
        .is_some_and(|v| v.chars().count() > 4_000)
    {
        return Err(ApiError::bad("Комментарий длиннее 4000 символов"));
    }
    let item = jsn::item_json(conn, item_id, false)
        .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
    let patch = portable_change_fields(conn, ws, &payload)?;
    let before_source = Value::Object(
        payload
            .as_object()
            .unwrap()
            .keys()
            .map(|key| (key.clone(), item.get(key).cloned().unwrap_or(Value::Null)))
            .collect(),
    );
    let before = portable_change_fields(conn, ws, &before_source)?;
    let guid = Uuid::new_v4().to_string();
    let created_at = now();
    let workspace_guid =
        ledger::guid(conn, "workspaces", ws).map_err(|e| ApiError::internal(e.to_string()))?;
    let item_guid =
        ledger::guid(conn, "items", item_id).map_err(|e| ApiError::internal(e.to_string()))?;
    let requester_guid =
        ledger::guid(conn, "users", uid).map_err(|e| ApiError::internal(e.to_string()))?;
    let patch_json =
        serde_json::to_string(&patch).map_err(|e| ApiError::internal(e.to_string()))?;
    let before_json =
        serde_json::to_string(&before).map_err(|e| ApiError::internal(e.to_string()))?;
    let payload_hash = change_record_payload_hash(
        &guid,
        None,
        0,
        &workspace_guid,
        &item_guid,
        &requester_guid,
        &requester_guid,
        &patch_json,
        &before_json,
        comment.as_deref(),
        "pending",
        None,
        &created_at,
    );
    let event = ledger::append(
        conn,
        ws,
        uid,
        Some(item_id),
        "change_request",
        Some(&guid),
        Some(&payload_hash),
        None,
        comment.as_deref().or(Some("Заявка на изменение карточки")),
    )
    .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
    let ledger_hash = event["opId"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?;
    let record_hash = change_record_hash(&payload_hash, ledger_hash);
    conn.execute(
        "INSERT INTO change_requests(item_id,workspace_id,author_id,payload,comment,created_at,guid) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![item_id,ws,uid,payload.to_string(),comment,created_at,guid],
    )?;
    let rid = conn.last_insert_rowid();
    conn.execute("INSERT INTO change_request_records(record_hash,request_guid,parent_hash,depth,workspace_guid,item_guid,requester_guid,actor_guid,patch_json,before_json,comment,status,reason,payload_hash,ledger_hash,created_at)
        VALUES(?1,?2,NULL,0,?3,?4,?5,?5,?6,?7,?8,'pending',NULL,?9,?10,?11)",params![record_hash,guid,workspace_guid,item_guid,requester_guid,patch_json,before_json,comment,payload_hash,ledger_hash,created_at])?;
    notify_admins(
        conn,
        ws,
        item_id,
        "Заявка на правку",
        s(input, "comment")
            .as_deref()
            .unwrap_or("Изменение карточки"),
    );
    Ok(
        json!({"id":rid,"guid":guid,"recordHash":record_hash,"ledgerHash":ledger_hash,"status":"pending"}),
    )
}

/// Человекочитаемое имя записи справочника. Для пользователей это ФИО,
/// для остальных таблиц — колонка name.
fn lookup_name(conn: &Connection, table: &str, id: Option<i64>) -> Option<String> {
    let id = id?;
    let column = if table == "users" {
        "full_name"
    } else {
        "name"
    };
    let sql = format!("SELECT {column} FROM {table} WHERE id=?1");
    conn.query_row(&sql, params![id], |r| r.get::<_, String>(0))
        .optional()
        .ok()
        .flatten()
}

/// Поля карточки, которые может менять заявка: ключ, подпись и справочник,
/// через который идентификатор разворачивается в название.
const CHANGEABLE_FIELDS: [(&str, &str, Option<&str>); 13] = [
    ("title", "Наименование", None),
    ("categoryId", "Категория", Some("categories")),
    ("brandId", "Бренд", Some("brands")),
    ("statusId", "Статус", Some("statuses")),
    ("responsibleUserId", "Ответственный", Some("users")),
    ("buildingSiteId", "Объект", Some("building_sites")),
    ("storageId", "Место хранения", Some("storages")),
    ("serialNumber", "Серийный номер", None),
    ("cost", "Стоимость", None),
    ("comment", "Комментарий", None),
    ("qrCode", "QR-код", None),
    ("calibratedUntil", "Поверка до", None),
    ("minQuantity", "Мин. остаток", None),
];

fn portable_change_fields(conn: &Connection, ws: i64, source: &Value) -> Result<Value, ApiError> {
    let mut result = serde_json::Map::new();
    for (key, _, dictionary) in CHANGEABLE_FIELDS {
        let Some(value) = source.get(key) else {
            continue;
        };
        if value.is_null() {
            result.insert(key.into(), Value::Null);
            continue;
        }
        let portable = match dictionary {
            None => value.clone(),
            Some("users") => {
                let id = value
                    .as_i64()
                    .ok_or_else(|| ApiError::bad(format!("{key}: ожидается ID")))?;
                let guid:String=conn.query_row("SELECT u.guid FROM users u JOIN user_workspaces uw ON uw.user_id=u.id WHERE u.id=?1 AND uw.workspace_id=?2",params![id,ws],|r|r.get(0)).map_err(|_|ApiError::bad(format!("{key}: пользователь не найден")))?;
                json!({"$ref":"user","guid":guid})
            }
            Some("statuses") => {
                let id = value
                    .as_i64()
                    .ok_or_else(|| ApiError::bad(format!("{key}: ожидается ID")))?;
                let (slug, name): (String, String) = conn
                    .query_row(
                        "SELECT slug,name FROM statuses WHERE id=?1 AND workspace_id=?2",
                        params![id, ws],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(|_| ApiError::bad(format!("{key}: статус не найден")))?;
                json!({"$ref":"status","slug":slug,"name":name})
            }
            Some(table) => {
                let id = value
                    .as_i64()
                    .ok_or_else(|| ApiError::bad(format!("{key}: ожидается ID")))?;
                let sql = format!("SELECT name FROM {table} WHERE id=?1 AND workspace_id=?2");
                let name: String = conn
                    .query_row(&sql, params![id, ws], |r| r.get(0))
                    .map_err(|_| ApiError::bad(format!("{key}: справочник не найден")))?;
                json!({"$ref":table,"name":name})
            }
        };
        result.insert(key.into(), portable);
    }
    Ok(Value::Object(result))
}

#[allow(clippy::too_many_arguments)]
fn change_record_payload_hash(
    request_guid: &str,
    parent_hash: Option<&str>,
    depth: i64,
    workspace_guid: &str,
    item_guid: &str,
    requester_guid: &str,
    actor_guid: &str,
    patch_json: &str,
    before_json: &str,
    comment: Option<&str>,
    status: &str,
    reason: Option<&str>,
    created_at: &str,
) -> String {
    let payload = json!({"domain":"everyday/change-request-record/v1","requestGuid":request_guid,"parentHash":parent_hash,"depth":depth,
        "workspaceGuid":workspace_guid,"itemGuid":item_guid,"requesterGuid":requester_guid,"actorGuid":actor_guid,
        "patch":serde_json::from_str::<Value>(patch_json).unwrap_or(Value::Null),"before":serde_json::from_str::<Value>(before_json).unwrap_or(Value::Null),
        "comment":comment,"status":status,"reason":reason,"createdAt":created_at});
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload).expect("JSON serialization"))
    )
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

/// Приводит значение поля к строке для показа администратору.
fn display_value(conn: &Connection, raw: &Value, dictionary: Option<&str>) -> Option<String> {
    if raw.is_null() {
        return None;
    }
    if let Some(table) = dictionary {
        let id = raw.as_i64().or_else(|| raw.as_f64().map(|v| v as i64));
        return lookup_name(conn, table, id);
    }
    match raw {
        Value::String(v) if v.is_empty() => None,
        Value::String(v) => Some(v.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "да".into() } else { "нет".into() }),
        _ => Some(raw.to_string()),
    }
}

/// Сравнение «было / предлагается» (ТЗ §4): администратор должен видеть
/// разницу, а не сырой JSON заявки.
fn describe_change(conn: &Connection, item_id: i64, payload: &Value) -> Value {
    let Some(before) = jsn::item_json(conn, item_id, false) else {
        return Value::Array(vec![]);
    };
    let mut rows = Vec::new();
    for (key, label, dictionary) in CHANGEABLE_FIELDS {
        let Some(proposed) = payload.get(key) else {
            continue;
        };
        let after = display_value(conn, proposed, dictionary);
        let before_raw = before.get(key).cloned().unwrap_or(Value::Null);
        let before_text = display_value(conn, &before_raw, dictionary);
        if before_text == after {
            continue; // поле в заявке есть, но значение то же — не шумим
        }
        rows.push(json!({
            "field": key,
            "label": label,
            "before": before_text,
            "after": after,
        }));
    }
    Value::Array(rows)
}

fn list_changes(conn: &Connection, input: &Value) -> ApiResult {
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    let mut stmt = conn.prepare("SELECT id,item_id,workspace_id,author_id,payload,comment,status,reason,decided_by,created_at,decided_at,guid FROM change_requests WHERE workspace_id=?1 ORDER BY id DESC LIMIT 200")?;
    let rows: Vec<Value> = stmt.query_map(params![ws], |r| {
        let author: i64 = r.get(3)?;
        let payload: String = r.get(4)?;
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "itemId": r.get::<_, i64>(1)?, "workspaceId": r.get::<_, i64>(2)?,
            "authorId": author, "payload": serde_json::from_str::<Value>(&payload).unwrap_or(json!({})),
            "comment": r.get::<_, Option<String>>(5)?, "status": r.get::<_, String>(6)?,
            "reason": r.get::<_, Option<String>>(7)?, "decidedBy": r.get::<_, Option<i64>>(8)?,
            "createdAt": r.get::<_, String>(9)?, "decidedAt": r.get::<_, Option<String>>(10)?,
            "guid":r.get::<_,Option<String>>(11)?,
            "author": jsn::user_public(conn, author),
            "item": jsn::item_json(conn, r.get(1)?, false),
            "changes": describe_change(conn, r.get(1)?, &serde_json::from_str::<Value>(&payload).unwrap_or(json!({})))
        }))
    })?.filter_map(|x| x.ok()).collect();
    Ok(Value::Array(rows))
}

fn decide_change(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    atomic(conn, |conn| decide_change_atomic(conn, input, user_id))
}

fn decide_change_atomic(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let id = i64v(input, "id").ok_or_else(|| ApiError::bad("id"))?;
    let (item_id,ws,payload,request_comment,requester_id,stored_guid):(i64,i64,String,Option<String>,i64,Option<String>)=conn
        .query_row(
            "SELECT item_id,workspace_id,payload,comment,author_id,guid FROM change_requests WHERE id=?1",
            params![id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::not_found("Заявка не найдена"))?;
    require_member(conn, uid, ws)?;
    require_can_in_workspace(conn, uid, ws, "editItems")?;
    let already: String = conn.query_row(
        "SELECT status FROM change_requests WHERE id=?1",
        params![id],
        |r| r.get(0),
    )?;
    if already != "pending" {
        return Err(ApiError::conflict("Решение по заявке уже принято"));
    }
    let accept = b(input, "accept").unwrap_or(false);
    let status = if accept { "accepted" } else { "rejected" };
    let guid = stored_guid
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    conn.execute(
        "UPDATE change_requests SET guid=?1 WHERE id=?2 AND (guid IS NULL OR guid='')",
        params![guid, id],
    )?;
    let existing:Option<(String,i64,String,String,String,Option<String>)>=conn.query_row("SELECT record_hash,depth,patch_json,before_json,requester_guid,comment FROM change_request_records WHERE request_guid=?1 ORDER BY depth DESC,record_hash DESC LIMIT 1",[&guid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let raw_payload = serde_json::from_str::<Value>(&payload)
        .map_err(|_| ApiError::bad("Заявка содержит некорректные данные"))?;
    let (parent_hash, depth, patch_json, before_json, requester_guid, root_comment, event_type) =
        if let Some((parent, depth, patch, before, requester, comment)) = existing {
            (
                Some(parent),
                depth + 1,
                patch,
                before,
                requester,
                comment,
                "change_decision",
            )
        } else {
            let item = jsn::item_json(conn, item_id, false)
                .ok_or_else(|| ApiError::not_found("Инструмент не найден"))?;
            let before_source = Value::Object(
                raw_payload
                    .as_object()
                    .ok_or_else(|| ApiError::bad("payload должен быть объектом"))?
                    .keys()
                    .map(|key| (key.clone(), item.get(key).cloned().unwrap_or(Value::Null)))
                    .collect(),
            );
            let patch = portable_change_fields(conn, ws, &raw_payload)?;
            let before = portable_change_fields(conn, ws, &before_source)?;
            let requester = ledger::guid(conn, "users", requester_id)
                .map_err(|e| ApiError::internal(e.to_string()))?;
            (
                None,
                0,
                patch.to_string(),
                before.to_string(),
                requester,
                request_comment.clone(),
                "change_adopt",
            )
        };
    let created_at = now();
    let reason = s(input, "reason");
    if reason.as_deref().is_some_and(|v| v.chars().count() > 4_000) {
        return Err(ApiError::bad("Причина длиннее 4000 символов"));
    }
    let workspace_guid =
        ledger::guid(conn, "workspaces", ws).map_err(|e| ApiError::internal(e.to_string()))?;
    let item_guid =
        ledger::guid(conn, "items", item_id).map_err(|e| ApiError::internal(e.to_string()))?;
    let actor_guid =
        ledger::guid(conn, "users", uid).map_err(|e| ApiError::internal(e.to_string()))?;
    let payload_hash = change_record_payload_hash(
        &guid,
        parent_hash.as_deref(),
        depth,
        &workspace_guid,
        &item_guid,
        &requester_guid,
        &actor_guid,
        &patch_json,
        &before_json,
        root_comment.as_deref(),
        status,
        reason.as_deref(),
        &created_at,
    );
    let ledger_hash = if accept {
        // Правку применяем ДО отметки «принято»: если она не проходит проверки
        // (например, смена статуса без причины), заявка остаётся в работе,
        // а не «принятой», но не применённой.
        let mut patch = raw_payload;
        if let Value::Object(ref mut o) = patch {
            o.insert("id".into(), json!(item_id));
            if !o.contains_key("reason") {
                let reason = reason
                    .clone()
                    .or(request_comment)
                    .unwrap_or_else(|| "Принята заявка на правку".into());
                o.insert("reason".into(), json!(reason));
            }
        }
        let updated = items_update_atomic_bound(
            conn,
            &patch,
            Some(uid),
            Some((event_type, &guid, &payload_hash)),
        )?;
        updated["ledgerHash"]
            .as_str()
            .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?
            .to_string()
    } else {
        let event = ledger::append(
            conn,
            ws,
            uid,
            Some(item_id),
            event_type,
            Some(&guid),
            Some(&payload_hash),
            None,
            reason.as_deref().or(Some("Заявка отклонена")),
        )
        .map_err(|e| ApiError::internal(format!("Ошибка журнала: {e}")))?;
        event["opId"]
            .as_str()
            .ok_or_else(|| ApiError::internal("Ledger не вернул hash"))?
            .to_string()
    };
    let record_hash = change_record_hash(&payload_hash, &ledger_hash);
    conn.execute("INSERT INTO change_request_records(record_hash,request_guid,parent_hash,depth,workspace_guid,item_guid,requester_guid,actor_guid,patch_json,before_json,comment,status,reason,payload_hash,ledger_hash,created_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![record_hash,guid,parent_hash,depth,workspace_guid,item_guid,requester_guid,actor_guid,patch_json,before_json,root_comment,status,reason,payload_hash,ledger_hash,created_at])?;
    conn.execute(
        "UPDATE change_requests SET status=?1, reason=?2, decided_by=?3, decided_at=?4 WHERE id=?5 AND status='pending'",
        params![status,reason,uid,created_at,id],
    )?;
    Ok(
        json!({"ok":true,"id":id,"guid":guid,"recordHash":record_hash,"ledgerHash":ledger_hash,"itemId":item_id,"status":status}),
    )
}

fn chat_list(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    require_member(conn, uid, ws)?;
    let mut stmt = conn.prepare("SELECT id,guid,workspace_id,user_id,text,created_at,ledger_hash FROM chat_messages WHERE workspace_id=?1 ORDER BY created_at DESC,guid DESC LIMIT 200")?;
    let mut rows: Vec<Value> = stmt
        .query_map(params![ws], |r| {
            let author: i64 = r.get(3)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "guid": r.get::<_, String>(1)?,
                "workspaceId": r.get::<_, i64>(2)?, "userId": author,
                "text": r.get::<_, String>(4)?, "createdAt": r.get::<_, String>(5)?,
                "ledgerHash": r.get::<_, Option<String>>(6)?,
                "ledgerVerified": r.get::<_, Option<String>>(6)?.is_some(),
                "user": jsn::user_public(conn, author)
            }))
        })?
        .filter_map(|x| x.ok())
        .collect();
    rows.reverse();
    Ok(Value::Array(rows))
}

fn chat_send(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    let text = s(input, "text")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad("Пустое сообщение"))?;
    if text.chars().count() > 4000 {
        return Err(ApiError::bad("Сообщение длиннее 4000 символов"));
    }
    if text.contains('\0') {
        return Err(ApiError::bad("Сообщение содержит недопустимый символ"));
    }
    let ws = i64v(input, "workspaceId").unwrap_or_else(|| ws_fallback(conn));
    require_member(conn, uid, ws)?;
    let result = atomic(conn, |conn| {
        let minute_ago = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        let recent: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chat_messages WHERE workspace_id=?1 AND user_id=?2 AND created_at>=?3",
            params![ws, uid, minute_ago],
            |row| row.get(0),
        )?;
        if recent >= 20 {
            return Err(ApiError::new(
                "TOO_MANY_REQUESTS",
                429,
                "Слишком много сообщений: подождите минуту",
            ));
        }
        let duplicate_since = (chrono::Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
        let duplicate: bool = conn
            .query_row(
                "SELECT 1 FROM chat_messages WHERE workspace_id=?1 AND user_id=?2 AND text=?3 AND created_at>=?4 LIMIT 1",
                params![ws, uid, text, duplicate_since],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if duplicate {
            return Err(ApiError::new(
                "CONFLICT",
                409,
                "Такое сообщение уже отправлено",
            ));
        }
        let guid = uuid::Uuid::new_v4().to_string();
        let event = ledger::append(
            conn,
            ws,
            uid,
            None,
            "chat_message",
            Some(&guid),
            None,
            None,
            Some(&text),
        )
        .map_err(|error| ApiError::internal(error.to_string()))?;
        let hash = event.get("opId").and_then(Value::as_str).unwrap_or("");
        let created_at = event.get("createdAt").and_then(Value::as_str).unwrap_or("");
        conn.execute(
            "INSERT INTO chat_messages (guid,workspace_id,user_id,text,ledger_hash,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![guid, ws, uid, text, hash, created_at],
        )?;
        Ok(json!({
            "id": conn.last_insert_rowid(), "guid": guid,
            "workspaceId": ws, "userId": uid, "text": text,
            "createdAt": created_at, "ledgerHash": hash, "ledgerVerified": true,
            "user": jsn::user_public(conn, uid)
        }))
    });
    if result.is_ok() {
        crate::sync::request_sync_now();
    }
    result
}

fn backup_export(conn: &Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "manageWorkspaces")?;
    let password = s(input, "password").ok_or_else(|| ApiError::bad("Пароль архива обязателен"))?;
    let mut journal = crate::sync::export_journal(conn);
    journal["blobData"] = crate::content::backup_data(conn);
    ledger::sign_journal(conn, &mut journal)
        .map_err(|e| ApiError::internal(format!("Не удалось подписать архив: {e}")))?;
    crate::sync::encrypt_backup(&password, &journal.to_string())
        .map_err(|e| ApiError::bad(e.to_string()))
}

fn backup_import(conn: &mut Connection, input: &Value, user_id: Option<i64>) -> ApiResult {
    let uid = require_user(conn, user_id)?;
    require_can(conn, uid, "manageWorkspaces")?;
    let password = s(input, "password").ok_or_else(|| ApiError::bad("Пароль архива обязателен"))?;
    let blob = input
        .get("blob")
        .cloned()
        .ok_or_else(|| ApiError::bad("Нет архива"))?;
    let plain =
        crate::sync::decrypt_backup(&password, &blob).map_err(|e| ApiError::bad(e.to_string()))?;
    let journal: Value = serde_json::from_str(&plain).map_err(|e| ApiError::bad(e.to_string()))?;
    conn.execute_batch("SAVEPOINT complete_backup_restore")?;
    let result = crate::sync::apply_remote_journal(conn, &journal, "");
    if result.get("ok").and_then(Value::as_bool) == Some(false) {
        let _ = conn
            .execute_batch("ROLLBACK TO complete_backup_restore; RELEASE complete_backup_restore");
        return Err(ApiError::bad(
            result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Архив не прошёл криптографическую проверку"),
        ));
    }
    let restored_blobs = match crate::content::restore_backup_data(
        conn,
        journal.get("blobData").unwrap_or(&Value::Null),
    ) {
        Ok(count) => count,
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO complete_backup_restore; RELEASE complete_backup_restore",
            );
            return Err(ApiError::bad(format!(
                "Вложение архива повреждено: {error}"
            )));
        }
    };
    conn.execute_batch("RELEASE complete_backup_restore")?;
    let mut result = result;
    result["blobs"] = json!(restored_blobs);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_db() -> (Connection, PathBuf, [i64; 3], i64) {
        let path = std::env::temp_dir().join(format!("meshkeeper-api-{}.db", Uuid::new_v4()));
        let conn = db::open(&path).expect("test database");
        conn.execute(
            "INSERT INTO workspaces (name, timezone, internal_id_prefix, created_at) VALUES ('Test','UTC','T-',?1)",
            params![now()],
        )
        .unwrap();
        let ws = conn.last_insert_rowid();
        let mut users = [0; 3];
        for (index, slot) in users.iter_mut().enumerate() {
            let rights = if index == 0 {
                db::owner_rights()
            } else {
                db::default_rights()
            };
            conn.execute(
                "INSERT INTO users (full_name, phone, status, role_rights, created_at)
                 VALUES (?1,?2,'active',?3,?4)",
                params![
                    format!("User {index}"),
                    format!("+7000000000{index}"),
                    rights.to_string(),
                    now()
                ],
            )
            .unwrap();
            *slot = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO user_workspaces (user_id, workspace_id) VALUES (?1,?2)",
                params![*slot, ws],
            )
            .unwrap();
        }
        (conn, path, users, ws)
    }

    #[test]
    fn ble_transport_diagnostics_require_auth_deduplicate_and_resolve() {
        let (mut conn, path, users, _) = test_db();
        let report = json!({
            "transport":"ble",
            "error":true,
            "message":"Соединение потеряно\nповтор"
        });
        assert_eq!(
            dispatch(&mut conn, "sync.reportTransportStatus", &report, None)
                .unwrap_err()
                .http,
            401
        );
        dispatch(
            &mut conn,
            "sync.reportTransportStatus",
            &report,
            Some(users[0]),
        )
        .unwrap();
        dispatch(
            &mut conn,
            "sync.reportTransportStatus",
            &report,
            Some(users[0]),
        )
        .unwrap();
        let active = crate::diagnostics::list(&conn);
        assert_eq!(active["unresolved"], 1);
        assert_eq!(active["events"][0]["component"], "transport");
        assert_eq!(active["events"][0]["code"], "ble_transport");
        assert_eq!(active["events"][0]["count"], 2);
        assert_eq!(active["events"][0]["message"], "Соединение потеряноповтор");

        dispatch(
            &mut conn,
            "sync.reportTransportStatus",
            &json!({"transport":"ble","error":false,"message":"Связь восстановлена"}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(crate::diagnostics::list(&conn)["unresolved"], 0);
        assert!(dispatch(
            &mut conn,
            "sync.reportTransportStatus",
            &json!({"transport":"unknown","error":true}),
            Some(users[0]),
        )
        .is_err());
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    fn insert_item(
        conn: &Connection,
        ws: i64,
        responsible: Option<i64>,
        quantitative: bool,
        quantity: Option<f64>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO items (internal_id, title, responsible_user_id, workspace_id, quantitative, quantity, comment, qr_code, created_at)
             VALUES (?1,'Test item',?2,?3,?4,?5,'keep','QR-KEEP',?6)",
            params![
                format!("T-{}", Uuid::new_v4()),
                responsible,
                ws,
                quantitative as i64,
                quantity,
                now()
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// Делает запись в журнал невозможной, чтобы проверить откат мутации.
    fn break_ledger(conn: &Connection) {
        conn.execute_batch(
            "CREATE TRIGGER block_ledger BEFORE INSERT ON history_entries
             BEGIN SELECT RAISE(ABORT, 'ledger unavailable'); END;",
        )
        .unwrap();
    }

    fn cleanup(conn: Connection, path: PathBuf) {
        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn item_patch_preserves_absent_nullable_fields_and_clears_explicit_null() {
        let (mut conn, path, users, ws) = test_db();
        let item_id = insert_item(&conn, ws, Some(users[0]), false, None);

        items_update(
            &mut conn,
            &json!({"id": item_id, "title": "Renamed"}),
            Some(users[0]),
        )
        .unwrap();
        let preserved: (Option<i64>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT responsible_user_id, comment, qr_code FROM items WHERE id=?1",
                params![item_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            preserved,
            (Some(users[0]), Some("keep".into()), Some("QR-KEEP".into()))
        );

        items_update(
            &mut conn,
            &json!({"id": item_id, "responsibleUserId": null, "comment": null}),
            Some(users[0]),
        )
        .unwrap();
        let cleared: (Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT responsible_user_id, comment FROM items WHERE id=?1",
                params![item_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(cleared, (None, None));
        cleanup(conn, path);
    }

    #[test]
    fn quantity_writeoff_rejects_underflow_without_mutation_or_ledger_entry() {
        let (mut conn, path, users, ws) = test_db();
        let item_id = insert_item(&conn, ws, None, true, Some(5.0));
        let error = history_write_off(
            &mut conn,
            &json!({"itemId": item_id, "quantity": 6.0, "comment": "damage"}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(error.http, 400);
        let quantity: f64 = conn
            .query_row(
                "SELECT quantity FROM items WHERE id=?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM history_entries WHERE item_id=?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(quantity, 5.0);
        assert_eq!(events, 0);
        cleanup(conn, path);
    }

    #[test]
    fn item_update_rolls_back_when_ledger_append_fails() {
        let (mut conn, path, users, ws) = test_db();
        let item_id = insert_item(&conn, ws, Some(users[0]), false, None);
        break_ledger(&conn);

        let error = items_update(
            &mut conn,
            &json!({"id": item_id, "title": "Must roll back"}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(error.http, 500);
        let title: String = conn
            .query_row(
                "SELECT title FROM items WHERE id=?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "Test item");
        cleanup(conn, path);
    }

    #[test]
    fn transfer_can_only_be_accepted_by_recipient() {
        let (mut conn, path, users, ws) = test_db();
        let item_id = insert_item(&conn, ws, Some(users[0]), false, None);
        conn.execute(
            "INSERT INTO transfers (code, item_id, from_user_id, to_user_id, workspace_id, status, no_confirmation, created_at)
             VALUES ('P-1',?1,?2,?3,?4,'pending',0,?5)",
            params![item_id, users[0], users[1], ws, now()],
        )
        .unwrap();
        let transfer_id = conn.last_insert_rowid();

        let error = transfers_accept(&mut conn, &json!({"id": transfer_id}), Some(users[0]), true)
            .unwrap_err();
        assert_eq!(error.http, 403);
        let status: String = conn
            .query_row(
                "SELECT status FROM transfers WHERE id=?1",
                params![transfer_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending");

        transfers_accept(&mut conn, &json!({"id": transfer_id}), Some(users[1]), true).unwrap();
        let responsible: Option<i64> = conn
            .query_row(
                "SELECT responsible_user_id FROM items WHERE id=?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(responsible, Some(users[1]));
        cleanup(conn, path);
    }

    #[test]
    fn inventory_completion_is_atomic_and_idempotent() {
        let (mut conn, path, users, ws) = test_db();
        conn.execute(
            "INSERT INTO inventory_sessions (number, workspace_id, status, started_by, created_at)
             VALUES ('INV-1',?1,'in_progress',?2,?3)",
            params![ws, users[0], now()],
        )
        .unwrap();
        let session_id = conn.last_insert_rowid();

        inv_complete(&mut conn, &json!({"sessionId": session_id}), Some(users[0])).unwrap();
        let error =
            inv_complete(&mut conn, &json!({"sessionId": session_id}), Some(users[0])).unwrap_err();
        assert_eq!(error.http, 409);
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM history_entries WHERE workspace_id=?1 AND type IN ('inventory_complete','inventory_adopt_complete')",
                params![ws],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
        cleanup(conn, path);
    }

    #[test]
    fn id_only_item_route_rejects_cross_workspace_access() {
        let (mut conn, path, users, _ws) = test_db();
        conn.execute(
            "INSERT INTO workspaces (name, timezone, internal_id_prefix, created_at) VALUES ('Other','UTC','O-',?1)",
            params![now()],
        ).unwrap();
        let other_ws = conn.last_insert_rowid();
        let item = insert_item(&conn, other_ws, None, false, None);
        let error = dispatch(
            &mut conn,
            "items.byId",
            &json!({"id": item}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(error.http, 403);
        cleanup(conn, path);
    }

    #[test]
    fn qr_lookup_selects_only_items_from_active_memberships() {
        let (mut conn, path, users, ws) = test_db();
        conn.execute(
            "INSERT INTO workspaces (name, timezone, internal_id_prefix, created_at) VALUES ('Foreign','UTC','F-',?1)",
            params![now()],
        )
        .unwrap();
        let foreign_ws = conn.last_insert_rowid();
        let foreign = insert_item(&conn, foreign_ws, None, false, None);
        let own = insert_item(&conn, ws, None, false, None);
        assert!(foreign < own, "регрессия требует более ранний чужой дубль");

        let found = dispatch(
            &mut conn,
            "items.byCode",
            &json!({"code":"qr-keep"}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(found["id"], own);
        assert_eq!(found["workspaceId"], ws);
        let guid = Uuid::new_v4().to_string();
        conn.execute("UPDATE items SET guid=?1 WHERE id=?2", params![guid, own])
            .unwrap();
        let canonical = dispatch(
            &mut conn,
            "items.byCode",
            &json!({"code":format!("everyday:item:{guid}")}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(canonical["id"], own);
        let malformed = dispatch(
            &mut conn,
            "items.byCode",
            &json!({"code":"everyday:item:not-a-guid"}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(malformed.http, 400);
        cleanup(conn, path);
    }

    #[test]
    fn passwordless_account_cannot_log_in() {
        let (conn, path, users, _ws) = test_db();
        let error = auth_login(
            &conn,
            &json!({"phone": "+70000000000", "password": "anything"}),
        )
        .unwrap_err();
        assert_eq!(error.http, 401);
        let hash: Option<String> = conn
            .query_row(
                "SELECT password_hash FROM users WHERE id=?1",
                params![users[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert!(hash.is_none());
        cleanup(conn, path);
    }

    #[test]
    fn expired_invite_is_rejected_everywhere() {
        let (mut conn, path, users, ws) = test_db();
        let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        conn.execute(
            "INSERT INTO invites (workspace_id, token, role, created_by, max_uses, expires_at, created_at)
             VALUES (?1,'expired-token','member',?2,20,?3,?4)",
            params![ws, users[0], past, now()],
        )
        .unwrap();

        let info = invite_info(&conn, &json!({"token": "expired-token"})).unwrap_err();
        assert_eq!(info.http, 400);

        let joined = dispatch(
            &mut conn,
            "auth.joinRegister",
            &json!({"token": "expired-token", "fullName": "Поздний", "phone": "+79995550000", "password": "LongEnoughPass1"}),
            None,
        )
        .unwrap_err();
        assert_eq!(joined.http, 400);

        let users_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
            .unwrap();
        assert_eq!(users_after, users.len() as i64);
        cleanup(conn, path);
    }

    #[test]
    fn fresh_invite_carries_expiry_and_role() {
        let (mut conn, path, users, ws) = test_db();
        let created = dispatch(
            &mut conn,
            "admin.workspaces.createInvite",
            &json!({"workspaceId": ws, "role": "viewer", "maxUses": 5}),
            Some(users[0]),
        )
        .unwrap();
        let expires = created["expiresAt"].as_str().expect("expiresAt");
        assert!(chrono::DateTime::parse_from_rfc3339(expires).unwrap() > chrono::Utc::now());
        assert_eq!(created["role"].as_str(), Some("viewer"));
        assert_eq!(created["payload"]["role"].as_str(), Some("viewer"));
        cleanup(conn, path);
    }

    #[test]
    fn viewer_invite_grants_read_only_membership() {
        let (mut conn, path, users, ws) = test_db();
        let item = insert_item(&conn, ws, None, false, None);
        let created = dispatch(
            &mut conn,
            "admin.workspaces.createInvite",
            &json!({"workspaceId": ws, "role": "viewer", "maxUses": 5}),
            Some(users[0]),
        )
        .unwrap();
        let token = created["token"].as_str().unwrap().to_string();
        let joined = dispatch(
            &mut conn,
            "auth.joinRegister",
            &json!({"token": token, "fullName": "Наблюдатель", "phone": "+79995551111", "password": "LongEnoughPass1"}),
            None,
        )
        .unwrap();
        let viewer = joined["id"].as_i64().unwrap();

        let stored: String = conn
            .query_row(
                "SELECT rights_json FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
                params![viewer, ws],
                |r| r.get(0),
            )
            .unwrap();
        let rights: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(rights["viewItems"].as_bool(), Some(true));
        assert_eq!(rights["createItems"].as_bool(), Some(false));

        // Наблюдатель читает каталог, но не создаёт и не берёт предметы —
        // даже если не передаёт workspaceId в запросе.
        dispatch(&mut conn, "items.byId", &json!({"id": item}), Some(viewer)).unwrap();
        let create = dispatch(
            &mut conn,
            "items.create",
            &json!({"title": "Чужой предмет"}),
            Some(viewer),
        )
        .unwrap_err();
        assert_eq!(create.http, 403);
        let take = dispatch(
            &mut conn,
            "transfers.take",
            &json!({"itemId": item}),
            Some(viewer),
        )
        .unwrap_err();
        assert_eq!(take.http, 403);
        cleanup(conn, path);
    }

    #[test]
    fn workspace_right_applies_when_request_omits_workspace_id() {
        let (mut conn, path, users, ws) = test_db();
        let limited = json!({"viewItems": true, "createItems": false});
        conn.execute(
            "UPDATE user_workspaces SET rights_json=?1 WHERE user_id=?2 AND workspace_id=?3",
            params![limited.to_string(), users[1], ws],
        )
        .unwrap();
        let error = dispatch(
            &mut conn,
            "items.create",
            &json!({"title": "Без workspaceId"}),
            Some(users[1]),
        )
        .unwrap_err();
        assert_eq!(error.http, 403);
        let items: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(items, 0);
        cleanup(conn, path);
    }

    #[test]
    fn status_change_to_repair_requires_a_reason() {
        let (mut conn, path, users, ws) = test_db();
        let item = insert_item(&conn, ws, None, false, None);
        conn.execute(
            "INSERT INTO statuses (name, workspace_id, type, slug) VALUES ('В ремонте',?1,'status','in-repair')",
            params![ws],
        )
        .unwrap();
        let repair = conn.last_insert_rowid();

        let refused = dispatch(
            &mut conn,
            "items.update",
            &json!({"id": item, "statusId": repair}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(refused.http, 400);
        let unchanged: Option<i64> = conn
            .query_row(
                "SELECT status_id FROM items WHERE id=?1",
                params![item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unchanged, None);

        dispatch(
            &mut conn,
            "items.update",
            &json!({"id": item, "statusId": repair, "reason": "Сгорел якорь"}),
            Some(users[0]),
        )
        .unwrap();
        let applied: Option<i64> = conn
            .query_row(
                "SELECT status_id FROM items WHERE id=?1",
                params![item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(applied, Some(repair));

        let note: String = conn
            .query_row(
                "SELECT comment FROM history_entries WHERE item_id=?1 ORDER BY id DESC LIMIT 1",
                params![item],
                |r| r.get(0),
            )
            .unwrap();
        assert!(note.contains("В ремонте"), "{note}");
        assert!(note.contains("Сгорел якорь"), "{note}");
        cleanup(conn, path);
    }

    #[test]
    fn new_workspace_gets_statuses_and_a_storage() {
        let (mut conn, path, users, _ws) = test_db();
        let created = dispatch(
            &mut conn,
            "admin.workspaces.create",
            &json!({"name": "Второй объект"}),
            Some(users[0]),
        )
        .unwrap();
        let ws = created["id"].as_i64().unwrap();

        let mut stmt = conn
            .prepare("SELECT slug FROM statuses WHERE workspace_id=?1 ORDER BY slug")
            .unwrap();
        let slugs: Vec<String> = stmt
            .query_map(params![ws], |r| r.get(0))
            .unwrap()
            .filter_map(|x| x.ok())
            .collect();
        drop(stmt);
        for expected in [
            "in-work",
            "in-repair",
            "in-stock",
            "needs-check",
            "written-off",
        ] {
            assert!(
                slugs.iter().any(|s| s == expected),
                "{expected} missing from {slugs:?}"
            );
        }

        let storages: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM storages WHERE workspace_id=?1",
                params![ws],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(storages, 1);

        // Предмет, взятый в новом пространстве, получает статус «В работе».
        let item = dispatch(
            &mut conn,
            "items.create",
            &json!({"workspaceId": ws, "title": "Новый предмет"}),
            Some(users[0]),
        )
        .unwrap();
        let taken = dispatch(
            &mut conn,
            "transfers.take",
            &json!({"itemId": item["id"].as_i64().unwrap()}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(taken["status"]["slug"].as_str(), Some("in-work"));
        cleanup(conn, path);
    }

    #[test]
    fn return_rolls_back_when_ledger_append_fails() {
        let (mut conn, path, users, ws) = test_db();
        let item_id = insert_item(&conn, ws, Some(users[0]), false, None);
        break_ledger(&conn);

        let error =
            transfers_return(&mut conn, &json!({"itemId": item_id}), Some(users[0])).unwrap_err();
        assert_eq!(error.http, 500);

        // Предмет остался у сотрудника: возврат без записи в журнал не считается.
        let responsible: Option<i64> = conn
            .query_row(
                "SELECT responsible_user_id FROM items WHERE id=?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(responsible, Some(users[0]));
        cleanup(conn, path);
    }

    #[test]
    fn quantity_return_adds_stock_back_and_journals_it() {
        let (mut conn, path, users, ws) = test_db();
        let item_id = insert_item(&conn, ws, None, true, Some(10.0));
        dispatch(
            &mut conn,
            "transfers.take",
            &json!({"itemId": item_id, "quantity": 4.0}),
            Some(users[1]),
        )
        .unwrap();
        let after_take: f64 = conn
            .query_row(
                "SELECT quantity FROM items WHERE id=?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!((after_take - 6.0).abs() < 1e-9, "{after_take}");

        dispatch(
            &mut conn,
            "transfers.returnItem",
            &json!({"itemId": item_id, "quantity": 4.0}),
            Some(users[1]),
        )
        .unwrap();
        let after_return: f64 = conn
            .query_row(
                "SELECT quantity FROM items WHERE id=?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!((after_return - 10.0).abs() < 1e-9, "{after_return}");

        let journaled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM history_entries WHERE item_id=?1 AND type='transfer_send'",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(journaled, 1);
        let (custody_entries, custody_balance, linked): (i64, f64, i64) = conn
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(c.quantity_delta),0),
                        SUM(CASE WHEN h.hash IS NOT NULL THEN 1 ELSE 0 END)
                 FROM custody_entries c LEFT JOIN history_entries h ON h.hash=c.ledger_hash
                 WHERE c.item_guid=(SELECT guid FROM items WHERE id=?1)",
                params![item_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(custody_entries, 2);
        assert_eq!(linked, 2);
        assert!(custody_balance.abs() < 1e-9);
        cleanup(conn, path);
    }

    #[test]
    fn quantitative_direct_transfer_moves_holding_without_charging_stock_twice() {
        let (mut conn, path, users, ws) = test_db();
        let item = insert_item(&conn, ws, None, true, Some(10.0));
        dispatch(
            &mut conn,
            "transfers.take",
            &json!({"itemId":item,"quantity":4.0}),
            Some(users[0]),
        )
        .unwrap();
        let transfer = dispatch(
            &mut conn,
            "transfers.prepare",
            &json!({"itemId":item,"toUserId":users[1],"quantity":3.0}),
            Some(users[0]),
        )
        .unwrap();
        dispatch(
            &mut conn,
            "transfers.accept",
            &json!({"id":transfer["id"]}),
            Some(users[1]),
        )
        .unwrap();
        let stock: f64 = conn
            .query_row("SELECT quantity FROM items WHERE id=?1", [item], |row| {
                row.get(0)
            })
            .unwrap();
        let held = |user: i64| -> f64 {
            conn.query_row(
                "SELECT COALESCE(SUM(quantity),0) FROM item_holdings
                 WHERE item_id=?1 AND user_id=?2 AND returned_at IS NULL",
                params![item, user],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!((stock - 6.0).abs() < 1e-9, "stock charged twice: {stock}");
        assert!((held(users[0]) - 1.0).abs() < 1e-9);
        assert!((held(users[1]) - 3.0).abs() < 1e-9);
        let (sender_delta, recipient_delta): (f64, f64) = conn
            .query_row(
                "SELECT
                   COALESCE(SUM(CASE WHEN user_guid=(SELECT guid FROM users WHERE id=?1) THEN quantity_delta ELSE 0 END),0),
                   COALESCE(SUM(CASE WHEN user_guid=(SELECT guid FROM users WHERE id=?2) THEN quantity_delta ELSE 0 END),0)
                 FROM custody_entries WHERE item_guid=(SELECT guid FROM items WHERE id=?3)",
                params![users[0],users[1],item],
                |row| Ok((row.get(0)?,row.get(1)?)),
            )
            .unwrap();
        assert!((sender_delta - 1.0).abs() < 1e-9);
        assert!((recipient_delta - 3.0).abs() < 1e-9);
        cleanup(conn, path);
    }

    #[test]
    fn inventory_discrepancy_becomes_a_correcting_ledger_entry() {
        let (mut conn, path, users, ws) = test_db();
        let item = insert_item(&conn, ws, None, true, Some(10.0));
        let session = dispatch(
            &mut conn,
            "inventory.create",
            &json!({"workspaceId": ws}),
            Some(users[0]),
        )
        .unwrap();
        let sid = session["id"].as_i64().unwrap();

        dispatch(
            &mut conn,
            "inventory.checkItem",
            &json!({"sessionId": sid, "itemId": item, "checked": true, "actualQty": 7.0}),
            Some(users[0]),
        )
        .unwrap();
        let done = dispatch(
            &mut conn,
            "inventory.complete",
            &json!({"sessionId": sid}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(done["corrections"].as_u64(), Some(1));

        // Остаток приведён к фактическому.
        let qty: f64 = conn
            .query_row(
                "SELECT quantity FROM items WHERE id=?1",
                params![item],
                |r| r.get(0),
            )
            .unwrap();
        assert!((qty - 7.0).abs() < 1e-9, "{qty}");

        // История сохранила корректировку с дельтой, а не переписала прошлое.
        let (delta, comment): (f64, String) = conn
            .query_row(
                "SELECT quantity_delta, comment FROM history_entries
                 WHERE item_id=?1 AND type='inventory' ORDER BY id DESC LIMIT 1",
                params![item],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!((delta + 3.0).abs() < 1e-9, "{delta}");
        assert!(comment.contains("фактически 7"), "{comment}");
        cleanup(conn, path);
    }

    #[test]
    fn auth_options_opens_registration_only_until_bootstrap() {
        let path = std::env::temp_dir().join(format!("meshkeeper-boot-{}.db", Uuid::new_v4()));
        let mut conn = db::open(&path).expect("test database");

        let empty = dispatch(&mut conn, "auth.options", &Value::Null, None).unwrap();
        assert_eq!(empty["registrationOpen"].as_bool(), Some(true));
        assert_eq!(empty["bootstrap"].as_bool(), Some(true));

        dispatch(
            &mut conn,
            "auth.register",
            &json!({"fullName": "Владелец", "phone": "+79990000000", "password": "LongEnoughPass1", "workspaceName": "Объект"}),
            None,
        )
        .unwrap();

        let after = dispatch(&mut conn, "auth.options", &Value::Null, None).unwrap();
        assert_eq!(after["registrationOpen"].as_bool(), Some(false));
        assert_eq!(after["bootstrap"].as_bool(), Some(false));
        cleanup(conn, path);
    }

    #[test]
    fn fault_and_change_requests_stay_inside_their_workspace() {
        let (mut conn, path, users, _ws) = test_db();
        // Второе пространство с собственным предметом и заявками.
        conn.execute(
            "INSERT INTO workspaces (name, timezone, internal_id_prefix, created_at) VALUES ('Other','UTC','O-',?1)",
            params![now()],
        )
        .unwrap();
        let other_ws = conn.last_insert_rowid();
        let other_item = insert_item(&conn, other_ws, None, false, None);
        conn.execute(
            "INSERT INTO faults (item_id, workspace_id, author_id, severity, description, created_at)
             VALUES (?1,?2,?3,'high','Чужая поломка',?4)",
            params![other_item, other_ws, users[0], now()],
        )
        .unwrap();
        let fault_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO change_requests (item_id, workspace_id, author_id, payload, created_at)
             VALUES (?1,?2,?3,'{\"title\":\"Взлом\"}',?4)",
            params![other_item, other_ws, users[0], now()],
        )
        .unwrap();
        let change_id = conn.last_insert_rowid();

        // users[0] состоит только в `ws`, но не в `other_ws`.
        let fault_err = dispatch(
            &mut conn,
            "items.resolveFault",
            &json!({"id": fault_id, "status": "resolved"}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(fault_err.http, 403);

        let change_err = dispatch(
            &mut conn,
            "items.decideChange",
            &json!({"id": change_id, "accept": true}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(change_err.http, 403);

        let title: String = conn
            .query_row(
                "SELECT title FROM items WHERE id=?1",
                params![other_item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "Test item");
        cleanup(conn, path);
    }

    #[test]
    fn reported_fault_puts_item_under_review_and_blocks_checkout() {
        let (mut conn, path, users, ws) = test_db();
        seed_workspace_defaults(&conn, ws, users[0]).unwrap();
        let item = insert_item(&conn, ws, None, false, None);
        dispatch(
            &mut conn,
            "items.reportFault",
            &json!({"itemId": item, "severity": "high", "description": "Не держит патрон"}),
            Some(users[1]),
        )
        .unwrap();

        let slug: String = conn
            .query_row(
                "SELECT s.slug FROM items i JOIN statuses s ON s.id=i.status_id WHERE i.id=?1",
                params![item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(slug, "needs-check");

        let blocked = dispatch(
            &mut conn,
            "transfers.take",
            &json!({"itemId": item}),
            Some(users[1]),
        )
        .unwrap_err();
        assert_eq!(blocked.http, 400);

        let journaled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM history_entries WHERE item_id=?1",
                params![item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(journaled, 1);
        cleanup(conn, path);
    }

    #[test]
    fn resolving_legacy_fault_adopts_it_into_signed_record_model() {
        let (mut conn, path, users, ws) = test_db();
        seed_workspace_defaults(&conn, ws, users[0]).unwrap();
        let item = insert_item(&conn, ws, None, false, None);
        conn.execute("INSERT INTO faults(item_id,workspace_id,author_id,severity,description,created_at) VALUES(?1,?2,?3,'medium','Старая запись',?4)",params![item,ws,users[1],now()]).unwrap();
        let id = conn.last_insert_rowid();
        let result = dispatch(
            &mut conn,
            "items.resolveFault",
            &json!({"id":id,"status":"resolved","comment":"Проверено после обновления"}),
            Some(users[0]),
        )
        .unwrap();
        assert!(result["guid"].as_str().is_some_and(|v| !v.is_empty()));
        let (depth,event_type):(i64,String)=conn.query_row("SELECT r.depth,h.type FROM fault_records r JOIN history_entries h ON h.hash=r.ledger_hash WHERE r.fault_guid=?1",[result["guid"].as_str().unwrap()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(depth, 0);
        assert_eq!(event_type, "fault_adopt");
        cleanup(conn, path);
    }

    #[test]
    fn accepted_change_request_that_cannot_apply_is_not_marked_accepted() {
        let (mut conn, path, users, ws) = test_db();
        seed_workspace_defaults(&conn, ws, users[0]).unwrap();
        let item = insert_item(&conn, ws, None, false, None);
        let written_off: i64 = conn
            .query_row(
                "SELECT id FROM statuses WHERE workspace_id=?1 AND slug='written-off'",
                params![ws],
                |r| r.get(0),
            )
            .unwrap();
        // Заявка меняет статус на «Списан», но причины в ней нет.
        conn.execute(
            "INSERT INTO change_requests (item_id, workspace_id, author_id, payload, created_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                item,
                ws,
                users[1],
                json!({"statusId": written_off}).to_string(),
                now()
            ],
        )
        .unwrap();
        let change_id = conn.last_insert_rowid();

        // Причина берётся из решения администратора — заявка применяется.
        dispatch(
            &mut conn,
            "items.decideChange",
            &json!({"id": change_id, "accept": true, "reason": "Утилизирован по акту"}),
            Some(users[0]),
        )
        .unwrap();
        let status_id: Option<i64> = conn
            .query_row(
                "SELECT status_id FROM items WHERE id=?1",
                params![item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status_id, Some(written_off));
        let adopted:String=conn.query_row("SELECT h.type FROM change_request_records r JOIN history_entries h ON h.hash=r.ledger_hash WHERE r.request_guid=(SELECT guid FROM change_requests WHERE id=?1)",[change_id],|r|r.get(0)).unwrap();
        assert_eq!(adopted, "change_adopt");

        // Повторное решение по той же заявке отклоняется.
        let again = dispatch(
            &mut conn,
            "items.decideChange",
            &json!({"id": change_id, "accept": false}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(again.http, 409);
        cleanup(conn, path);
    }

    #[test]
    fn written_off_item_cannot_be_transferred() {
        let (mut conn, path, users, ws) = test_db();
        seed_workspace_defaults(&conn, ws, users[0]).unwrap();
        let item = insert_item(&conn, ws, Some(users[0]), false, None);
        let written_off: i64 = conn
            .query_row(
                "SELECT id FROM statuses WHERE workspace_id=?1 AND slug='written-off'",
                params![ws],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE items SET status_id=?1 WHERE id=?2",
            params![written_off, item],
        )
        .unwrap();

        let error = dispatch(
            &mut conn,
            "transfers.prepare",
            &json!({"itemId": item, "toUserId": users[1]}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(error.http, 400);
        let transfers: i64 = conn
            .query_row("SELECT COUNT(*) FROM transfers", [], |r| r.get(0))
            .unwrap();
        assert_eq!(transfers, 0);
        cleanup(conn, path);
    }

    #[test]
    fn removing_a_member_keeps_their_history() {
        let (mut conn, path, users, ws) = test_db();
        seed_workspace_defaults(&conn, ws, users[0]).unwrap();
        let item = insert_item(&conn, ws, None, false, None);
        dispatch(
            &mut conn,
            "transfers.take",
            &json!({"itemId": item}),
            Some(users[1]),
        )
        .unwrap();

        // Себя удалить нельзя.
        let self_remove = dispatch(
            &mut conn,
            "admin.users.remove",
            &json!({"id": users[0], "workspaceId": ws}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(self_remove.http, 400);

        let result = dispatch(
            &mut conn,
            "admin.users.remove",
            &json!({"id": users[1], "workspaceId": ws}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(result["deleted"].as_bool(), Some(false));

        // Учётная запись заблокирована и выведена из пространства…
        let status: String = conn
            .query_row(
                "SELECT status FROM users WHERE id=?1",
                params![users[1]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "disabled");
        let member: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_workspaces WHERE user_id=?1 AND workspace_id=?2",
                params![users[1], ws],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(member, 0);

        // …но записи журнала остались на месте.
        let entries: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM history_entries WHERE actor_user_id=?1",
                params![users[1]],
                |r| r.get(0),
            )
            .unwrap();
        assert!(entries > 0);
        cleanup(conn, path);
    }

    #[test]
    fn repeated_bad_passwords_lock_the_account_out() {
        let (conn, path, users, _ws) = test_db();
        conn.execute(
            "UPDATE users SET password_hash=?1, phone='+79001234567' WHERE id=?2",
            params![hash_password("CorrectHorse1"), users[0]],
        )
        .unwrap();

        // Первые попытки просто отклоняются.
        for _ in 0..LOGIN_FREE_ATTEMPTS {
            let e = auth_login(
                &conn,
                &json!({"phone": "+7 900 123-45-67", "password": "wrong"}),
            )
            .unwrap_err();
            assert_eq!(e.http, 401, "{}", e.message);
        }

        // Дальше включается пауза.
        let locked = auth_login(
            &conn,
            &json!({"phone": "+7 900 123-45-67", "password": "wrong"}),
        )
        .unwrap_err();
        assert_eq!(locked.http, 429, "{}", locked.message);

        // Верный пароль в этот момент тоже не проходит — иначе паузу
        // можно было бы обойти, угадав с шестого раза.
        let blocked = auth_login(
            &conn,
            &json!({"phone": "+7 900 123-45-67", "password": "CorrectHorse1"}),
        )
        .unwrap_err();
        assert_eq!(blocked.http, 429);

        // После снятия блокировки вход работает и счётчик обнуляется.
        conn.execute("UPDATE login_throttle SET locked_until=NULL", [])
            .unwrap();
        auth_login(
            &conn,
            &json!({"phone": "+7 900 123-45-67", "password": "CorrectHorse1"}),
        )
        .unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM login_throttle", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
        cleanup(conn, path);
    }

    #[test]
    fn change_request_shows_before_and_after() {
        let (mut conn, path, users, ws) = test_db();
        seed_workspace_defaults(&conn, ws, users[0]).unwrap();
        let item = insert_item(&conn, ws, None, false, None);
        conn.execute(
            "INSERT INTO categories (name, workspace_id, type) VALUES ('Электроинструмент',?1,'category')",
            params![ws],
        )
        .unwrap();
        let category = conn.last_insert_rowid();

        dispatch(
            &mut conn,
            "items.requestChange",
            &json!({
                "itemId": item,
                "payload": {"title": "Перфоратор Bosch", "categoryId": category},
                "comment": "уточнил модель"
            }),
            Some(users[1]),
        )
        .unwrap();

        let list = dispatch(
            &mut conn,
            "items.changeRequests",
            &json!({"workspaceId": ws}),
            Some(users[0]),
        )
        .unwrap();
        let changes = list[0]["changes"].as_array().expect("changes");
        assert_eq!(changes.len(), 2, "{changes:?}");

        let title = changes.iter().find(|c| c["field"] == "title").unwrap();
        assert_eq!(title["label"].as_str(), Some("Наименование"));
        assert_eq!(title["before"].as_str(), Some("Test item"));
        assert_eq!(title["after"].as_str(), Some("Перфоратор Bosch"));

        // Идентификатор категории развёрнут в название, а не показан числом.
        let cat = changes.iter().find(|c| c["field"] == "categoryId").unwrap();
        assert!(cat["before"].is_null());
        assert_eq!(cat["after"].as_str(), Some("Электроинструмент"));
        cleanup(conn, path);
    }

    #[test]
    fn change_request_hides_fields_that_do_not_change() {
        let (mut conn, path, users, ws) = test_db();
        let item = insert_item(&conn, ws, None, false, None);
        dispatch(
            &mut conn,
            "items.requestChange",
            &json!({"itemId": item, "payload": {"title": "Test item"}}),
            Some(users[1]),
        )
        .unwrap();
        let list = dispatch(
            &mut conn,
            "items.changeRequests",
            &json!({"workspaceId": ws}),
            Some(users[0]),
        )
        .unwrap();
        assert!(list[0]["changes"].as_array().unwrap().is_empty());
        cleanup(conn, path);
    }

    #[test]
    fn photo_and_location_rights_hide_fields_in_every_shape() {
        let (mut conn, path, users, ws) = test_db();
        seed_workspace_defaults(&conn, ws, users[0]).unwrap();
        let item = insert_item(&conn, ws, None, false, None);
        conn.execute(
            "INSERT INTO item_photos (item_id, url, is_title) VALUES (?1,'data:image/png;base64,AAA',1)",
            params![item],
        )
        .unwrap();
        let storage: i64 = conn
            .query_row(
                "SELECT id FROM storages WHERE workspace_id=?1",
                params![ws],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE items SET storage_id=?1 WHERE id=?2",
            params![storage, item],
        )
        .unwrap();

        // Владелец видит всё.
        let full = dispatch(
            &mut conn,
            "items.byId",
            &json!({"id": item}),
            Some(users[0]),
        )
        .unwrap();
        assert!(!full["photos"].as_array().unwrap().is_empty());
        assert!(!full["storage"].is_null());

        // У второго пользователя оба права сняты.
        let limited = json!({"viewItems": true, "viewPhotos": false, "viewLocation": false});
        conn.execute(
            "UPDATE user_workspaces SET rights_json=?1 WHERE user_id=?2 AND workspace_id=?3",
            params![limited.to_string(), users[1], ws],
        )
        .unwrap();

        let hidden = dispatch(
            &mut conn,
            "items.byId",
            &json!({"id": item}),
            Some(users[1]),
        )
        .unwrap();
        assert!(hidden.get("photos").is_none(), "{hidden}");
        assert!(hidden.get("storage").is_none(), "{hidden}");
        assert!(hidden.get("storageId").is_none(), "{hidden}");
        // Само название и статус остаются — каталог смотреть можно.
        assert_eq!(hidden["title"].as_str(), Some("Test item"));

        // И во вложенных формах: список тоже вычищен.
        let list = dispatch(
            &mut conn,
            "reports.allItems",
            &json!({"workspaceId": ws}),
            Some(users[1]),
        )
        .unwrap();
        assert!(list[0].get("storage").is_none(), "{list}");
        cleanup(conn, path);
    }

    #[test]
    fn document_acl_filters_member_accounting_and_manager_files() {
        let (mut conn, path, users, ws) = test_db();
        let item = insert_item(&conn, ws, None, false, None);
        for (name, access) in [
            ("Общая инструкция", "members"),
            ("Счёт", "accounting"),
            ("Акт руководителя", "managers"),
        ] {
            conn.execute("INSERT INTO item_documents(item_id,name,url,guid,access_level) VALUES(?1,?2,?3,?4,?5)",
                params![item,name,format!("https://example.invalid/{access}"),Uuid::new_v4().to_string(),access]).unwrap();
        }
        let owner = dispatch(&mut conn, "items.byId", &json!({"id":item}), Some(users[0])).unwrap();
        assert_eq!(owner["documents"].as_array().unwrap().len(), 3);

        let member = json!({"viewItems":true,"viewDocuments":true,"viewAccounting":false,"manageDocuments":false});
        conn.execute(
            "UPDATE user_workspaces SET rights_json=?1 WHERE user_id=?2 AND workspace_id=?3",
            params![member.to_string(), users[1], ws],
        )
        .unwrap();
        let visible =
            dispatch(&mut conn, "items.byId", &json!({"id":item}), Some(users[1])).unwrap();
        let docs = visible["documents"].as_array().unwrap();
        assert_eq!(docs.len(), 1, "{visible}");
        assert_eq!(docs[0]["accessLevel"], "members");

        let denied = json!({"viewItems":true,"viewDocuments":false});
        conn.execute(
            "UPDATE user_workspaces SET rights_json=?1 WHERE user_id=?2 AND workspace_id=?3",
            params![denied.to_string(), users[1], ws],
        )
        .unwrap();
        let hidden =
            dispatch(&mut conn, "items.byId", &json!({"id":item}), Some(users[1])).unwrap();
        assert!(hidden.get("documents").is_none(), "{hidden}");
        cleanup(conn, path);
    }

    #[test]
    fn one_person_has_distinct_positions_in_multiple_organizations() {
        let (conn, path, users, first_ws) = test_db();
        conn.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at) VALUES('Вторая организация','B-',?1)",[now()]).unwrap();
        let second_ws = conn.last_insert_rowid();
        conn.execute("UPDATE user_workspaces SET position='Кладовщик',role_name='Материально ответственное лицо',personnel_number='A-17' WHERE user_id=?1 AND workspace_id=?2",params![users[1],first_ws]).unwrap();
        conn.execute("INSERT INTO user_workspaces(user_id,workspace_id,rights_json,position,role_name,personnel_number) VALUES(?1,?2,?3,'Аудитор','Наблюдатель','B-04')",params![users[1],second_ws,db::default_rights().to_string()]).unwrap();
        let first = admin_users(&conn, &json!({"workspaceId":first_ws})).unwrap();
        let second = admin_users(&conn, &json!({"workspaceId":second_ws})).unwrap();
        let a = first
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["id"] == users[1])
            .unwrap();
        let b = second
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["id"] == users[1])
            .unwrap();
        assert_eq!(a["position"], "Кладовщик");
        assert_eq!(a["personnelNumber"], "A-17");
        assert_eq!(b["position"], "Аудитор");
        assert_eq!(b["organizationRole"], "Наблюдатель");
        cleanup(conn, path);
    }

    #[test]
    fn membership_administration_is_atomic_and_ledger_bound() {
        let (mut conn, path, users, ws) = test_db();
        let created = dispatch(
            &mut conn,
            "admin.users.create",
            &json!({
                "workspaceId": ws,
                "fullName": "Новый аудитор",
                "phone": "+7 900 123-45-67",
                "organizationRole": "Аудитор"
            }),
            Some(users[0]),
        )
        .unwrap();
        let target = created["id"].as_i64().unwrap();
        dispatch(
            &mut conn,
            "admin.users.update",
            &json!({
                "workspaceId": ws,
                "id": target,
                "organizationRole": "Старший аудитор",
                "personnelNumber": "AUD-7"
            }),
            Some(users[0]),
        )
        .unwrap();
        dispatch(
            &mut conn,
            "admin.users.remove",
            &json!({"workspaceId": ws, "id": target}),
            Some(users[0]),
        )
        .unwrap();

        let types: Vec<String> = conn
            .prepare("SELECT type FROM history_entries WHERE type LIKE 'membership_%' ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(
            types,
            [
                "membership_create",
                "membership_update",
                "membership_remove"
            ]
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM users WHERE id=?1", [target], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        ledger::verify_all(&conn).unwrap();
        cleanup(conn, path);
    }

    #[test]
    fn bit_transfer_is_atomic_balanced_and_permission_checked() {
        let (mut conn, path, users, ws) = test_db();
        let minted=dispatch(&mut conn,"bit.mint",&json!({"workspaceId":ws,"recipientUserId":users[0],"amount":100,"memo":"Начальная эмиссия"}),Some(users[0])).unwrap();
        assert_eq!(minted["status"], "posted");
        let sent = dispatch(
            &mut conn,
            "bit.transfer",
            &json!({"workspaceId":ws,"recipientUserId":users[1],"amount":40,"memo":"Работа"}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(sent["status"], "posted");
        assert_eq!(
            bit_balance(&conn, &json!({"workspaceId":ws}), Some(users[0])).unwrap()["balance"],
            60
        );
        assert_eq!(
            bit_balance(&conn, &json!({"workspaceId":ws}), Some(users[1])).unwrap()["balance"],
            40
        );
        let item = insert_item(&conn, ws, None, false, None);
        let sale = dispatch(
            &mut conn,
            "bit.sale",
            &json!({"itemId":item,"sellerUserId":users[1],"amount":10,"memo":"Покупка расходника"}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(sale["kind"], "sale");
        assert_eq!(
            bit_balance(&conn, &json!({"workspaceId":ws}), Some(users[0])).unwrap()["balance"],
            50
        );
        assert_eq!(
            bit_balance(&conn, &json!({"workspaceId":ws}), Some(users[1])).unwrap()["balance"],
            50
        );
        let before: i64 = conn
            .query_row("SELECT count(*) FROM accounting_transactions", [], |r| {
                r.get(0)
            })
            .unwrap();
        let rejected = dispatch(
            &mut conn,
            "bit.transfer",
            &json!({"workspaceId":ws,"recipientUserId":users[1],"amount":1000}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(rejected.http, 409);
        let after: i64 = conn
            .query_row("SELECT count(*) FROM accounting_transactions", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            before, after,
            "отклонённый перевод не должен оставлять полупроводку"
        );
        crate::accounting::verify(&conn).unwrap();

        conn.execute(
            "UPDATE user_workspaces SET rights_json=?1 WHERE user_id=?2 AND workspace_id=?3",
            params![db::viewer_rights().to_string(), users[1], ws],
        )
        .unwrap();
        let denied = dispatch(
            &mut conn,
            "bit.transfer",
            &json!({"workspaceId":ws,"recipientUserId":users[0],"amount":1}),
            Some(users[1]),
        )
        .unwrap_err();
        assert_eq!(denied.http, 403);
        cleanup(conn, path);
    }

    #[test]
    fn knowledge_revisions_use_cas_acl_and_tamper_evidence() {
        let (mut conn, path, users, ws) = test_db();
        let page=dispatch(&mut conn,"knowledge.save",&json!({"workspaceId":ws,"slug":"safety/drill","title":"Работа с дрелью","content":"# Инструкция\nОтключить питание.","attachments":[{"name":"Схема","url":"data:image/png;base64,QUJD"}]}),Some(users[0])).unwrap();
        assert_eq!(page["hasConflict"], false);
        assert!(page["current"]["attachments"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png"));
        crate::knowledge::verify(&conn).unwrap();
        let limited = json!({"viewKnowledge":true,"editKnowledge":false,"viewAccounting":false});
        conn.execute(
            "UPDATE user_workspaces SET rights_json=?1 WHERE user_id=?2 AND workspace_id=?3",
            params![limited.to_string(), users[1], ws],
        )
        .unwrap();
        let list = dispatch(
            &mut conn,
            "knowledge.list",
            &json!({"workspaceId":ws}),
            Some(users[1]),
        )
        .unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(
            dispatch(
                &mut conn,
                "knowledge.save",
                &json!({"workspaceId":ws,"slug":"forbidden","title":"Нет","content":"x"}),
                Some(users[1])
            )
            .unwrap_err()
            .http,
            403
        );
        dispatch(&mut conn,"knowledge.save",&json!({"workspaceId":ws,"slug":"finance","title":"Бюджет","content":"Закрыто","visibility":"accounting"}),Some(users[0])).unwrap();
        let filtered = dispatch(
            &mut conn,
            "knowledge.list",
            &json!({"workspaceId":ws}),
            Some(users[1]),
        )
        .unwrap();
        assert_eq!(filtered.as_array().unwrap().len(), 1);
        conn.execute(
            "UPDATE knowledge_revisions SET content='подмена' WHERE guid=?1",
            [page["savedRevisionGuid"].as_str().unwrap()],
        )
        .unwrap();
        assert!(crate::knowledge::verify(&conn).is_err());
        cleanup(conn, path);
    }

    #[test]
    fn rights_saved_before_a_new_permission_existed_keep_working() {
        let (mut conn, path, users, ws) = test_db();
        let item = insert_item(&conn, ws, None, false, None);
        // Старая запись прав: про viewPhotos/viewLocation она ничего не знает.
        let legacy = json!({"viewItems": true, "createItems": true});
        conn.execute(
            "UPDATE user_workspaces SET rights_json=?1 WHERE user_id=?2 AND workspace_id=?3",
            params![legacy.to_string(), users[1], ws],
        )
        .unwrap();

        let seen = dispatch(
            &mut conn,
            "items.byId",
            &json!({"id": item}),
            Some(users[1]),
        )
        .unwrap();
        assert!(
            seen.get("photos").is_some(),
            "поле фото пропало у старой записи прав"
        );
        cleanup(conn, path);
    }

    #[test]
    fn write_off_photo_can_be_required_by_the_group() {
        let (mut conn, path, users, ws) = test_db();
        seed_workspace_defaults(&conn, ws, users[0]).unwrap();
        let first = insert_item(&conn, ws, None, false, None);
        let second = insert_item(&conn, ws, None, false, None);

        // По умолчанию фото не требуется — достаточно причины.
        dispatch(
            &mut conn,
            "history.writeOff",
            &json!({"itemId": first, "comment": "Сломан безвозвратно"}),
            Some(users[0]),
        )
        .unwrap();

        dispatch(
            &mut conn,
            "admin.workspaces.update",
            &json!({"id": ws, "requireWriteoffPhoto": true}),
            Some(users[0]),
        )
        .unwrap();

        let refused = dispatch(
            &mut conn,
            "history.writeOff",
            &json!({"itemId": second, "comment": "Утилизирован"}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(refused.http, 400, "{}", refused.message);

        dispatch(
            &mut conn,
            "history.writeOff",
            &json!({
                "itemId": second,
                "comment": "Утилизирован по акту",
                "photoUrl": "data:image/png;base64,AAAA"
            }),
            Some(users[0]),
        )
        .unwrap();

        // Фото сохранено при записи журнала, а не потеряно.
        let stored: Option<String> = conn
            .query_row(
                "SELECT photo_url FROM history_entries WHERE item_id=?1 AND type='write_off'",
                params![second],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored.as_deref(), Some("data:image/png;base64,AAAA"));
        cleanup(conn, path);
    }

    #[test]
    fn photos_get_a_thumbnail_and_a_checksum() {
        let (mut conn, path, users, ws) = test_db();
        let full = "data:image/jpeg;base64,QQQQQQQQQQQQ";
        let thumb = "data:image/jpeg;base64,VGh1bWI=";

        let created = dispatch(
            &mut conn,
            "items.create",
            &json!({
                "workspaceId": ws,
                "title": "Перфоратор",
                "photos": [{"url": full, "thumbUrl": thumb}]
            }),
            Some(users[0]),
        )
        .unwrap();
        let item = created["id"].as_i64().unwrap();

        let photo = &created["photos"][0];
        assert_eq!(photo["url"].as_str(), Some(full));
        assert_eq!(photo["thumbUrl"].as_str(), Some(thumb));
        // Контрольная сумма считается сервером, а не приходит от клиента.
        let expected = "c4440406fc1aa8365f34bf9d3a2e0cf08ace02c65de7847db1c542e5cce9f2ad";
        assert_eq!(photo["sha256"].as_str(), Some(expected));
        assert_eq!(expected.len(), 64);

        // В списке каталога оригинал не отдаётся — только миниатюра.
        let list = dispatch(
            &mut conn,
            "items.list",
            &json!({"workspaceId": ws}),
            Some(users[0]),
        )
        .unwrap();
        let listed = &list["rows"][0]["photos"][0];
        assert_eq!(
            listed["url"].as_str(),
            Some(thumb),
            "в списке уехал оригинал"
        );
        assert_eq!(listed["thumbUrl"].as_str(), Some(thumb));

        // В карточке оригинал по-прежнему доступен.
        let card = dispatch(
            &mut conn,
            "items.byId",
            &json!({"id": item}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(card["photos"][0]["url"].as_str(), Some(full));
        cleanup(conn, path);
    }

    #[test]
    fn photos_added_as_plain_strings_still_work() {
        let (mut conn, path, users, ws) = test_db();
        let url = "data:image/png;base64,AAAA";
        let created = dispatch(
            &mut conn,
            "items.create",
            &json!({"workspaceId": ws, "title": "Дрель", "photos": [url]}),
            Some(users[0]),
        )
        .unwrap();
        let photo = &created["photos"][0];
        assert_eq!(photo["url"].as_str(), Some(url));
        // Миниатюры нет — подставляется оригинал, карточка не остаётся пустой.
        assert_eq!(photo["thumbUrl"].as_str(), Some(url));
        assert!(photo["sha256"].as_str().is_some());
        cleanup(conn, path);
    }

    #[test]
    fn organization_tree_supports_arbitrary_depth_and_rejects_cycles() {
        let (mut conn, path, users, ws) = test_db();
        let division = dispatch(
            &mut conn,
            "admin.organizationNodes.create",
            &json!({"workspaceId":ws,"kind":"division","name":"Производство","tabLabel":"Цеха"}),
            Some(users[0]),
        )
        .unwrap();
        let warehouse = dispatch(
            &mut conn,
            "admin.organizationNodes.create",
            &json!({"workspaceId":ws,"parentId":division["id"],"kind":"warehouse","name":"Склад №1"}),
            Some(users[0]),
        ).unwrap();
        let cabinet = dispatch(
            &mut conn,
            "admin.organizationNodes.create",
            &json!({"workspaceId":ws,"parentId":warehouse["id"],"kind":"cabinet","name":"Кабинет 204"}),
            Some(users[0]),
        ).unwrap();

        let nodes = dispatch(
            &mut conn,
            "admin.organizationNodes.list",
            &json!({"workspaceId":ws}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(nodes.as_array().unwrap().len(), 3);
        assert_eq!(cabinet["parentId"], warehouse["id"]);
        assert_eq!(division["tabLabel"], "Цеха");

        let error = dispatch(
            &mut conn,
            "admin.organizationNodes.update",
            &json!({"id":division["id"],"parentId":cabinet["id"]}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(error.http, 400);
        assert!(error.message.contains("дочернего"));
        cleanup(conn, path);
    }

    #[test]
    fn occupied_organization_node_cannot_be_archived() {
        let (mut conn, path, users, ws) = test_db();
        let parent = dispatch(
            &mut conn,
            "admin.organizationNodes.create",
            &json!({"workspaceId":ws,"kind":"site","name":"Объект"}),
            Some(users[0]),
        )
        .unwrap();
        let child = dispatch(
            &mut conn,
            "admin.organizationNodes.create",
            &json!({"workspaceId":ws,"parentId":parent["id"],"kind":"room","name":"Комната"}),
            Some(users[0]),
        )
        .unwrap();
        let error = dispatch(
            &mut conn,
            "admin.organizationNodes.remove",
            &json!({"id":parent["id"]}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(error.http, 409);

        dispatch(
            &mut conn,
            "admin.organizationNodes.remove",
            &json!({"id":child["id"]}),
            Some(users[0]),
        )
        .unwrap();
        dispatch(
            &mut conn,
            "admin.organizationNodes.remove",
            &json!({"id":parent["id"]}),
            Some(users[0]),
        )
        .unwrap();
        let visible = dispatch(
            &mut conn,
            "admin.organizationNodes.list",
            &json!({"workspaceId":ws}),
            Some(users[0]),
        )
        .unwrap();
        assert!(visible.as_array().unwrap().is_empty());
        cleanup(conn, path);
    }

    #[test]
    fn item_location_uses_tree_and_rejects_foreign_workspace_node() {
        let (mut conn, path, users, ws) = test_db();
        let room = dispatch(
            &mut conn,
            "admin.organizationNodes.create",
            &json!({"workspaceId":ws,"kind":"room","name":"Кабинет 204"}),
            Some(users[0]),
        )
        .unwrap();
        let item = dispatch(
            &mut conn,
            "items.create",
            &json!({"workspaceId":ws,"title":"Осциллограф","organizationNodeId":room["id"]}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(item["organizationNode"]["name"], "Кабинет 204");

        let other_ws = ws_create(
            &mut conn,
            &json!({"name":"Чужая организация"}),
            Some(users[0]),
        )
        .unwrap()["id"]
            .as_i64()
            .unwrap();
        let foreign = organization_node_create_atomic(
            &conn,
            &json!({"workspaceId":other_ws,"kind":"warehouse","name":"Чужой склад"}),
            Some(users[0]),
        )
        .unwrap();
        let error = dispatch(
            &mut conn,
            "items.update",
            &json!({"id":item["id"],"organizationNodeId":foreign["id"]}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(error.http, 400);
        assert!(error.message.contains("другому рабочему пространству"));
        cleanup(conn, path);
    }

    #[test]
    fn chat_message_is_ledger_bound_and_duplicate_is_rejected() {
        let (mut conn, path, users, ws) = test_db();
        let sent = dispatch(
            &mut conn,
            "chat.send",
            &json!({"workspaceId":ws,"text":"Проверка связи"}),
            Some(users[0]),
        )
        .unwrap();
        assert_eq!(sent["ledgerVerified"], true);
        assert!(sent["guid"].as_str().is_some_and(|value| !value.is_empty()));
        assert!(sent["ledgerHash"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert_eq!(ledger::verify_chat_links(&conn).unwrap(), 1);

        let duplicate = dispatch(
            &mut conn,
            "chat.send",
            &json!({"workspaceId":ws,"text":"Проверка связи"}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(duplicate.http, 409);
        conn.execute("UPDATE chat_messages SET text='подмена'", [])
            .unwrap();
        assert!(ledger::verify_chat_links(&conn).is_err());
        cleanup(conn, path);
    }

    #[test]
    fn chat_rejects_foreign_workspace() {
        let (mut conn, path, users, _ws) = test_db();
        conn.execute(
            "INSERT INTO workspaces(name,timezone,internal_id_prefix,created_at) VALUES('Чужая','UTC','X-',?1)",
            params![now()],
        )
        .unwrap();
        let foreign = conn.last_insert_rowid();
        let error = dispatch(
            &mut conn,
            "chat.list",
            &json!({"workspaceId":foreign}),
            Some(users[0]),
        )
        .unwrap_err();
        assert_eq!(error.http, 403);
        cleanup(conn, path);
    }
}
