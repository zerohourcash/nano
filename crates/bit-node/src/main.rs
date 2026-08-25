mod store;

use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bit_core::{Block, CommunityId, Identity, Ledger, Operation, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

struct App {
    ledger: Ledger,
    store: store::Store,
    identity: Identity,
    token: String,
}
type Shared = Arc<Mutex<App>>;
#[derive(Serialize)]
struct Health {
    status: &'static str,
    protocol: u16,
}
#[derive(Deserialize)]
struct WireBlock {
    data: Vec<u8>,
}
#[derive(Serialize)]
struct Inserted {
    hash: String,
}
#[derive(Deserialize)]
struct AssetInput {
    name: String,
    serial: String,
    location: String,
    value_minor: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let dir = PathBuf::from(std::env::var("BIT_DATA_DIR").unwrap_or_else(|_| "bit-data".into()));
    let mut store = store::Store::open(&dir)?;
    let identity = store.identity()?;
    let mut ledger = store.load()?;
    let token = store.admin_token()?;
    if ledger.community().is_none() {
        let c = CommunityId(*Uuid::now_v7().as_bytes());
        let unsigned = ledger.next_unsigned(
            &identity,
            Operation::Genesis {
                community: c,
                name: std::env::var("BIT_COMMUNITY_NAME")
                    .unwrap_or_else(|_| "Local Community".into()),
                bit_decimals: 2,
            },
            now(),
        )?;
        let block = identity.sign(unsigned);
        let hash = ledger.insert(block.clone())?;
        store.save(hash, &block)?;
        tracing::info!(community = %hex::encode(c.0), "created community genesis");
    }
    let state = Arc::new(Mutex::new(App {
        ledger,
        store,
        identity,
        token,
    }));
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/state", get(state_view))
        .route("/v1/blocks", post(import_block))
        .route("/v1/admin/assets", post(create_asset))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let addr = std::env::var("BIT_LISTEN").unwrap_or_else(|_| "0.0.0.0:8787".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "Bit node listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        protocol: PROTOCOL_VERSION,
    })
}
async fn state_view(State(s): State<Shared>) -> Json<serde_json::Value> {
    let a = s.lock().await;
    let p = a.ledger.state();
    let members: Vec<_> = p.members.iter().map(|(id,(name,roles))| serde_json::json!({"account":hex::encode(id.0),"name":name,"roles":roles})).collect();
    let assets: Vec<_> = p.assets.iter().map(|(id,x)| serde_json::json!({"asset_id":id,"name":x.name,"serial":x.serial,"location":x.location,"holder":x.holder.map(|v|hex::encode(v.0)),"pending_offer":x.pending_offer.map(|v|hex::encode(v.0))})).collect();
    let bit_balances: Vec<_> = p
        .bit_balances
        .iter()
        .map(|(id, value)| serde_json::json!({"account":hex::encode(id.0),"amount_minor":value}))
        .collect();
    Json(
        serde_json::json!({"members":members,"assets":assets,"bit_balances":bit_balances,"journal":p.journal,"chat_count":p.chats.len()}),
    )
}
async fn import_block(
    State(s): State<Shared>,
    Json(w): Json<WireBlock>,
) -> Result<Json<Inserted>, (StatusCode, String)> {
    let block: Block = postcard::from_bytes(&w.data).map_err(bad)?;
    commit(&s, block)
        .await
        .map(|h| {
            Json(Inserted {
                hash: hex::encode(h.0),
            })
        })
        .map_err(bad)
}
async fn create_asset(
    State(s): State<Shared>,
    headers: HeaderMap,
    Json(x): Json<AssetInput>,
) -> Result<Json<Inserted>, (StatusCode, String)> {
    let mut a = s.lock().await;
    authorize(&headers, &a.token)?;
    let op = Operation::AssetCreate {
        asset_id: Uuid::now_v7(),
        name: x.name,
        serial: x.serial,
        location: x.location,
        value_minor: x.value_minor,
    };
    let block = a.identity.sign(
        a.ledger
            .next_unsigned(&a.identity, op, now())
            .map_err(bad)?,
    );
    let mut candidate = a.ledger.clone();
    let hash = candidate.insert(block.clone()).map_err(bad)?;
    a.store.save(hash, &block).map_err(internal)?;
    a.ledger = candidate;
    Ok(Json(Inserted {
        hash: hex::encode(hash.0),
    }))
}
async fn commit(s: &Shared, block: Block) -> Result<bit_core::Hash> {
    let mut a = s.lock().await;
    let mut candidate = a.ledger.clone();
    let hash = candidate.insert(block.clone())?;
    a.store.save(hash, &block)?;
    a.ledger = candidate;
    Ok(hash)
}
fn authorize(headers: &HeaderMap, token: &str) -> Result<(), (StatusCode, String)> {
    let got = headers
        .get("authorization")
        .and_then(|x| x.to_str().ok())
        .and_then(|x| x.strip_prefix("Bearer "));
    if got == Some(token) {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, "unauthorized".into()))
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn bad(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}
fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
