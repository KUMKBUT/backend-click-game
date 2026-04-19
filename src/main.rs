use axum::{Json, Router, extract::State, http::{HeaderMap, StatusCode, header}, routing::{get, post}};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::{sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use serde::{Serialize, Deserialize};
use redis::{aio::ConnectionManager, AsyncCommands};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

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

const MAX_CLICKS_PER_REQUEST: i64 = 100;
const MAX_BALANCE: i64 = 1_000_000_000_000;
const RATE_LIMIT_INTERVAL_MS: i64 = 500;
const BLOCK_DURATION_SECS: u64 = 60;
const RATE_LIMIT_TTL_SECS: u64 = 2;
const USER_CACHE_TTL_SECS: u64 = 300;

// ─── Rate limiting (один Lua round-trip) ──────────────────────────────────────

async fn rate_limit_check(
    redis: &mut ConnectionManager,
    token: &str,
) -> Result<(), (StatusCode, String)> {
    let script = redis::Script::new(r#"
        local blocked_key = KEYS[1]
        local rl_key = KEYS[2]
        local now = tonumber(ARGV[1])
        local interval = tonumber(ARGV[2])
        local block_ttl = tonumber(ARGV[3])
        local rl_ttl = tonumber(ARGV[4])
        if redis.call('EXISTS', blocked_key) == 1 then return -1 end
        local last = redis.call('GET', rl_key)
        if last then
            if now - tonumber(last) < interval then
                redis.call('SET', blocked_key, '1', 'EX', block_ttl)
                return -2
            end
        end
        redis.call('SET', rl_key, now, 'EX', rl_ttl)
        return 1
    "#);

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    let blocked_key = format!("blocked:{token}");
    let rl_key = format!("rl:{token}");

    let result: i64 = script
        .key(&blocked_key).key(&rl_key)
        .arg(now_ms).arg(RATE_LIMIT_INTERVAL_MS)
        .arg(BLOCK_DURATION_SECS as i64).arg(RATE_LIMIT_TTL_SECS as i64)
        .invoke_async(redis).await.unwrap_or(1);

    match result {
        -1 => Err((StatusCode::FORBIDDEN, "Token is blocked".into())),
        -2 => Err((StatusCode::TOO_MANY_REQUESTS, "Too many requests".into())),
        _ => Ok(()),
    }
}

// ─── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());
    let redis_client = redis::Client::open(redis_url)?;
    let redis_manager = redis_client.get_connection_manager().await?;

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool_size: u32 = std::env::var("DB_POOL_SIZE")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(20);

    let pool = PgPoolOptions::new()
        .max_connections(pool_size)
        .min_connections(5)
        // Увеличиваем acquire_timeout — даём PgBouncer время найти слот
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(300))
        .connect(&db_url)
        .await?;

    sqlx::migrate!().run(&pool).await?;

    let tracker = TaskTracker::new();
    let token = CancellationToken::new();

    let shared_state = Arc::new(AppState { db: pool.clone(), redis: redis_manager.clone() });

    let sync_db = pool.clone();
    let sync_redis = redis_manager.clone();
    let sync_token = token.clone();

    tracker.spawn(async move {
        println!("🚀 Воркер синхронизации запущен");
        spawn_db_syncer(sync_db, sync_redis, sync_token).await;
    });

    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/data", get(api_data_handler))
        .route("/api/click", post(api_click_handler))
        .route("/api/sync", post(api_sync_handler))
        .with_state(shared_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3719").await?;
    println!("📡 Сервер слушает на 0.0.0.0:3719");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(token.clone(), tracker.clone()))
        .await?;

    tracker.close();
    tracker.wait().await;
    println!("Все данные сохранены. Бэкенд остановлен.");
    Ok(())
}

// ─── Получение пользователя ────────────────────────────────────────────────────
// КЛЮЧЕВОЕ ИЗМЕНЕНИЕ: при cache miss сначала пробуем GET из Redis-флага существования.
// Только если пользователя точно нет — идём в БД за INSERT.
// Это убирает лавину INSERT'ов при старте нагрузочного теста.

async fn get_or_create_user(
    db: &PgPool,
    redis: &mut ConnectionManager,
    token: &str,
) -> Result<GameUser, (StatusCode, String)> {
    // 1. Смотрим полный кэш пользователя
    let cache_key = format!("user:{token}");
    if let Ok(Some(data)) = redis.get::<_, Option<String>>(&cache_key).await {
        if let Ok(user) = serde_json::from_str::<GameUser>(&data) {
            return Ok(user);
        }
    }

    // 2. Cache miss → идём в БД. UPSERT атомарен — нет гонки между репликами.
    let now_sec = now_secs();
    let user = sqlx::query_as::<_, GameUser>(
        r#"INSERT INTO "user" (token, first_name, per_click, auto_click, balance, last_sync)
           VALUES ($1, 'Player', 1, 0, 0, $2)
           ON CONFLICT (token) DO UPDATE
             SET last_sync = "user".last_sync
           RETURNING id, token, first_name, per_click, auto_click, balance, last_sync"#,
    )
    .bind(token)
    .bind(now_sec)
    .fetch_one(db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 3. Кладём в кэш
    if let Ok(json) = serde_json::to_string(&user) {
        let _: () = redis.set_ex(&cache_key, json, USER_CACHE_TTL_SECS).await.unwrap_or_default();
    }
    Ok(user)
}

async fn get_game_user(
    db: &PgPool,
    redis: &mut ConnectionManager,
    token: &str,
) -> Result<Option<GameUser>, (StatusCode, String)> {
    let cache_key = format!("user:{token}");
    if let Ok(Some(data)) = redis.get::<_, Option<String>>(&cache_key).await {
        if let Ok(user) = serde_json::from_str::<GameUser>(&data) {
            return Ok(Some(user));
        }
    }

    let user = sqlx::query_as::<_, GameUser>(
        r#"SELECT id, token, first_name, per_click, auto_click, balance, last_sync
           FROM "user" WHERE token = $1"#,
    )
    .bind(token)
    .fetch_optional(db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(u) = &user {
        if let Ok(json) = serde_json::to_string(u) {
            let _: () = redis.set_ex(&cache_key, json, USER_CACHE_TTL_SECS).await.unwrap_or_default();
        }
    }
    Ok(user)
}

async fn health_check() -> StatusCode { StatusCode::OK }

// ─── GET /api/data ─────────────────────────────────────────────────────────────

async fn api_data_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<GameUser>, (StatusCode, String)> {
    let mut redis = state.redis.clone();
    let token = extract_token(&headers)?;
    rate_limit_check(&mut redis, token).await?;
    // get_or_create: один запрос вместо GET + conditional INSERT
    let user = get_or_create_user(&state.db, &mut redis, token).await?;
    Ok(Json(user))
}

// ─── POST /api/click ───────────────────────────────────────────────────────────

async fn api_click_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ClickPayload>,
) -> Result<Json<SyncResponse>, (StatusCode, String)> {
    let token = extract_token(&headers)?;
    let mut redis = state.redis.clone();
    rate_limit_check(&mut redis, token).await?;

    // click не создаёт пользователя — только для существующих
    let user = get_game_user(&state.db, &mut redis, token)
        .await?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let actual_clicks = payload.data.clamp(0, MAX_CLICKS_PER_REQUEST);
    let delta = actual_clicks.saturating_mul(user.per_click).min(MAX_BALANCE);

    if delta > 0 {
        let pending_key = format!("pending_balance:{token}");
        let mut pipe = redis::pipe();
        pipe.atomic()
            .cmd("INCRBY").arg(&pending_key).arg(delta).ignore()
            .cmd("EXPIRE").arg(&pending_key).arg(300i64).ignore()
            .cmd("SADD").arg("sync_queue").arg(token).ignore();
        let _: () = pipe.query_async(&mut redis).await.unwrap_or_default();
    }

    let pending_key = format!("pending_balance:{token}");
    let pending: i64 = redis.get(&pending_key).await.unwrap_or(0);
    let optimistic_balance = user.balance.saturating_add(pending).min(MAX_BALANCE);

    Ok(Json(SyncResponse { balance: optimistic_balance, last_sync: now_secs() }))
}

// ─── POST /api/sync ────────────────────────────────────────────────────────────

async fn api_sync_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<SyncResponse>, (StatusCode, String)> {
    let token = extract_token(&headers)?;
    let mut redis = state.redis.clone();
    rate_limit_check(&mut redis, token).await?;

    let user = get_game_user(&state.db, &mut redis, token)
        .await?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let pending_key = format!("pending_balance:{token}");
    let pending: i64 = redis.get(&pending_key).await.unwrap_or(0);
    let now_sec = now_secs();
    let offline_gains = (now_sec - user.last_sync).max(0) * user.auto_click;

    let current_balance = user.balance
        .saturating_add(pending)
        .saturating_add(offline_gains)
        .min(MAX_BALANCE);

    Ok(Json(SyncResponse { balance: current_balance, last_sync: now_sec }))
}

// ─── Batch sync ────────────────────────────────────────────────────────────────

async fn spawn_db_syncer(db: PgPool, mut redis: ConnectionManager, token: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let is_leader: bool = redis.set_nx("cron_lock:sync", "1").await.unwrap_or(false);
                if is_leader {
                    let _: () = redis.expire("cron_lock:sync", 8).await.unwrap_or_default();
                    do_batch_sync(&db, &mut redis).await;
                }
            }
            _ = token.cancelled() => {
                let _: () = redis.del("cron_lock:sync").await.unwrap_or_default();
                do_batch_sync(&db, &mut redis).await;
                break;
            }
        }
    }
}

async fn do_batch_sync(db: &PgPool, redis: &mut ConnectionManager) {
    let tokens: Vec<String> = redis::cmd("SPOP")
        .arg("sync_queue").arg(10_000)
        .query_async(redis).await.unwrap_or_default();

    if tokens.is_empty() { return; }

    let mut pipe = redis::pipe();
    for t in &tokens {
        pipe.cmd("GETDEL").arg(format!("pending_balance:{t}"));
    }
    let deltas_raw: Vec<Option<i64>> = pipe.query_async(redis).await.unwrap_or_default();

    let mut batch_tokens: Vec<String> = Vec::with_capacity(tokens.len());
    let mut batch_deltas: Vec<i64> = Vec::with_capacity(tokens.len());

    for (t, delta_opt) in tokens.iter().zip(deltas_raw.iter()) {
        let delta = delta_opt.unwrap_or(0);
        if delta > 0 {
            batch_tokens.push(t.clone());
            batch_deltas.push(delta);
        }
    }

    if batch_tokens.is_empty() { return; }

    let result = sqlx::query(
        r#"UPDATE "user" AS u
           SET balance   = LEAST(u.balance + data.delta, $1),
               last_sync = EXTRACT(EPOCH FROM NOW())::bigint
           FROM (SELECT UNNEST($2::text[]) AS token, UNNEST($3::bigint[]) AS delta) AS data
           WHERE u.token = data.token"#,
    )
    .bind(MAX_BALANCE)
    .bind(&batch_tokens)
    .bind(&batch_deltas)
    .execute(db)
    .await;

    match result {
        Ok(r) => {
            println!("✅ Синхронизировано: {} пользователей", r.rows_affected());
            let mut pipe = redis::pipe();
            for t in &batch_tokens {
                pipe.del(format!("user:{t}")).ignore();
            }
            let _: () = pipe.query_async(redis).await.unwrap_or_default();
        }
        Err(e) => {
            eprintln!("🚨 Ошибка batch-синхронизации: {e}");
            let mut pipe = redis::pipe();
            for (t, &delta) in batch_tokens.iter().zip(batch_deltas.iter()) {
                let pk = format!("pending_balance:{t}");
                pipe.cmd("INCRBY").arg(&pk).arg(delta).ignore();
                pipe.cmd("EXPIRE").arg(&pk).arg(300i64).ignore();
                pipe.sadd("sync_queue", t).ignore();
            }
            let _: () = pipe.query_async(redis).await.unwrap_or_default();
        }
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

fn extract_token(headers: &HeaderMap) -> Result<&str, (StatusCode, String)> {
    headers.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header".to_string()))?
        .strip_prefix("Bearer ")
        .ok_or((StatusCode::BAD_REQUEST, "Invalid Authorization header format".to_string()))
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

async fn shutdown_signal(token: CancellationToken, _tracker: TaskTracker) {
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
    println!("Получен сигнал остановки...");
    token.cancel();
}