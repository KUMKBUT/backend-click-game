use std::collections::HashMap;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
};
use uuid::Uuid;
use redis::AsyncCommands;

use crate::SharedState;
use crate::ws::config::{RafflePlayer, RaffleRoom};

pub async fn fetch_raffle_room(
    redis: &mut redis::aio::ConnectionManager,
    uuid: &str,
) -> Result<RaffleRoom, (StatusCode, String)> {
    let cache_room_key = format!("raffle:{}", uuid);
    let cache_bet_key = format!("raffle:{}:bets", uuid);

    let time_str: Option<String> = redis
        .hget(&cache_room_key, "time")
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let time = time_str
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    
    let bets: HashMap<String, String> = redis
        .hgetall(&cache_bet_key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let players = bets
        .into_iter()
        .filter_map(|(_, key)| serde_json::from_str::<RafflePlayer>(&key).ok())
        .collect();

    Ok(RaffleRoom {
        time,
        bets: players,
    })
}

pub async fn set_new_raffle_room(
    State(state): State<SharedState>,
) -> Result<Json<RaffleRoom>, (StatusCode, String)> {
    let mut redis = state.redis.clone();
    let id = Uuid::new_v4().to_string();
    
    let cache_room_key = format!("raffle:{}", id);

    let fields = [
        ("time", "30"), 
        ("winning_id", "0"),
        ("winning_name", "Name"),
        ("winning_amount", "0")
    ];

    let _: () = redis
        .hset_multiple(&cache_room_key, &fields)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
    let _: () = redis.expire(&cache_room_key, 60).await.ok().unwrap_or(());
    
    let room_state = fetch_raffle_room(&mut redis, &id).await?;

    Ok(Json(room_state))
}

pub async fn get_all_raffle_players(
    State(state): State<SharedState>,
    uuid: &str,
) -> Result<Json<RaffleRoom>, (StatusCode, String)> {
    let mut redis = state.redis.clone();

    let room_state = fetch_raffle_room(&mut redis, uuid).await?;
    
    Ok(Json(room_state))
}

pub async fn bet_raffle(
    State(state): State<SharedState>,
    uuid: &str,
    amount: i64,
    user_id: i64,
    first_name: &str,
) -> Result<Json<RaffleRoom>, (StatusCode, String)> {
    let mut redis = state.redis.clone();

    if amount <= 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be positive".to_string()));
    }

    let cache_bet_key = format!("raffle:{}:bets", uuid);

    let player_json: Option<String> = redis
        .hget(&cache_bet_key, user_id.to_string())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let player = match player_json {
        Some(json) => {
            let mut p: RafflePlayer = serde_json::from_str(&json)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            p.total_amount += amount;
            p
        },
        None => RafflePlayer {
            user_id,
            first_name: first_name.to_string(),
            total_amount: amount,
        },
    };

    let result = sqlx::query(
       r#"
       UPDATE "user" 
       SET balance = balance - $1 
       WHERE id = $2 AND balance >= $1
       "#
    )
    .bind(&amount)
    .bind(&user_id)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::PAYMENT_REQUIRED, "Insufficient funds".to_string()));
    }

    let player_serde = serde_json::to_string(&player)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis
        .hset(&cache_bet_key, user_id.to_string(), player_serde)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis.expire(&cache_bet_key, 60).await.ok().unwrap_or(());

    let room_state = fetch_raffle_room(&mut redis, uuid).await?;
    
    Ok(Json(room_state))
}
