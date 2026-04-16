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
