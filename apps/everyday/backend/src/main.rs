mod accounting;
mod api;
mod auth;
mod content;
mod db;
mod device;
mod json;
mod ledger;
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
use std::{collections::HashMap, path::PathBuf, sync::Arc};
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
    if calls
        .iter()
        .any(|(procedure, _)| device::requires_signature(procedure))
    {
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
        "sync": if sync_token().is_some() { "enabled" } else { "disabled" },
    }))
}

/// Роль узла определяется конфигурацией, отдельного переключателя не нужно:
/// есть upstream — это локальный узел, нет upstream, но есть токен — сервер.
fn node_role() -> &'static str {
    match (upstream_url(), sync_token()) {
        (Some(_), _) => "node",
        (None, Some(_)) => "server",
        (None, None) => "standalone",
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

fn sync_authorized(headers: &HeaderMap) -> bool {
    let Some(secret) = sync_token() else {
        return false;
    };
    let expected = format!("Bearer {secret}");
    let got = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    // Постоянное по времени сравнение: длина токена не секрет, содержимое — да.
    got.len() == expected.len()
        && got
            .as_bytes()
            .iter()
            .zip(expected.as_bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn blob_capability(secret: &str, hash: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(b"everyday/cas-capability/v1\0");
    mac.update(hash.as_bytes());
    format!("cas1:{hash}:{}", hex::encode(mac.finalize().into_bytes()))
}

fn blob_authorized(headers: &HeaderMap, hash: &str) -> bool {
    if sync_authorized(headers) {
        return true;
    }
    let Some(secret) = sync_token() else {
        return false;
    };
    let expected = format!("Bearer {}", blob_capability(&secret, hash));
    let got = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    got.len() == expected.len()
        && got
            .as_bytes()
            .iter()
            .zip(expected.as_bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
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

async fn sync_journal_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !sync_authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"sync disabled"})),
        )
            .into_response();
    }
    let db = state.db.lock();
    Json(sync::export_journal(&db)).into_response()
}

async fn sync_journal_post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if !sync_authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"sync disabled"})),
        )
            .into_response();
    }
    let db = state.db.lock();
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
    if !sync_authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"sync disabled"})),
        )
            .into_response();
    }
    let requested = body.get("frontier").unwrap_or(&Value::Null);
    let db = state.db.lock();
    Json(sync::export_journal_since(&db, Some(requested))).into_response()
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
    if !blob_authorized(&headers, &hash) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"sync disabled"})),
        )
            .into_response();
    }
    let db = state.db.lock();
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
async fn peer_loop(state: Arc<AppState>, upstream: Option<String>, token: String) {
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
                let db = state.db.lock();
                sync::peer_urls(&db)
            };
            if let Some(url) = upstream.as_ref() {
                peers.push(url.clone());
            }
            peers.sort();
            peers.dedup();
            let local_urls = [sync::local_http_base(), sync::guess_lan_base()];
            peers.retain(|peer| !local_urls.iter().any(|local| local == peer));
            for peer in peers {
                sync_once(&client, &state, &peer, &token).await;
            }
            waited = 0;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        waited += 1;
    }
}

async fn sync_once(client: &reqwest::Client, state: &Arc<AppState>, upstream: &str, token: &str) {
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
        if used_frontier_protocol && remote_frontier.is_array() {
            sync::export_journal_since(&db, Some(&remote_frontier))
        } else {
            sync::export_journal(&db)
        }
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

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let db_path = std::env::var("MESHKEEPER_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dir.join("data").join("meshkeeper-rs.db"));
    eprintln!("Узел MeshKeeper, база {}", db_path.display());
    // Понятное сообщение вместо трассировки паники: сюда попадают и обычные
    // ошибки доступа к файлу, и неверный ключ шифрования.
    let conn = match db::open(&db_path) {
        Ok(conn) => conn,
        Err(e) => {
            eprintln!("Не удалось открыть базу {}: {e}", db_path.display());
            std::process::exit(1);
        }
    };
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
    if !bootstrap_peers.is_empty() && sync_token().is_none() {
        panic!("MESHKEEPER_PEERS требует MESHKEEPER_SYNC_TOKEN не короче 32 символов");
    }
    for peer in bootstrap_peers {
        if let Err(message) = validate_peer_url(&peer) {
            panic!("MESHKEEPER_PEERS: {message}");
        }
        let result = sync::add_peer(&conn, &peer, Some("bootstrap"), None);
        if result.get("ok").and_then(Value::as_bool) == Some(false) {
            panic!("MESHKEEPER_PEERS: {}", result["error"]);
        }
    }
    let state = Arc::new(AppState {
        db: Mutex::new(conn),
    });
    let upstream = upstream_url();
    if let Some(url) = upstream.as_deref() {
        if let Err(message) = validate_peer_url(url) {
            panic!("{message}");
        }
    }
    if let Ok(url) = std::env::var("MESHKEEPER_ADVERTISE_URL") {
        if let Err(message) = validate_peer_url(url.trim()) {
            panic!("MESHKEEPER_ADVERTISE_URL: {message}");
        }
    }
    match (upstream, sync_token()) {
        (upstream, Some(token)) => {
            tokio::spawn(peer_loop(state.clone(), upstream, token));
            eprintln!("Режим mesh: принимаю и инициирую обмен через /sync/journal");
        }
        (Some(_), None) => {
            panic!("MESHKEEPER_UPSTREAM требует MESHKEEPER_SYNC_TOKEN не короче 32 символов")
        }
        (None, None) => eprintln!("Автономный режим: обмен с сервером выключен"),
    }
    let web_root = std::env::var("MESHKEEPER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dir.join("dist").join("public"));
    // Маршруты SPA (/tool/1, /join?token=…) должны отдавать index.html со
    // статусом 200: ServeFile как not_found_service сохранял 404, из-за чего
    // ссылка-приглашение выглядела как «страница не найдена».
    let static_files = ServeDir::new(&web_root)
        .fallback(any(spa_index).with_state(Arc::new(web_root.join("index.html"))));
    let app = Router::new()
        .route("/health", get(health))
        .route("/sync/hello", get(sync_hello))
        .route(
            "/sync/journal",
            get(sync_journal_get).post(sync_journal_post),
        )
        .route("/sync/journal/pull", post(sync_journal_pull))
        .route("/sync/blob/{hash}", get(sync_blob_get))
        .route("/api/trpc/{*procedures}", any(trpc))
        .fallback_service(static_files)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(security_headers))
        .with_state(state);
    let addr = std::env::var("MESHKEEPER_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let loopback =
        addr.starts_with("127.") || addr.starts_with("localhost") || addr.starts_with("[::1]");
    if !loopback && std::env::var("MESHKEEPER_COOKIE_SECURE").as_deref() != Ok("1") {
        panic!("non-loopback bind requires MESHKEEPER_COOKIE_SECURE=1 and an HTTPS reverse proxy");
    }
    eprintln!("Слушаю {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}
