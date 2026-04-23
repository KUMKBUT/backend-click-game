use axum::{Json, http::{HeaderMap, StatusCode, header}};
use redis::{aio::ConnectionManager, AsyncCommands};
use std::{time::{SystemTime, UNIX_EPOCH}};
use sqlx::postgres::{PgPool};
use tokio_util::{sync::{CancellationToken}, task::{TaskTracker}};

use crate::config;
use crate::{SharedState};
use config::{ get_upgrade_config, GameUser, SyncResponse, BuyUpgradePayload, TopUsers, TopUserItem, TransferReq, TransferRes};

// --- Защита от спама и блокировка токенов ---
pub async fn rate_limit_check(redis: &mut redis::aio::ConnectionManager, token: &str) -> Result<(), (StatusCode, String)> {
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

// --- Получени пользователя из бд или кеша ---
pub async fn get_game_user(
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

// --- Общая логика начисления баланса ---
pub async fn process_clicks_and_sync(
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

// --- Обработка получения юзера или создание нового ---
pub async fn process_fetch_data_user(
    state: &SharedState,
    token: &str,
) -> Result<Json<GameUser>, (StatusCode, String)> {
    let mut redis = state.redis.clone();
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

// --- Обработка покупки апгрейдов --- 
pub async fn process_buy_upgrade(
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
// --- Получения топа юзеров ---
pub async fn process_get_top_user(
    state: &SharedState,
    _token: &str,
    limit: i64,
) -> Result<Json<TopUsers>, (StatusCode, String)> {
    let mut redis = state.redis.clone();
    let cache_key = "top_users_50";

    if let Ok(Some(cached_data)) = redis.get::<_, Option<String>>(cache_key).await {
        if let Ok(top_users) = serde_json::from_str::<TopUsers>(&cached_data) {
            return Ok(Json(top_users));
        }
    }

    let users_from_db = sqlx::query_as::<_, GameUser>(
        r#"SELECT id, token, first_name, per_click, auto_click, balance, last_sync 
           FROM "user" 
           ORDER BY balance DESC 
           LIMIT $1"#
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)))?;

    let top_list = users_from_db.into_iter().map(|u| TopUserItem {
        id: u.id,
        first_name: u.first_name,
        balance: u.balance,
        per_click: u.per_click,
        auto_click: u.auto_click,
    }).collect();

    let response = TopUsers { users: top_list };

    if let Ok(json) = serde_json::to_string(&response) {
        let _: () = redis.set_ex(cache_key, json, 60).await.unwrap_or_default();
    }

    Ok(Json(response))
}
pub async fn process_transfer(
    state: &SharedState,
    token: &str,
    payload: TransferReq,
) -> Result<Json<TransferRes>, (StatusCode, String)> {
    let mut redis = state.redis.clone();

    if payload.ammount <= 0 {
        return Err((StatusCode::BAD_REQUEST, "Сумма должна быть больше 0".into()));
    }

    let sender = get_game_user(&state.db, &mut redis, token)
        .await?
        .ok_or((StatusCode::UNAUTHORIZED, "Отправитель не найден".into()))?;

    if sender.id == payload.recipient {
        return Err((StatusCode::BAD_REQUEST, "Нельзя переводить самому себе".into()));
    }

    let mut tx = state.db.begin().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (id1, id2) = if sender.id < payload.recipient {
        (sender.id, payload.recipient)
    } else {
        (payload.recipient, sender.id)
    };

    sqlx::query("SELECT id FROM \"user\" WHERE id IN ($1, $2) FOR UPDATE")
        .bind(id1)
        .bind(id2)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Lock error: {}", e)))?;

    let sender_balance: (i64,) = sqlx::query_as("SELECT balance FROM \"user\" WHERE id = $1")
        .bind(sender.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if sender_balance.0 < payload.ammount {
        return Err((StatusCode::PAYMENT_REQUIRED, "Недостаточно средств".into()));
    }

    sqlx::query("UPDATE \"user\" SET balance = balance - $1 WHERE id = $2")
        .bind(payload.ammount)
        .bind(sender.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let res = sqlx::query("UPDATE \"user\" SET balance = balance + $1 WHERE id = $2")
        .bind(payload.ammount)
        .bind(payload.recipient)
        .execute(&mut *tx)
        .await;

    if res.is_err() {
        return Err((StatusCode::NOT_FOUND, "Получатель не найден".into()));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    sqlx::query(
        "INSERT INTO transfer_history (sender_id, recipient_id, amount, created_at)
         VALUES ($1, $2, $3, $4)"
    )
    .bind(sender.id)
    .bind(payload.recipient)
    .bind(payload.ammount)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("History error: {}", e)))?;

    tx.commit().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis.del(format!("user:{}", token))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(TransferRes {
        status: "success".into(),
        new_balance: sender_balance.0 - payload.ammount,
    }))
}

// --- Извлечение Bearer токена из заголовков запроса ---
pub fn extract_token(headers: &HeaderMap) -> Result<&str, (StatusCode, String)> {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header".to_string()))?;
    auth.strip_prefix("Bearer ")
        .ok_or((StatusCode::BAD_REQUEST, "Invalid Authorization header format".to_string()))
}

// -- Фоновый процесс периодической синхронизации данных с БД ---
pub async fn spawn_db_syncer(db: PgPool, mut redis: ConnectionManager, token: CancellationToken) {
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

// --- Логика переноса накопленного баланса из Redis в Postgres ---
pub async fn do_sync(db: &PgPool, redis: &mut ConnectionManager) {
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

// --- Обработка сигнала завершения работы и Graceful Shutdown ---
pub async fn shutdown_signal(token: CancellationToken, _tracker: TaskTracker) {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl+c");
    
    println!("Получен сигнал остановки (Ctrl+C)...");
    token.cancel();
}