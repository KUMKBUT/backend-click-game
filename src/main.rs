use axum::{Json, Router, extract::State, http::{HeaderMap, StatusCode}, routing::{get, post}};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::{sync::Arc};
use redis::{aio::ConnectionManager};
use tokio_util::{sync::{CancellationToken}, task::{TaskTracker}};

mod config;
mod helpers;
use helpers::{ 
    extract_token, spawn_db_syncer, shutdown_signal, 
    process_clicks_and_sync, process_fetch_data_user, process_buy_upgrade, 
    process_get_top_user, process_transfer, ws_handler
};
use config::{ 
    GameUser, ClickPayload, SyncResponse, 
    BuyUpgradePayload, TopUsers, TransferReq, TransferRes
};

mod services;
use crate::services::routes::{api_service_create_handler, api_service_get_info_handler};
pub struct AppState {
    pub db: PgPool,
    pub redis: ConnectionManager,
}

pub type SharedState = Arc<AppState>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());
    let redis_client = redis::Client::open(redis_url)?;
    let redis_manager = redis_client.get_connection_manager().await?;
    let db_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let pool = PgPoolOptions::new()
        .max_connections(50)
        .connect(&db_url)
        .await?;

    sqlx::migrate!().run(&pool).await?;

    let tracker = TaskTracker::new();
    let token = CancellationToken::new();

    let shared_state = Arc::new(AppState {
        db: pool.clone(),
        redis: redis_manager.clone(),
    });

    let sync_db = pool.clone();
    let sync_redis = redis_manager.clone();
    let sync_token = token.clone();
    
    tracker.spawn(async move {
        println!("🚀 Воркер синхронизации запущен");
        spawn_db_syncer(sync_db, sync_redis, sync_token).await;
    });

    let app = Router::new()
        .route("/api/data", get(api_data_handler))
        .route("/api/click", post(api_click_handler))
        .route("/api/sync", post(api_sync_handler))
        .route("/api/buy-upgrade", post(api_buy_upgrade_handler))
        .route("/api/top", post(api_top_user_handler))
        .route("/api/transfer", post(api_transfer_handler))
        .route("/ws", get(ws_handler))
        .route("/api/service/create", post(api_service_create_handler))
        .route("/api/service/info", get(api_service_get_info_handler))
        .with_state(shared_state);

    let addr = "0.0.0.0:3719";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("📡 Сервер слушает на {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(token.clone(), tracker.clone())) // Добавлен tracker.clone()
        .await?;

    tracker.close();
    tracker.wait().await;
    
    println!("Все данные сохранены. Бэкенд остановлен.");
    Ok(())
}

// Роут /api/data ---
async fn api_data_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<GameUser>, (StatusCode, String)> {
    let token = extract_token(&headers)?;

    process_fetch_data_user(&state, token).await
}
// Роут /api/click
async fn api_click_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ClickPayload>,
) -> Result<Json<SyncResponse>, (StatusCode, String)> {
    let token = extract_token(&headers)?;
    
    process_clicks_and_sync(&state, token, payload.data).await
}
// Роут /api/sync
async fn api_sync_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<SyncResponse>, (StatusCode, String)> {
    let token = extract_token(&headers)?;

    process_clicks_and_sync(&state, token, 0).await
}
// Роут /api/buy_upgrade
async fn api_buy_upgrade_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<BuyUpgradePayload>,
) -> Result<Json<GameUser>, (StatusCode, String)> {
    let token = extract_token(&headers)?;
    
    process_buy_upgrade(&state, token, payload).await
}
// Роут /api/top
async fn api_top_user_handler(
	State(state): State<SharedState>,
	headers: HeaderMap,
) -> Result<Json<TopUsers>,( StatusCode, String)> {
	let token = extract_token(&headers)?;
	
	process_get_top_user(&state, token, 50).await
}
// Роут /api/transfer
async fn api_transfer_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<TransferReq>,
) -> Result<Json<TransferRes>,( StatusCode, String)> {
    let token = extract_token(&headers)?;

    process_transfer(&state, token, payload).await
}

