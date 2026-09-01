mod accounting;
mod api;
mod auth;
mod content;
mod db;
mod device;
mod diagnostics;
mod discovery;
pub mod interorg;
mod json;
mod knowledge;
mod ledger;
pub mod stream_transport;
mod sync;

use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use tower_http::services::ServeDir;

struct AppState {
    db: Mutex<Connection>,
}

const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

/// Единый защитный контур для API и SPA. Заголовки выставляются самим узлом,
/// поэтому защита остаётся и при ошибочной конфигурации reverse proxy.
async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let sensitive =
        request.uri().path().starts_with("/api/") || request.uri().path().starts_with("/sync/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(self), microphone=(), geolocation=()"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' https: wss:; worker-src 'self' blob:",
        ),
    );
    headers.insert(
        HeaderName::from_static("x-permitted-cross-domain-policies"),
        HeaderValue::from_static("none"),
    );
    if sensitive {
        headers.insert("cache-control", HeaderValue::from_static("no-store"));
    }
    if std::env::var("MESHKEEPER_COOKIE_SECURE").as_deref() == Ok("1") {
        headers.insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

fn unwrap_json(v: &Value) -> Value {
    if let Some(inner) = v.get("json") {
        if inner.get("json").is_some() && inner.get("meta").is_some() {
            return inner.get("json").cloned().unwrap_or(Value::Null);
        }
        return inner.clone();
    }
    v.clone()
}

fn parse_calls(
    procedures: &str,
    query_input: Option<&str>,
    body: Option<&[u8]>,
) -> Vec<(String, Value)> {
    let names: Vec<String> = procedures
        .split(',')
        .map(|s| s.trim().trim_start_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let raw: Option<Value> = body
        .and_then(|b| {
            if b.is_empty() {
                None
            } else {
                serde_json::from_slice(b).ok()
            }
        })
        .or_else(|| query_input.and_then(|s| serde_json::from_str(s).ok()));
    match raw {
        None => names.into_iter().map(|n| (n, Value::Null)).collect(),
        Some(Value::Object(map))
            if map.contains_key("0") || map.keys().any(|k| k.parse::<usize>().is_ok()) =>
        {
            names
                .into_iter()
                .enumerate()
                .map(|(i, n)| {
                    let inp = map.get(&i.to_string()).cloned().unwrap_or(Value::Null);
                    (n, unwrap_json(&inp))
                })
                .collect()
        }
        Some(v) => {
            let inp = unwrap_json(&v);
            if names.len() == 1 {
                vec![(names[0].clone(), inp)]
            } else {
                names.into_iter().map(|n| (n, inp.clone())).collect()
            }
        }
    }
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(cookie) = headers.get("cookie").and_then(|h| h.to_str().ok()) {
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some(v) = part.strip_prefix("mk_session=") {
                return Some(v);
            }
        }
    }
    None
}

fn ok_payload(data: Value) -> Value {
    json!({"result": {"data": {"json": data}}})
}

fn err_payload(e: &api::ApiError) -> Value {
    json!({
        "error": {
            "json": {
                "message": e.message,
                "code": match e.http { 401 => -32001, 403 => -32003, 404 => -32004, 409 => -32009, _ => -32603 },
                "data": { "code": e.code, "httpStatus": e.http }
            }
        }
    })
}

async fn trpc(
    State(state): State<Arc<AppState>>,
    Path(procedures): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let calls = parse_calls(&procedures, q.get("input").map(|s| s.as_str()), Some(&body));
    let has_mutation = calls
        .iter()
        .any(|(procedure, _)| api::is_mutation(procedure));
    if method == Method::GET && has_mutation {
        return (StatusCode::METHOD_NOT_ALLOWED, "mutations require POST").into_response();
    }
    if method != Method::GET && has_mutation {
        let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
            return (
                StatusCode::FORBIDDEN,
                "mutation requires same-origin Origin header",
            )
                .into_response();
        };
        let host = headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let allowed = origin == format!("http://{host}") || origin == format!("https://{host}");
        if !allowed {
            return (StatusCode::FORBIDDEN, "cross-site request rejected").into_response();
        }
    }
    let token = session_token(&headers).map(str::to_owned);
    let batched = calls.len() > 1 || q.get("batch").map(|s| s.as_str()) == Some("1");
    let mut conn = state.db.lock();
    let uid = auth::resolve_session(&conn, token.as_deref());
    let signed_call_count = calls
        .iter()
        .filter(|(procedure, _)| device::requires_signature(procedure))
        .count();
    // pending_device_proofs хранит одно доказательство на пользователя. Если
    // разрешить batch, первый ledger::append заберёт proof, а последующие
    // события останутся только node-signed. Один HTTP-запрос = одна
    // пользовательская транзакция сохраняет точную и проверяемую связь.
    if signed_call_count > 0 && calls.len() != 1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(err_payload(&api::ApiError::new(
                "SIGNED_BATCH_FORBIDDEN",
                400,
                "Подписываемые операции должны отправляться отдельными запросами",
            ))),
        )
            .into_response();
    }
    if signed_call_count == 1 {
        let verified = uid.ok_or("Войдите в систему").and_then(|user_id| {
            let proof = device::verify_request(
                &conn,
                user_id,
                &format!("/api/trpc/{procedures}"),
                &body,
                &headers,
            )
            .map_err(|_| "Требуется действительная подпись устройства")?;
            device::set_pending(&conn, user_id, &proof)
                .map_err(|_| "Не удалось привязать подпись к операции")
        });
        if let Err(message) = verified {
            return (
                StatusCode::FORBIDDEN,
                Json(err_payload(&api::ApiError::new(
                    "DEVICE_SIGNATURE_REQUIRED",
                    403,
                    message,
                ))),
            )
                .into_response();
        }
    }
    let mut out = Vec::new();
    let mut set_session: Option<Option<String>> = None;
    for (proc, input) in &calls {
        match api::dispatch(&mut conn, proc, input, uid) {
            Ok(data) => {
                if proc == "auth.login" || proc == "auth.register" || proc == "auth.joinRegister" {
                    if let Some(id) = data.get("id").and_then(|v| v.as_i64()) {
                        match auth::create_session(&conn, id) {
                            Ok(new_token) => set_session = Some(Some(new_token)),
                            Err(e) => {
                                out.push(err_payload(&api::ApiError::internal(format!(
                                    "Не удалось создать сессию: {e}"
                                ))));
                                continue;
                            }
                        }
                    }
                }
                if proc == "auth.logout" {
                    let _ = auth::revoke_session(&conn, token.as_deref());
                    set_session = Some(None);
                }
                out.push(ok_payload(data));
            }
            Err(e) => out.push(err_payload(&e)),
        }
    }
    device::clear_pending(&conn, uid);
    let body = if batched || out.len() != 1 {
        Value::Array(out)
    } else {
        out.pop().unwrap_or(json!({}))
    };
    let mut builder = axum::http::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json; charset=utf-8");
    if let Some(session) = set_session {
        let secure = std::env::var("MESHKEEPER_COOKIE_SECURE").as_deref() == Ok("1");
        let cookie = match session {
            Some(token) => format!(
                "mk_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000{}",
                if secure { "; Secure" } else { "" }
            ),
            None => format!(
                "mk_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
                if secure { "; Secure" } else { "" }
            ),
        };
        builder = builder.header("set-cookie", cookie);
    }
    builder.body(body.to_string()).unwrap().into_response()
}

async fn spa_index(State(index): State<Arc<PathBuf>>, uri: Uri) -> impl IntoResponse {
    // Отсутствующий ассет должен оставаться 404, иначе сломанный бандл
    // возвращает HTML вместо скрипта и ошибка становится незаметной.
    let path = uri.path();
    let looks_like_file = path
        .rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'));
    if path.starts_with("/assets/") || looks_like_file {
        return (StatusCode::NOT_FOUND, "Файл не найден").into_response();
    }
    match tokio::fs::read(index.as_path()).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            "UI не собран. Выполните: npm run build",
        )
            .into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "node": "meshkeeper-node",
        "journal": "signed-account-chains",
        "role": node_role(),
        "sync": if sync_capabilities().is_ok_and(|values| !values.is_empty()) { "enabled" } else { "disabled" },
    }))
}

/// Роль узла определяется конфигурацией, отдельного переключателя не нужно:
/// есть upstream — это локальный узел, нет upstream, но есть токен — сервер.
fn node_role() -> &'static str {
    let capabilities = sync_capabilities().unwrap_or_default();
    if capabilities
        .iter()
        .any(|capability| !capability.peers.is_empty())
    {
        "node"
    } else if !capabilities.is_empty() {
        "server"
    } else {
        "standalone"
    }
}

fn upstream_url() -> Option<String> {
    std::env::var("MESHKEEPER_UPSTREAM")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
}

pub(crate) fn validate_peer_url(upstream: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(upstream).map_err(|_| "Некорректный адрес peer")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(
            "Адрес peer не должен содержать путь, логин, пароль, query или fragment".into(),
        );
    }
    if url.scheme() == "https" {
        return Ok(());
    }
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    let explicitly_allowed = std::env::var("MESHKEEPER_ALLOW_INSECURE_SYNC").as_deref() == Ok("1");
    if url.scheme() == "http" && (loopback || explicitly_allowed) {
        return Ok(());
    }
    Err("Peer требует HTTPS; для изолированной доверенной LAN задайте MESHKEEPER_ALLOW_INSECURE_SYNC=1".into())
}

/// Общий секрет сервера и локальных узлов. Не задан — обмен выключен.
pub(crate) fn sync_token() -> Option<String> {
    std::env::var("MESHKEEPER_SYNC_TOKEN")
        .ok()
        .filter(|t| t.chars().count() >= 32)
}

/// Optional organization allowlist bound to the mesh capability deployed on
/// this node. GUIDs are used because local numeric IDs differ between peers.
pub(crate) fn sync_workspace_scope() -> Option<HashSet<String>> {
    let raw = std::env::var("MESHKEEPER_SYNC_WORKSPACES").ok()?;
    let values: HashSet<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    // Explicitly empty is fail-closed. Only an absent variable enables the
    // legacy trusted-infrastructure scope containing every organization.
    Some(values)
}

#[derive(Clone, Debug)]
struct SyncCapability {
    token: String,
    workspace_scope: Option<HashSet<String>>,
    peers: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncCapabilityConfig {
    token: String,
    #[serde(default)]
    workspaces: Vec<String>,
    #[serde(default)]
    peers: Vec<String>,
}

/// Multiple isolated organization capabilities. When the JSON variable is
/// present, malformed/empty configuration is fail-closed and never falls back
/// to the legacy all-database token.
fn parse_sync_capabilities(raw: &str) -> Result<Vec<SyncCapability>, String> {
    let entries: Vec<SyncCapabilityConfig> = serde_json::from_str(raw)
        .map_err(|error| format!("MESHKEEPER_SYNC_CAPABILITIES: {error}"))?;
    if entries.is_empty() {
        return Err("MESHKEEPER_SYNC_CAPABILITIES не должен быть пустым".into());
    }
    let mut tokens = HashSet::new();
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.token.chars().count() < 32 || entry.token.chars().count() > 256 {
            return Err("каждый capability token должен содержать 32–256 символов".into());
        }
        if !tokens.insert(entry.token.clone()) {
            return Err("capability token повторяется".into());
        }
        if entry.workspaces.is_empty() || entry.workspaces.len() > 100 {
            return Err("каждая capability должна разрешать 1–100 организаций".into());
        }
        let workspace_scope: HashSet<String> = entry
            .workspaces
            .into_iter()
            .map(|value| value.trim().to_owned())
            .collect();
        if workspace_scope
            .iter()
            .any(|guid| guid.is_empty() || guid.len() > 128)
        {
            return Err("некорректный workspace GUID в capability".into());
        }
        if entry.peers.len() > sync::MAX_PEERS as usize {
            return Err(format!(
                "capability содержит более {} peers",
                sync::MAX_PEERS
            ));
        }
        let mut peers = Vec::new();
        for peer in entry.peers {
            let peer = peer.trim().trim_end_matches('/').to_owned();
            validate_peer_url(&peer).map_err(|error| format!("capability peer: {error}"))?;
            if !peers.contains(&peer) {
                peers.push(peer);
            }
        }
        out.push(SyncCapability {
            token: entry.token,
            workspace_scope: Some(workspace_scope),
            peers,
        });
    }
    Ok(out)
}

fn sync_capabilities() -> Result<Vec<SyncCapability>, String> {
    if let Ok(raw) = std::env::var("MESHKEEPER_SYNC_CAPABILITIES") {
        return parse_sync_capabilities(&raw);
    }
    let mut peers = Vec::new();
    if let Some(peer) = upstream_url() {
        validate_peer_url(&peer)?;
        peers.push(peer);
    }
    Ok(sync_token()
        .map(|token| SyncCapability {
            token,
            workspace_scope: sync_workspace_scope(),
            peers,
        })
        .into_iter()
        .collect())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

fn authorized_capability(headers: &HeaderMap) -> Option<SyncCapability> {
    let got = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    find_bearer_capability(sync_capabilities().ok()?, got)
}

fn find_bearer_capability(capabilities: Vec<SyncCapability>, got: &str) -> Option<SyncCapability> {
    capabilities.into_iter().find(|capability| {
        constant_time_eq(
            got.as_bytes(),
            format!("Bearer {}", capability.token).as_bytes(),
        )
    })
}

pub(crate) fn sync_capability_summary() -> (String, Vec<String>, usize) {
    let capabilities = sync_capabilities().unwrap_or_default();
    if capabilities.is_empty() {
        return ("disabled".into(), Vec::new(), 0);
    }
    if capabilities.len() == 1 && capabilities[0].workspace_scope.is_none() {
        return ("all".into(), Vec::new(), 1);
    }
    let mut workspaces: Vec<String> = capabilities
        .iter()
        .flat_map(|capability| {
            capability
                .workspace_scope
                .iter()
                .flat_map(|scope| scope.iter().cloned())
        })
        .collect();
    workspaces.sort();
    workspaces.dedup();
    (
        if capabilities.len() > 1 {
            "capabilities"
        } else {
            "restricted"
        }
        .into(),
        workspaces,
        capabilities.len(),
    )
}

pub(crate) fn sync_bundle_export_capability(
    workspace_guid: Option<&str>,
) -> Result<(String, Option<HashSet<String>>), String> {
    let capabilities = sync_capabilities()?;
    if capabilities.len() == 1 {
        let capability = capabilities.into_iter().next().unwrap();
        if let (Some(guid), Some(scope)) = (workspace_guid, capability.workspace_scope.as_ref()) {
            if !scope.contains(guid) {
                return Err("capability не разрешает выбранную организацию".into());
            }
        }
        return Ok((capability.token, capability.workspace_scope));
    }
    let guid =
        workspace_guid.ok_or_else(|| "Выберите организацию для offline bundle".to_string())?;
    let mut matching = capabilities.into_iter().filter(|capability| {
        capability
            .workspace_scope
            .as_ref()
            .is_some_and(|scope| scope.contains(guid))
    });
    let capability = matching
        .next()
        .ok_or_else(|| "Для организации не настроена capability".to_string())?;
    if matching.next().is_some() {
        return Err("Для организации настроено несколько capability; выбор неоднозначен".into());
    }
    Ok((capability.token, capability.workspace_scope))
}

pub(crate) fn sync_bundle_import_capabilities() -> Vec<(String, Option<HashSet<String>>)> {
    sync_capabilities()
        .unwrap_or_default()
        .into_iter()
        .map(|capability| (capability.token, capability.workspace_scope))
        .collect()
}

fn sync_authorized(headers: &HeaderMap) -> bool {
    authorized_capability(headers).is_some()
}

fn blob_capability(secret: &str, hash: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(b"everyday/cas-capability/v1\0");
    mac.update(hash.as_bytes());
    format!("cas1:{hash}:{}", hex::encode(mac.finalize().into_bytes()))
}

fn blob_authorized(headers: &HeaderMap, hash: &str) -> Option<SyncCapability> {
    if let Some(capability) = authorized_capability(headers) {
        return Some(capability);
    }
    let got = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    sync_capabilities().ok()?.into_iter().find(|capability| {
        constant_time_eq(
            got.as_bytes(),
            format!("Bearer {}", blob_capability(&capability.token, hash)).as_bytes(),
        )
    })
}

async fn sync_hello(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if !sync_authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"синхронизация выключена или неверный токен"})),
        )
            .into_response();
    }
    let db = state.db.lock();
    Json(sync::hello(&db)).into_response()
}

pub(crate) fn interorg_work_bits() -> u8 {
    std::env::var("MESHKEEPER_INTERORG_POW_BITS")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|bits| (8..=24).contains(bits))
        .unwrap_or(18)
}

/// Public mesh ingress for opaque cross-organization envelopes. It has no
/// organization capability by design: relays validate signature/PoW/TTL and
/// retain only bounded ciphertext, while organization data stays encrypted.
async fn interorg_envelope_post(
    State(state): State<Arc<AppState>>,
    Json(envelope): Json<interorg::Envelope>,
) -> impl IntoResponse {
    let db = state.db.lock();
    match interorg::relay_store(&db, &envelope, interorg_work_bits()) {
        Ok(inserted) => {
            let delivered = interorg::receive_local(&db, interorg_work_bits()).unwrap_or(0);
            (
                StatusCode::OK,
                Json(json!({"ok":true,"inserted":inserted,"delivered":delivered})),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn interorg_envelopes_get(
    State(state): State<Arc<AppState>>,
    Path(destination): Path<String>,
) -> impl IntoResponse {
    let db = state.db.lock();
    match interorg::pending_for(&db, &destination, 128) {
        Ok(envelopes) => Json(json!({"envelopes":envelopes})).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn interorg_gossip_get(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db = state.db.lock();
    match interorg::gossip_batch(&db, 128) {
        Ok(envelopes) => Json(json!({"envelopes":envelopes})).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn interorg_relay_loop(state: Arc<AppState>, peers: Vec<String>) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Interorg relay client: {error}");
            return;
        }
    };
    loop {
        let outgoing = {
            let db = state.db.lock();
            interorg::gossip_batch(&db, 128).unwrap_or_default()
        };
        for peer in &peers {
            if let Ok(response) = client.get(format!("{peer}/mesh/gossip")).send().await {
                if let Ok(body) = response.json::<Value>().await {
                    if let Some(envelopes) = body.get("envelopes").and_then(Value::as_array) {
                        let db = state.db.lock();
                        for raw in envelopes {
                            if let Ok(envelope) =
                                serde_json::from_value::<interorg::Envelope>(raw.clone())
                            {
                                let _ = interorg::relay_store(&db, &envelope, interorg_work_bits());
                            }
                        }
                        let _ = interorg::receive_local(&db, interorg_work_bits());
                    }
                }
            }
            for envelope in &outgoing {
                let _ = client
                    .post(format!("{peer}/mesh/envelopes"))
                    .json(envelope)
                    .send()
                    .await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn sync_journal_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(capability) = authorized_capability(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"sync disabled"})),
        )
            .into_response();
    };
    let db = state.db.lock();
    Json(sync::export_journal_scoped(
        &db,
        None,
        capability.workspace_scope.as_ref(),
    ))
    .into_response()
}

async fn sync_journal_post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let Some(capability) = authorized_capability(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"sync disabled"})),
        )
            .into_response();
    };
    let db = state.db.lock();
    if capability
        .workspace_scope
        .as_ref()
        .is_some_and(|scope| !sync::journal_within_scope(&body, scope))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"ok":false,"error":"журнал содержит организацию вне capability scope"})),
        )
            .into_response();
    }
    let from = body
        .get("nodeUrl")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    Json(sync::apply_remote_journal(&db, &body, from)).into_response()
}

async fn sync_journal_pull(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let Some(capability) = authorized_capability(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"sync disabled"})),
        )
            .into_response();
    };
    let requested = body.get("frontier").unwrap_or(&Value::Null);
    let db = state.db.lock();
    Json(sync::export_journal_scoped(
        &db,
        Some(requested),
        capability.workspace_scope.as_ref(),
    ))
    .into_response()
}

#[derive(Deserialize)]
struct BlobQuery {
    offset: Option<usize>,
}

async fn sync_blob_get(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
    Query(query): Query<BlobQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(capability) = blob_authorized(&headers, &hash) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"sync disabled"})),
        )
            .into_response();
    };
    let db = state.db.lock();
    if capability
        .workspace_scope
        .as_ref()
        .is_some_and(|scope| !sync::content_hash_allowed(&db, scope, &hash))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"CAS-объект вне organization scope"})),
        )
            .into_response();
    }
    match content::chunk(&db, &hash, query.offset.unwrap_or(0)) {
        Ok(chunk) => Json(chunk).into_response(),
        Err(error) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn sync_missing_blobs(
    client: &reqwest::Client,
    state: &Arc<AppState>,
    upstream: &str,
    token: &str,
    journal: &Value,
) -> anyhow::Result<()> {
    let missing = {
        let db = state.db.lock();
        content::wanted_missing(&db, journal)
    };
    let manifest = journal
        .get("contentCatalog")
        .or_else(|| journal.get("blobs"))
        .unwrap_or(&Value::Null);
    for hash in missing {
        let expected = manifest
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry.get("hash").and_then(Value::as_str) == Some(hash.as_str()))
            .ok_or_else(|| anyhow::anyhow!("blob отсутствует в подписанном manifest"))?;
        let expected_mime = expected.get("mime").and_then(Value::as_str);
        let expected_size = expected.get("size").and_then(Value::as_u64);
        for _ in 0..=512 {
            let offset = {
                let db = state.db.lock();
                content::download_offset(&db, &hash)
            };
            let providers = {
                let db = state.db.lock();
                content::providers(&db, &hash, upstream)
            };
            let mut response = None;
            for provider in providers {
                if let Ok(candidate) = client
                    .get(format!("{provider}/sync/blob/{hash}?offset={offset}"))
                    .bearer_auth(blob_capability(token, &hash))
                    .send()
                    .await
                {
                    if candidate.status().is_success() {
                        response = Some(candidate);
                        break;
                    }
                }
            }
            let response = response
                .ok_or_else(|| anyhow::anyhow!("ни один CAS-провайдер не доступен для {hash}"))?;
            let response_bytes = response.bytes().await?;
            let payload: Value = serde_json::from_slice(&response_bytes)?;
            if payload.get("mime").and_then(Value::as_str) != expected_mime
                || payload.get("totalSize").and_then(Value::as_u64) != expected_size
            {
                anyhow::bail!("chunk не соответствует подписанному manifest");
            }
            let complete = {
                let db = state.db.lock();
                sync::metric_add(&db, "sync_bytes_received", response_bytes.len() as u64);
                content::accept_chunk(&db, &payload)?
            };
            if complete {
                break;
            }
        }
        let still_missing = {
            let db = state.db.lock();
            content::missing(&db, &json!([{"hash":hash}]))
        };
        if !still_missing.is_empty() {
            anyhow::bail!("blob не завершён после максимального числа chunks");
        }
    }
    Ok(())
}

/// Локальный узел обменивается изменениями с центральным сервером.
///
/// Работает офлайн-first: если сервер недоступен, узел продолжает работать на
/// своей базе, ошибка попадает в «Админка → Офлайн-узлы», а следующая попытка
/// произойдёт на следующем тике.
async fn peer_loop(
    state: Arc<AppState>,
    capability: SyncCapability,
    include_discovered_peers: bool,
) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Синхронизация не запущена: {e}");
            return;
        }
    };
    let interval = std::env::var("MESHKEEPER_SYNC_INTERVAL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(15)
        .clamp(5, 3600);
    eprintln!("P2P-синхронизация каждые {interval} с");
    let mut waited = interval; // первый проход — сразу после старта
    loop {
        let asked_now = sync::take_sync_request();
        if asked_now || waited >= interval {
            let mut peers = {
                if include_discovered_peers {
                    let db = state.db.lock();
                    sync::peer_urls(&db)
                } else {
                    Vec::new()
                }
            };
            peers.extend(capability.peers.iter().cloned());
            peers.sort();
            peers.dedup();
            let local_urls = [sync::local_http_base(), sync::guess_lan_base()];
            peers.retain(|peer| !local_urls.iter().any(|local| local == peer));
            for peer in peers {
                sync_once(&client, &state, &peer, &capability).await;
            }
            waited = 0;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        waited += 1;
    }
}

async fn sync_once(
    client: &reqwest::Client,
    state: &Arc<AppState>,
    upstream: &str,
    capability: &SyncCapability,
) {
    let token = &capability.token;
    {
        let db = state.db.lock();
        sync::ensure_node(&db);
        sync::add_peer(&db, upstream, None, None);
    }

    // 1. Забираем изменения сервера.
    let local_frontier = {
        let db = state.db.lock();
        sync::frontier(&db)
    };
    let mut used_frontier_protocol = true;
    let mut pulled = client
        .post(format!("{upstream}/sync/journal/pull"))
        .bearer_auth(token)
        .json(&json!({"frontier": local_frontier}))
        .send()
        .await;
    // Совместимость при поэтапном обновлении: старый peer не знает pull-route.
    if matches!(&pulled, Ok(response) if response.status() == StatusCode::NOT_FOUND) {
        used_frontier_protocol = false;
        pulled = client
            .get(format!("{upstream}/sync/journal"))
            .bearer_auth(token)
            .send()
            .await;
    }
    let (remote_frontier, remote_content) = match pulled {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(journal) => {
                    if capability
                        .workspace_scope
                        .as_ref()
                        .is_some_and(|scope| !sync::journal_within_scope(&journal, scope))
                    {
                        let db = state.db.lock();
                        sync::touch_peer_error(
                            &db,
                            upstream,
                            "peer прислал организацию вне capability scope",
                        );
                        return;
                    }
                    let remote_frontier = journal.get("frontier").cloned().unwrap_or(Value::Null);
                    let db = state.db.lock();
                    sync::metric_add(&db, "sync_bytes_received", bytes.len() as u64);
                    let applied = sync::apply_remote_journal(&db, &journal, upstream);
                    if applied.get("ok").and_then(Value::as_bool) == Some(false) {
                        sync::touch_peer_error(
                            &db,
                            upstream,
                            applied
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("журнал peer отклонён"),
                        );
                        return;
                    }
                    (
                        remote_frontier,
                        json!({
                            "blobs": journal.get("blobs").cloned().unwrap_or_else(|| json!([])),
                            "photos": journal.get("photos").cloned().unwrap_or_else(|| json!([])),
                            "contentCatalog": journal.get("contentCatalog").cloned().unwrap_or_else(|| journal.get("blobs").cloned().unwrap_or_else(|| json!([]))),
                            "contentProviders": journal.get("contentProviders").cloned().unwrap_or_else(|| json!([]))
                        }),
                    )
                }
                Err(e) => {
                    let db = state.db.lock();
                    sync::touch_peer_error(&db, upstream, &format!("некорректный JSON: {e}"));
                    return;
                }
            },
            Err(e) => {
                let db = state.db.lock();
                sync::touch_peer_error(&db, upstream, &format!("некорректный ответ: {e}"));
                return;
            }
        },
        Ok(resp) => {
            let status = resp.status();
            let db = state.db.lock();
            sync::touch_peer_error(&db, upstream, &format!("сервер ответил {status}"));
            return;
        }
        Err(e) => {
            let db = state.db.lock();
            sync::touch_peer_error(&db, upstream, &short_net_error(&e));
            return;
        }
    };

    if let Err(error) = sync_missing_blobs(client, state, upstream, token, &remote_content).await {
        let db = state.db.lock();
        sync::touch_peer_error(&db, upstream, &format!("ошибка вложения: {error}"));
        return;
    }

    // 2. Отдаём свои.
    let mine = {
        let db = state.db.lock();
        sync::export_journal_scoped(
            &db,
            (used_frontier_protocol && remote_frontier.is_array()).then_some(&remote_frontier),
            capability.workspace_scope.as_ref(),
        )
    };
    let mine_bytes = match serde_json::to_vec(&mine) {
        Ok(value) => value,
        Err(error) => {
            let db = state.db.lock();
            sync::touch_peer_error(
                &db,
                upstream,
                &format!("не удалось собрать журнал: {error}"),
            );
            return;
        }
    };
    match client
        .post(format!("{upstream}/sync/journal"))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(mine_bytes.clone())
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let answer = resp.json::<Value>().await.unwrap_or_else(
                |error| json!({"ok":false,"error":format!("некорректный ответ peer: {error}")}),
            );
            let db = state.db.lock();
            if answer.get("ok").and_then(Value::as_bool) == Some(false) {
                sync::touch_peer_error(
                    &db,
                    upstream,
                    answer
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("peer отклонил журнал"),
                );
            } else {
                sync::metric_add(&db, "sync_bytes_sent", mine_bytes.len() as u64);
                sync::metric_add(&db, "sync_successes", 1);
                let _ = db.execute(
                    "UPDATE peers SET last_sync=?1, last_error=NULL WHERE url=?2",
                    rusqlite::params![chrono::Utc::now().to_rfc3339(), upstream],
                );
                sync::resolve_peer_error(&db, upstream);
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let db = state.db.lock();
            sync::touch_peer_error(
                &db,
                upstream,
                &format!("сервер отклонил выгрузку: {status}"),
            );
        }
        Err(e) => {
            let db = state.db.lock();
            sync::touch_peer_error(&db, upstream, &short_net_error(&e));
        }
    }
}

fn short_net_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "сервер не ответил вовремя".into()
    } else if e.is_connect() {
        "нет связи с сервером".into()
    } else {
        e.to_string()
    }
}

/// Запускает тот же узел из desktop binary или мобильного JNI bridge.
pub async fn run() -> anyhow::Result<()> {
    let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let db_path = std::env::var("MESHKEEPER_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dir.join("data").join("meshkeeper-rs.db"));
    eprintln!("Узел MeshKeeper, база {}", db_path.display());
    // Понятное сообщение вместо трассировки паники: сюда попадают и обычные
    // ошибки доступа к файлу, и неверный ключ шифрования.
    let conn = db::open(&db_path).map_err(|error| {
        anyhow::anyhow!("Не удалось открыть базу {}: {error}", db_path.display())
    })?;
    {
        let _ = sync::ensure_node(&conn);
        eprintln!(
            "Узел {}, LAN {}",
            sync::kv_get(&conn, "node_name").unwrap_or_default(),
            sync::guess_lan_base()
        );
    }
    let bootstrap_peers: Vec<String> = std::env::var("MESHKEEPER_PEERS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();
    let multi_capability_mode = std::env::var("MESHKEEPER_SYNC_CAPABILITIES").is_ok();
    if multi_capability_mode && (!bootstrap_peers.is_empty() || upstream_url().is_some()) {
        anyhow::bail!("MESHKEEPER_SYNC_CAPABILITIES задаёт peers внутри JSON; MESHKEEPER_PEERS/UPSTREAM неоднозначны");
    }
    if !bootstrap_peers.is_empty() && sync_token().is_none() {
        anyhow::bail!("MESHKEEPER_PEERS требует MESHKEEPER_SYNC_TOKEN не короче 32 символов");
    }
    for peer in bootstrap_peers {
        if let Err(message) = validate_peer_url(&peer) {
            anyhow::bail!("MESHKEEPER_PEERS: {message}");
        }
        let result = sync::add_peer(&conn, &peer, Some("bootstrap"), None);
        if result.get("ok").and_then(Value::as_bool) == Some(false) {
            anyhow::bail!("MESHKEEPER_PEERS: {}", result["error"]);
        }
    }
    let state = Arc::new(AppState {
        db: Mutex::new(conn),
    });
    let relay_peers: Vec<String> = std::env::var("MESHKEEPER_RELAY_PEERS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|peer| !peer.is_empty())
        .map(|peer| peer.trim_end_matches('/').to_owned())
        .collect();
    if relay_peers.len() > sync::MAX_PEERS as usize {
        anyhow::bail!("MESHKEEPER_RELAY_PEERS превышает лимит peers");
    }
    for peer in &relay_peers {
        validate_peer_url(peer).map_err(anyhow::Error::msg)?;
    }
    if !relay_peers.is_empty() {
        tokio::spawn(interorg_relay_loop(state.clone(), relay_peers));
    }
    let capabilities = sync_capabilities().map_err(anyhow::Error::msg)?;
    if capabilities.is_empty() && upstream_url().is_some() {
        anyhow::bail!("MESHKEEPER_UPSTREAM требует MESHKEEPER_SYNC_TOKEN не короче 32 символов");
    }
    if let Ok(url) = std::env::var("MESHKEEPER_ADVERTISE_URL") {
        if let Err(message) = validate_peer_url(url.trim()) {
            anyhow::bail!("MESHKEEPER_ADVERTISE_URL: {message}");
        }
    }
    if capabilities.is_empty() {
        eprintln!("Автономный режим: обмен с сервером выключен");
    } else {
        for capability in capabilities.iter().cloned() {
            for peer in &capability.peers {
                let db = state.db.lock();
                let result = sync::add_peer(&db, peer, Some("capability"), None);
                if result.get("ok").and_then(Value::as_bool) == Some(false) {
                    anyhow::bail!("capability peer: {}", result["error"]);
                }
            }
            tokio::spawn(peer_loop(state.clone(), capability, !multi_capability_mode));
        }
        if capabilities.len() == 1 {
            if let Ok(bind) = std::env::var("MESHKEEPER_DISCOVERY_BIND") {
                let bind = bind.trim().to_owned();
                if !bind.is_empty() {
                    let target = std::env::var("MESHKEEPER_DISCOVERY_TARGET")
                        .unwrap_or_else(|_| "255.255.255.255:8767".into());
                    tokio::spawn(discovery::run(
                        state.clone(),
                        capabilities[0].token.clone(),
                        bind,
                        target,
                    ));
                }
            }
        } else if std::env::var("MESHKEEPER_DISCOVERY_BIND").is_ok() {
            eprintln!("UDP discovery выключен для нескольких capability; используйте их peers");
        }
        eprintln!(
            "Режим mesh: {} capability, обмен через /sync/journal",
            capabilities.len()
        );
    }
    let web_root = std::env::var("MESHKEEPER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dir.join("dist").join("public"));
    // Маршруты SPA (/tool/1, /join?token=…) должны отдавать index.html со
    // статусом 200: ServeFile как not_found_service сохранял 404, из-за чего
    // ссылка-приглашение выглядела как «страница не найдена».
    let static_files = ServeDir::new(&web_root)
        .fallback(any(spa_index).with_state(Arc::new(web_root.join("index.html"))));
    // На телефоне UI/API слушает только loopback, а отдельный LAN listener
    // публикует исключительно token-protected sync/CAS endpoints. Так cookie и
    // пользовательский API не оказываются в незашифрованной локальной сети.
    if let Ok(sync_addr) = std::env::var("MESHKEEPER_SYNC_BIND") {
        let sync_addr = sync_addr.trim().to_string();
        if !sync_addr.is_empty() {
            let sync_app = Router::new()
                .route("/sync/hello", get(sync_hello))
                .route(
                    "/sync/journal",
                    get(sync_journal_get).post(sync_journal_post),
                )
                .route("/sync/journal/pull", post(sync_journal_pull))
                .route("/sync/blob/{hash}", get(sync_blob_get))
                .route("/mesh/envelopes", post(interorg_envelope_post))
                .route("/mesh/envelopes/{destination}", get(interorg_envelopes_get))
                .route("/mesh/gossip", get(interorg_gossip_get))
                .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
                .layer(middleware::from_fn(security_headers))
                .with_state(state.clone());
            let sync_listener = tokio::net::TcpListener::bind(&sync_addr).await?;
            eprintln!("Sync-only listener {sync_addr}");
            tokio::spawn(async move {
                if let Err(error) = axum::serve(sync_listener, sync_app).await {
                    eprintln!("Sync-only listener остановлен: {error}");
                }
            });
        }
    }
    let app = Router::new()
        .route("/health", get(health))
        .route("/sync/hello", get(sync_hello))
        .route(
            "/sync/journal",
            get(sync_journal_get).post(sync_journal_post),
        )
        .route("/sync/journal/pull", post(sync_journal_pull))
        .route("/sync/blob/{hash}", get(sync_blob_get))
        .route("/mesh/envelopes", post(interorg_envelope_post))
        .route("/mesh/envelopes/{destination}", get(interorg_envelopes_get))
        .route("/mesh/gossip", get(interorg_gossip_get))
        .route("/api/trpc/{*procedures}", any(trpc))
        .fallback_service(static_files)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(security_headers))
        .with_state(state);
    let addr = std::env::var("MESHKEEPER_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let loopback =
        addr.starts_with("127.") || addr.starts_with("localhost") || addr.starts_with("[::1]");
    if !loopback && std::env::var("MESHKEEPER_COOKIE_SECURE").as_deref() != Ok("1") {
        anyhow::bail!(
            "non-loopback bind requires MESHKEEPER_COOKIE_SECURE=1 and an HTTPS reverse proxy"
        );
    }
    eprintln!("Слушаю {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    #[test]
    fn parses_multiple_disjoint_capabilities_fail_closed() {
        let token_a = "a".repeat(32);
        let token_b = "b".repeat(32);
        let raw = serde_json::json!([
            {"token":token_a,"workspaces":["org-a"]},
            {"token":token_b,"workspaces":["org-b","org-c"]}
        ])
        .to_string();
        let parsed = parse_sync_capabilities(&raw).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0].workspace_scope.as_ref().unwrap(),
            &HashSet::from(["org-a".to_string()])
        );
        assert!(parsed[1]
            .workspace_scope
            .as_ref()
            .unwrap()
            .contains("org-c"));
        assert!(parsed
            .iter()
            .all(|capability| capability.workspace_scope.is_some()));
        let matched = find_bearer_capability(parsed, &format!("Bearer {token_b}")).unwrap();
        assert_eq!(
            matched.workspace_scope.unwrap(),
            HashSet::from(["org-b".to_string(), "org-c".to_string()])
        );

        assert!(parse_sync_capabilities("[]").is_err());
        assert!(parse_sync_capabilities(
            &serde_json::json!([{"token":"x".repeat(32),"workspaces":[]}]).to_string()
        )
        .is_err());
        assert!(parse_sync_capabilities(
            &serde_json::json!([
                {"token":"x".repeat(32),"workspaces":["a"]},
                {"token":"x".repeat(32),"workspaces":["b"]}
            ])
            .to_string()
        )
        .is_err());
        assert!(parse_sync_capabilities(
            &serde_json::json!([{"token":"x".repeat(32),"workspaces":["a"],"unexpected":true}])
                .to_string()
        )
        .is_err());
    }

    #[test]
    fn capability_token_comparison_is_exact() {
        assert!(constant_time_eq(b"same-token", b"same-token"));
        assert!(!constant_time_eq(b"same-token", b"same-tokee"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }
}

#[cfg(target_os = "android")]
mod android_jni {
    use jni::objects::{JByteArray, JClass, JObject, JObjectArray, JString};
    use jni::sys::{jbyteArray, jint, jobjectArray, jstring};
    use jni::JNIEnv;

    fn string(env: &mut JNIEnv<'_>, value: JString<'_>) -> Result<String, String> {
        env.get_string(&value)
            .map(|value| value.into())
            .map_err(|error| error.to_string())
    }

    fn java_frames(env: &mut JNIEnv<'_>, frames: JObjectArray<'_>) -> Result<Vec<Vec<u8>>, String> {
        let count = env
            .get_array_length(&frames)
            .map_err(|error| error.to_string())?;
        if count < 0 || count as usize > crate::stream_transport::MAX_FRAMES {
            return Err("invalid transport frame array length".into());
        }
        let mut values = Vec::with_capacity(count as usize);
        for index in 0..count {
            let object = env
                .get_object_array_element(&frames, index)
                .map_err(|error| error.to_string())?;
            if object.is_null() {
                return Err("transport frame must not be null".into());
            }
            values.push(
                env.convert_byte_array(JByteArray::from(object))
                    .map_err(|error| error.to_string())?,
            );
        }
        Ok(values)
    }

    fn assemble_frames(
        frames: Vec<Vec<u8>>,
    ) -> Result<(crate::stream_transport::Assembler, Option<Vec<u8>>), String> {
        let mut assembler = crate::stream_transport::Assembler::default();
        let mut completed = None;
        for frame in frames {
            if let Some(value) = assembler
                .accept(&frame)
                .map_err(|error| error.to_string())?
            {
                completed = Some(value.bytes);
            }
        }
        Ok((assembler, completed))
    }

    /// Блокирующий entrypoint вызывается NodeService на выделенном thread.
    #[no_mangle]
    pub extern "system" fn Java_ru_meshkeeper_app_RustNode_startNode(
        mut env: JNIEnv<'_>,
        _class: JClass<'_>,
        db_path: JString<'_>,
        web_root: JString<'_>,
        upstream: JString<'_>,
        sync_token: JString<'_>,
        workspace_scope: JString<'_>,
        sync_capabilities: JString<'_>,
        node_signing_key: JString<'_>,
        advertise_url: JString<'_>,
    ) -> jint {
        let result = (|| -> Result<(), String> {
            let db_path = string(&mut env, db_path)?;
            let web_root = string(&mut env, web_root)?;
            let upstream = string(&mut env, upstream)?;
            let sync_token = string(&mut env, sync_token)?;
            let workspace_scope = string(&mut env, workspace_scope)?;
            let sync_capabilities = string(&mut env, sync_capabilities)?;
            let node_signing_key = string(&mut env, node_signing_key)?;
            let advertise_url = string(&mut env, advertise_url)?;
            std::env::set_var("MESHKEEPER_DB", db_path);
            std::env::set_var("MESHKEEPER_WEB_ROOT", web_root);
            std::env::set_var("MESHKEEPER_BIND", "127.0.0.1:8765");
            std::env::set_var("MESHKEEPER_SYNC_BIND", "0.0.0.0:8766");
            std::env::set_var("MESHKEEPER_ALLOW_INSECURE_SYNC", "1");
            std::env::set_var("MESHKEEPER_CONTENT_MODE", "smart");
            std::env::set_var("MESHKEEPER_DISCOVERY_BIND", "0.0.0.0:8767");
            std::env::set_var("MESHKEEPER_DISCOVERY_TARGET", "255.255.255.255:8767");
            if upstream.is_empty() {
                std::env::remove_var("MESHKEEPER_UPSTREAM");
            } else {
                std::env::set_var("MESHKEEPER_UPSTREAM", upstream);
            }
            if sync_token.is_empty() {
                std::env::remove_var("MESHKEEPER_SYNC_TOKEN");
            } else {
                std::env::set_var("MESHKEEPER_SYNC_TOKEN", sync_token);
            }
            if sync_capabilities.is_empty() {
                std::env::remove_var("MESHKEEPER_SYNC_CAPABILITIES");
                if workspace_scope.is_empty() {
                    std::env::remove_var("MESHKEEPER_SYNC_WORKSPACES");
                } else {
                    std::env::set_var("MESHKEEPER_SYNC_WORKSPACES", workspace_scope);
                }
            } else {
                crate::parse_sync_capabilities(&sync_capabilities)?;
                std::env::set_var("MESHKEEPER_SYNC_CAPABILITIES", sync_capabilities);
                std::env::remove_var("MESHKEEPER_SYNC_WORKSPACES");
                std::env::remove_var("MESHKEEPER_UPSTREAM");
                std::env::remove_var("MESHKEEPER_SYNC_TOKEN");
            }
            if node_signing_key.is_empty() {
                return Err("Android node signing key is empty".into());
            }
            std::env::set_var("MESHKEEPER_NODE_SIGNING_KEY", node_signing_key);
            if advertise_url.is_empty() {
                std::env::remove_var("MESHKEEPER_ADVERTISE_URL");
            } else {
                std::env::set_var("MESHKEEPER_ADVERTISE_URL", advertise_url);
            }
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime
                .block_on(crate::run())
                .map_err(|error| format!("{error:#}"))
        })();
        match result {
            Ok(()) => 0,
            Err(message) => {
                let _ = env.throw_new("java/lang/IllegalStateException", message);
                -1
            }
        }
    }

    /// Crash-safe first half of the SQLite → AndroidKeyStore migration. This
    /// only reads/creates the seed; startNode validates it and then removes the
    /// plaintext SQLite copy through ledger::signing_key.
    #[no_mangle]
    pub extern "system" fn Java_ru_meshkeeper_app_RustNode_provisionNodeKey(
        mut env: JNIEnv<'_>,
        _class: JClass<'_>,
        db_path: JString<'_>,
    ) -> jstring {
        let result = (|| -> Result<String, String> {
            let path = string(&mut env, db_path)?;
            let conn = crate::db::open(std::path::Path::new(&path)).map_err(|error| {
                format!("Не удалось открыть базу для миграции ключа: {error:#}")
            })?;
            crate::ledger::provision_node_signing_key(&conn)
                .map_err(|error| format!("Не удалось получить ключ подписи ноды: {error:#}"))
        })();
        match result {
            Ok(value) => match env.new_string(value) {
                Ok(value) => value.into_raw(),
                Err(error) => {
                    let _ = env.throw_new("java/lang/IllegalStateException", error.to_string());
                    std::ptr::null_mut()
                }
            },
            Err(message) => {
                let _ = env.throw_new("java/lang/IllegalStateException", message);
                std::ptr::null_mut()
            }
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_ru_meshkeeper_app_RustNode_updateAdvertiseUrl(
        mut env: JNIEnv<'_>,
        _class: JClass<'_>,
        advertise_url: JString<'_>,
    ) {
        let Ok(advertise_url) = string(&mut env, advertise_url) else {
            return;
        };
        if crate::validate_peer_url(advertise_url.trim()).is_ok() {
            std::env::set_var(
                "MESHKEEPER_ADVERTISE_URL",
                advertise_url.trim().trim_end_matches('/'),
            );
            crate::sync::request_sync_now();
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_ru_meshkeeper_app_RustNode_fragmentTransport(
        mut env: JNIEnv<'_>,
        _class: JClass<'_>,
        payload: JByteArray<'_>,
        mtu: jint,
        kind: jint,
    ) -> jobjectArray {
        let result = (|| -> Result<_, String> {
            let payload = env
                .convert_byte_array(payload)
                .map_err(|error| error.to_string())?;
            let kind = crate::stream_transport::PayloadKind::try_from(kind as u8)
                .map_err(|error| error.to_string())?;
            let frames = crate::stream_transport::fragment(kind, &payload, mtu as usize)
                .map_err(|error| error.to_string())?;
            let array = env
                .new_object_array(frames.len() as i32, "[B", JObject::null())
                .map_err(|error| error.to_string())?;
            for (index, frame) in frames.iter().enumerate() {
                let bytes = env
                    .byte_array_from_slice(frame)
                    .map_err(|error| error.to_string())?;
                env.set_object_array_element(&array, index as i32, bytes)
                    .map_err(|error| error.to_string())?;
            }
            Ok(array.into_raw())
        })();
        match result {
            Ok(array) => array,
            Err(message) => {
                let _ = env.throw_new("java/lang/IllegalArgumentException", message);
                std::ptr::null_mut()
            }
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_ru_meshkeeper_app_RustNode_validateTransportFrame(
        mut env: JNIEnv<'_>,
        _class: JClass<'_>,
        frame: JByteArray<'_>,
    ) {
        let result = env
            .convert_byte_array(frame)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                crate::stream_transport::validate_frame(&bytes).map_err(|e| e.to_string())
            });
        if let Err(message) = result {
            let _ = env.throw_new("java/lang/IllegalArgumentException", message);
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_ru_meshkeeper_app_RustNode_missingTransportRanges(
        mut env: JNIEnv<'_>,
        _class: JClass<'_>,
        frames: JObjectArray<'_>,
    ) -> jstring {
        let result = java_frames(&mut env, frames)
            .and_then(assemble_frames)
            .and_then(|(assembler, _)| {
                serde_json::to_string(&assembler.missing_ranges()).map_err(|e| e.to_string())
            });
        match result.and_then(|value| env.new_string(value).map_err(|error| error.to_string())) {
            Ok(value) => value.into_raw(),
            Err(message) => {
                let _ = env.throw_new("java/lang/IllegalArgumentException", message);
                std::ptr::null_mut()
            }
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_ru_meshkeeper_app_RustNode_assembleTransport(
        mut env: JNIEnv<'_>,
        _class: JClass<'_>,
        frames: JObjectArray<'_>,
    ) -> jbyteArray {
        let result = java_frames(&mut env, frames)
            .and_then(assemble_frames)
            .and_then(|(assembler, completed)| {
                completed.ok_or_else(|| {
                    format!(
                        "transport frames are missing: {:?}",
                        assembler.missing_ranges()
                    )
                })
            })
            .and_then(|bytes| {
                env.byte_array_from_slice(&bytes)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(value) => value.into_raw(),
            Err(message) => {
                let _ = env.throw_new("java/lang/IllegalArgumentException", message);
                std::ptr::null_mut()
            }
        }
    }
}
