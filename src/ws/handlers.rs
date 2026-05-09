use std::collections::HashMap;
use std::time::Duration;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
};
use tokio::time::{interval, MissedTickBehavior};
use rand::Rng;
use uuid::Uuid;
use redis::AsyncCommands;

use crate::SharedState;
use crate::ws::config::{RafflePlayer, RaffleRoom, RaffleRoomWin, WsOutgoing};

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

    let players: Vec<RafflePlayer> = bets
        .values()
        .filter_map(|v| serde_json::from_str(v).ok())
        .collect();

    let total_bank: i64 = players
        .iter()
        .map(|p| p.total_amount)
        .sum();


    Ok(RaffleRoom {
        time,
        bets: players,
        total_bank
    })
}

pub async fn create_raffle_room_internal(
    state: &SharedState,
) -> Result<String, (StatusCode, String)> {
    let mut redis = state.redis.clone();
    let id = Uuid::new_v4().to_string();

    let cache_room_key = format!("raffle:{}", id);

    let fields = [
        ("time", "30"),
        ("winning_id", "0"),
        ("winning_name", "Name"),
        ("winning_amount", "0"),
    ];

    let _: () = redis
        .hset_multiple(&cache_room_key, &fields)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis.expire(&cache_room_key, 60).await.ok().unwrap_or(());

    Ok(id)
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
    .bind(amount)
    .bind(user_id)
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

pub async fn internal_finish_raffle(
    state: SharedState,
    uuid: String,
) -> Result<RaffleRoomWin, (StatusCode, String)> {
    let mut redis = state.redis.clone();
    let cache_bet_key = format!("raffle:{}:bets", uuid);
    let cache_room_key = format!("raffle:{}", uuid);
    let lock_key = format!("raffle:{}:finishing", uuid);

    let acquired: Option<String> = redis::cmd("SET")
        .arg(&lock_key).arg("1").arg("NX").arg("EX").arg(30)
        .query_async(&mut redis)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if acquired.is_none() {
        return Err((StatusCode::CONFLICT, "Raffle is already being finalized".to_string()));
    }

    let _: String = redis
        .hget(&cache_room_key, "time")
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Raffle room not found".to_string()))?;

    let bets: HashMap<String, String> = redis
        .hgetall(&cache_bet_key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if bets.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No players in raffle".to_string()));
    }

    let players: Vec<RafflePlayer> = bets
        .values()
        .filter_map(|v| serde_json::from_str(v).ok())
        .collect();

    let total_bank: i64 = players.iter().map(|p| p.total_amount).sum();
    let winning_ticket = rand::thread_rng().gen_range(1..=total_bank);

    let mut current_sum = 0;
    let mut winner = None;
    for player in &players {
        current_sum += player.total_amount;
        if current_sum >= winning_ticket {
            winner = Some(player);
            break;
        }
    }

    let winner = winner
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Winner calculation failed".to_string()))?;

    let commission = (total_bank + 39) / 40;
    let final_prize = total_bank.saturating_sub(commission);

    sqlx::query(r#"UPDATE "user" SET balance = balance + $1 WHERE id = $2"#)
        .bind(final_prize)
        .bind(winner.user_id)
        .execute(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let finishged_msg = serde_json::to_string(&WsOutgoing::RaffleFinished { 
        winner_id: winner.user_id, 
        winner_first_name: winner.first_name.clone(),  
        win_amount: final_prize, 
        winning_ticket,
    })
    .unwrap_or_default();

    let _ = state.raffle_tx.send(finishged_msg);

    let _: () = redis
        .del(&[&cache_bet_key, &cache_room_key, &lock_key])
        .await
        .ok()
        .unwrap_or(());

    Ok(RaffleRoomWin {
        bets: players.clone(),
        winning_ticket,
        winning_amount: final_prize,
        winner_id: winner.user_id,
        winner_first_name: winner.first_name.clone(),
    })
}

pub fn spawn_raffle_loop(state: SharedState) {
    tokio::spawn(async move {
        loop {
            let uuid = match create_raffle_room_internal(&state).await {
                Ok(id) => id,
                Err(e) => {
                    eprint!("Raffle loop: error a create room: {}", e.1);

                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            run_raffle_round(state.clone(), uuid).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

pub async fn run_raffle_round(
    state: SharedState,
    uuid: String,
) {
    let mut redis = state.redis.clone();
    let cache_room_key = format!("raffle:{}", uuid);

    let mut timer = interval(Duration::from_secs(1));
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        timer.tick().await;

        let exists: bool = redis.exists(&cache_room_key).await.unwrap_or(false);
        if !exists {
            println!("Raffle loop: room {} closed", uuid);
            break;
        }

        let current_time: i32 = match redis.hincr(&cache_room_key, "time", -1).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Raffle loop: error Redis in timer {}: {}", uuid, e);
                break;
            }
        };

        let time_msg = serde_json::to_string(&WsOutgoing::RaffleTime { 
            seconds_left: current_time, 
        })
        .unwrap_or_default();
        let _ = state.raffle_tx.send(time_msg);

        if current_time <= 0 {
            match internal_finish_raffle(state.clone(), uuid.clone()).await {
                Ok(win) => {
                    println!(
                        "Raffle loop: winner {} - {} (amount: {})",
                        uuid, win.winner_first_name, win.winning_amount
                    );
                }
                Err(e) if !matches!(e.0, StatusCode::CONFLICT) => {
                    eprintln!("Raffle loop: error finished {}: {}", uuid, e.1);
                }
                _ => {}
            }
            break;
        }
    }
}

pub fn spawn_raffle_timer(state: SharedState, uuid: String) {
    tokio::spawn(async move {
        let mut redis = state.redis.clone();
        let cache_room_key = format!("raffle:{}", uuid);

        let mut timer = interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            timer.tick().await;

            let exists: bool = redis
                .exists(&cache_room_key)
                .await
                .unwrap_or(false);

            if !exists {
                println!("Комната {} уже закрыта, останавливаем таймер", uuid);
                break;
            }

            let current_time: i32 = match redis.hincr(&cache_room_key, "time", -1).await {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Ошибка Redis в таймере {}: {}", uuid, e);
                    break;
                }
            };

            if current_time <= 0 {
                println!("Время в комнате {} истекло", uuid);
                match internal_finish_raffle(state.clone(), uuid.clone()).await {
                    Err(e) if !matches!(e.0, StatusCode::CONFLICT) => {
                        eprintln!("Ошибка завершения раффла {}: {}", uuid, e.1);
                    }
                    _ => {}
                }
                break;
            }
        }
    });
}

pub async fn finish_raffle_game(
    State(state): State<SharedState>,
    uuid: &str,
) -> Result<Json<RaffleRoomWin>, (StatusCode, String)> {
    let res = internal_finish_raffle(state, uuid.to_string()).await?;
    Ok(Json(res))
}
