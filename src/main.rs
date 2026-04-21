use axum::{Json, Router, extract::State, http::{HeaderMap, StatusCode, header}, routing::{get, post}};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::{sync::Arc, time::{SystemTime, UNIX_EPOCH}};
use serde::{Serialize, Deserialize};
use redis::{aio::ConnectionManager, AsyncCommands};
use tokio_util::{sync::{CancellationToken}, task::{TaskTracker}};

pub struct AppState {
    pub db: PgPool,
    pub redis: ConnectionManager,
}

type SharedState = Arc<AppState>;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct GameUser {
    pub id: i64,
    pub token: String,
    pub first_name: String,
    pub per_click: i64,
    pub auto_click: i64,
    pub balance: i64,
    pub last_sync: i64,
}

#[derive(Deserialize)]
pub struct ClickPayload {
    pub data: i64,
}

#[derive(Serialize)]
pub struct SyncResponse {
    pub balance: i64,
    pub last_sync: i64,
}

#[derive(Deserialize)]
pub struct BuyUpgradePayload {
    pub upgrade_id: String,
}

#[derive(Serialize, Clone)]
pub struct UpgradeInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub upgrade_type: &'static str,
    pub base_price: i64,
    pub base_power: i64,
    pub price_growth: f64,
}

// --- Защита от спама (Rate Limiting) ---
async fn rate_limit_check(redis: &mut redis::aio::ConnectionManager, token: &str) -> Result<(), (StatusCode, String)> {
    let blocked_key = format!("blocked:{}", token);
    let rl_key = format!("rl:{}", token);

    let is_blocked: bool = redis.exists(&blocked_key).await.unwrap_or(false);
    if is_blocked {
        return Err((StatusCode::FORBIDDEN, "Token is blocked for 15 minutes".into()));
    }

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;

    let last_req: Option<i64> = redis.get(&rl_key).await.unwrap_or(None);
    if let Some(last) = last_req {
        if now_ms - last < 1500 {
            let _: () = redis.set_ex(&blocked_key, "1", 900).await.unwrap_or_default();
            return Err((StatusCode::TOO_MANY_REQUESTS, "Spam detected. Blocked for 15 minutes".into()));
        }
    }

    let _: () = redis.set_ex(&rl_key, now_ms, 2).await.unwrap_or_default();
    Ok(())
}

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

async fn get_game_user(
    db: &PgPool, 
    redis: &mut redis::aio::ConnectionManager, 
    token: &str
) -> Result<Option<GameUser>, (StatusCode, String)> {
    let cache_key = format!("user:{}", token);

    if let Ok(Some(cached_data)) = redis.get::<_, Option<String>>(&cache_key).await {
        if let Ok(user) = serde_json::from_str(&cached_data) {
            return Ok(Some(user));
        }
    }

    let user = sqlx::query_as::<_, GameUser>(
        r#"SELECT id, token, first_name, per_click, auto_click, balance, last_sync FROM "user" WHERE token = $1"#
    )
    .bind(token)
    .fetch_optional(db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(u) = &user {
        let json = serde_json::to_string(u).unwrap();
        let _: () = redis.set_ex(&cache_key, json, 60).await.unwrap_or_default();
    }
    Ok(user)
}

async fn api_data_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<GameUser>, (StatusCode, String)> {
    let mut redis = state.redis.clone();
    let token = extract_token(&headers)?;
    rate_limit_check(&mut redis, token).await?;

    if let Some(user) = get_game_user(&state.db, &mut redis, token).await? {
        return Ok(Json(user));
    }

    let now_sec = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    
    let new_user = sqlx::query_as::<_, GameUser>(
        r#"
        INSERT INTO "user" (token, first_name, per_click, auto_click, balance, last_sync)
        VALUES ($1, 'Player', 1, 0, 0, $2)
        RETURNING id, token, first_name, per_click, auto_click, balance, last_sync
        "#
    )
    .bind(token)
    .bind(now_sec)
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let json = serde_json::to_string(&new_user).unwrap();
    let _: () = redis.set_ex(format!("user:{}", token), json, 60).await.unwrap_or_default();

    Ok(Json(new_user))
}

// Общая логика начисления баланса
async fn process_clicks_and_sync(
    state: &SharedState,
    token: &str,
    clicks_data: i64,
) -> Result<Json<SyncResponse>, (StatusCode, String)> {
    let mut redis = state.redis.clone();
    
    rate_limit_check(&mut redis, token).await?;

    let user = get_game_user(&state.db, &mut redis, token)
        .await?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let now_sec = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    
    let actual_clicks = clicks_data.min(100); 
    let time_diff = (now_sec - user.last_sync).max(0);

    let total_earned = (actual_clicks * user.per_click) + (time_diff * user.auto_click);

    let final_balance = user.balance + total_earned;
    let final_sync_time = now_sec;

    sqlx::query(r#"UPDATE "user" SET balance = $1, last_sync = $2 WHERE token = $3"#)
        .bind(final_balance)
        .bind(final_sync_time)
        .bind(token)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)))?;

    let _: () = redis.del(format!("user:{}", token)).await.unwrap_or_default();

    Ok(Json(SyncResponse {
        balance: final_balance,
        last_sync: final_sync_time,
    }))
}

async fn process_buy_upgrade(
    state: &SharedState,
    token: &str,
    payload: BuyUpgradePayload,
) -> Result<Json<GameUser>, (StatusCode, String)> {
    let mut redis = state.redis.clone();
    
    let config = get_upgrade_config(&payload.upgrade_id)
        .ok_or((StatusCode::BAD_REQUEST, "Неверный ID апгрейда".into()))?;

    let user = get_game_user(&state.db, &mut redis, token).await?
        .ok_or((StatusCode::NOT_FOUND, "Пользователь не найден".into()))?;

    let upgrade_row: (i32,) = sqlx::query_as("SELECT level FROM user_upgrades WHERE user_id = $1 AND upgrade_id = $2")
        .bind(user.id)
        .bind(&config.id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .unwrap_or((0,));

    let current_level = upgrade_row.0;

    let price = (config.base_price as f64 * config.price_growth.powi(current_level)) as i64;

    let mut tx = state.db.begin().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let update_result = sqlx::query(
            "UPDATE \"user\" SET balance = balance - $1 WHERE id = $2 AND balance >= $1"
        )
        .bind(price)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if update_result.rows_affected() == 0 {
        return Err((StatusCode::PAYMENT_REQUIRED, "Недостаточно средств".into()));
    }

    sqlx::query(r#"
        INSERT INTO user_upgrades (user_id, upgrade_id, level) 
        VALUES ($1, $2, 1) 
        ON CONFLICT (user_id, upgrade_id) DO UPDATE SET level = user_upgrades.level + 1
    "#)
    .bind(user.id)
    .bind(&config.id)
    .execute(&mut *tx).await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let field = if config.upgrade_type == "auto" { "auto_click" } else { "per_click" };
    let update_stat_query = format!("UPDATE \"user\" SET {field} = {field} + $1 WHERE id = $2");
    
    sqlx::query(&update_stat_query)
        .bind(config.base_power)
        .bind(user.id)
        .execute(&mut *tx).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tx.commit().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis.del(format!("user:{}", token)).await.unwrap_or_default();

    let updated_user = get_game_user(&state.db, &mut redis, token).await?
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Ошибка синхронизации".into()))?;

    Ok(Json(updated_user))
}

pub fn get_upgrade_config(id: &str) -> Option<UpgradeInfo> {
    match id {
        // --- КАТЕГОРИЯ: GPU ---
        "gpu_1" => Some(UpgradeInfo { id: "gpu_1", name: "GT 710", upgrade_type: "auto", base_price: 16, base_power: 1, price_growth: 1.6 }),
        "gpu_2" => Some(UpgradeInfo { id: "gpu_2", name: "GTX 1050 Ti", upgrade_type: "auto", base_price: 256, base_power: 8, price_growth: 1.6 }),
        "gpu_3" => Some(UpgradeInfo { id: "gpu_3", name: "RTX 2060 Super", upgrade_type: "auto", base_price: 4096, base_power: 64, price_growth: 1.6 }),
        "gpu_4" => Some(UpgradeInfo { id: "gpu_4", name: "RTX 4070 Ti", upgrade_type: "auto", base_price: 65536, base_power: 512, price_growth: 1.6 }),
        "gpu_5" => Some(UpgradeInfo { id: "gpu_5", name: "RTX 4090 OC", upgrade_type: "auto", base_price: 1048576, base_power: 4096, price_growth: 1.6 }),
        "gpu_6" => Some(UpgradeInfo { id: "gpu_6", name: "Antminer S21 Pro", upgrade_type: "auto", base_price: 16777216, base_power: 32768, price_growth: 1.6 }),
        "gpu_7" => Some(UpgradeInfo { id: "gpu_7", name: "RTX 5090 Ti Prototype", upgrade_type: "auto", base_price: 268435456, base_power: 262144, price_growth: 1.6 }),

        // --- КАТЕГОРИЯ: CPU ---
        "cpu_1" => Some(UpgradeInfo { id: "cpu_1", name: "Intel Celeron", upgrade_type: "auto", base_price: 32, base_power: 1, price_growth: 1.6 }),
        "cpu_2" => Some(UpgradeInfo { id: "cpu_2", name: "Core i3-10100", upgrade_type: "auto", base_price: 512, base_power: 8, price_growth: 1.6 }),
        "cpu_3" => Some(UpgradeInfo { id: "cpu_3", name: "Core i7-13700K", upgrade_type: "auto", base_price: 8192, base_power: 64, price_growth: 1.6 }),
        "cpu_4" => Some(UpgradeInfo { id: "cpu_4", name: "Ryzen 9 7950X", upgrade_type: "auto", base_price: 131072, base_power: 512, price_growth: 1.6 }),
        "cpu_5" => Some(UpgradeInfo { id: "cpu_5", name: "Threadripper 3990X", upgrade_type: "auto", base_price: 2097152, base_power: 4096, price_growth: 1.6 }),
        "cpu_6" => Some(UpgradeInfo { id: "cpu_6", name: "Quantum Cluster", upgrade_type: "auto", base_price: 33554432, base_power: 32768, price_growth: 1.6 }),
        "cpu_7" => Some(UpgradeInfo { id: "cpu_7", name: "EPYC 9654 Farm", upgrade_type: "auto", base_price: 536870912, base_power: 262144, price_growth: 1.6 }),

        // --- КАТЕГОРИЯ: MOUSE ---
        "mouse_1" => Some(UpgradeInfo { id: "mouse_1", name: "Офисная мышь", upgrade_type: "click", base_price: 64, base_power: 1, price_growth: 1.6 }),
        "mouse_2" => Some(UpgradeInfo { id: "mouse_2", name: "Игровая X7", upgrade_type: "click", base_price: 1024, base_power: 8, price_growth: 1.6 }),
        "mouse_3" => Some(UpgradeInfo { id: "mouse_3", name: "Logitech G502", upgrade_type: "click", base_price: 16384, base_power: 64, price_growth: 1.6 }),
        "mouse_4" => Some(UpgradeInfo { id: "mouse_4", name: "Razer DeathAdder", upgrade_type: "click", base_price: 262144, base_power: 512, price_growth: 1.6 }),
        "mouse_5" => Some(UpgradeInfo { id: "mouse_5", name: "Custom Clicker v2", upgrade_type: "click", base_price: 4194304, base_power: 4096, price_growth: 1.6 }),
        "mouse_6" => Some(UpgradeInfo { id: "mouse_6", name: "Neural Link", upgrade_type: "click", base_price: 67108864, base_power: 32768, price_growth: 1.6 }),
        "mouse_7" => Some(UpgradeInfo { id: "mouse_7", name: "Telepathic Command", upgrade_type: "click", base_price: 173741824, base_power: 262144, price_growth: 1.6 }),

        _ => None,
    }
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

// --- Helpers ---

fn extract_token(headers: &HeaderMap) -> Result<&str, (StatusCode, String)> {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header".to_string()))?;
    auth.strip_prefix("Bearer ")
        .ok_or((StatusCode::BAD_REQUEST, "Invalid Authorization header format".to_string()))
}

async fn spawn_db_syncer(db: PgPool, mut redis: ConnectionManager, token: CancellationToken) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(20));
    
    loop {
        tokio::select! {
            _ = interval.tick() => {
                do_sync(&db, &mut redis).await;
            }
            _ = token.cancelled() => {
                println!("Завершение работы: финальная синхронизация...");
                do_sync(&db, &mut redis).await;
                break;
            }
        }
    }
}

async fn do_sync(db: &PgPool, redis: &mut ConnectionManager) {
    let tokens: Vec<String> = redis.smembers("sync_queue").await.unwrap_or_default();
    for t in tokens {
        let delta: i64 = redis.getset(format!("pending_balance:{}", t), 0).await.unwrap_or(0);
        if delta > 0 {
            let _ = sqlx::query(r#"UPDATE "user" SET balance = balance + $1 WHERE token = $2"#)
                .bind(delta)
                .bind(&t)
                .execute(db).await;
        }
        let _: () = redis.srem("sync_queue", &t).await.unwrap_or_default();
    }
}

async fn shutdown_signal(token: CancellationToken, _tracker: TaskTracker) {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl+c");
    
    println!("Получен сигнал остановки (Ctrl+C)...");
    token.cancel();
}
