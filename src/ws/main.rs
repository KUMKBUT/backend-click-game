use axum::{
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}},
};
use redis::AsyncCommands;
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json;

use crate::SharedState;
use crate::helpers::{get_game_user};
use crate::ws::config::{WsIncoming, WsOutgoing};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

pub async fn handle_socket(mut socket: WebSocket, state: SharedState) {
    let mut current_token: Option<String> = None;

    // --- Auth handshake ---
    while let Some(Ok(msg)) = socket.recv().await {
        if let Message::Text(text) = msg
            && let Ok(WsIncoming::Auth { token }) = serde_json::from_str::<WsIncoming>(&text)
        {
            let mut redis_conn = state.redis.clone();

            match get_game_user(&state.db, &mut redis_conn, &token).await {
                Ok(_) => {
                    let _: () = redis_conn
                        .set_ex(format!("online:{}", &token), "1", 60)
                        .await
                        .unwrap_or_default();

                    let response = serde_json::to_string(&WsOutgoing::AuthSuccess).unwrap();
                    let _ = socket.send(Message::Text(response)).await;

                    current_token = Some(token);
                    break;
                }
                _ => {
                    let error = serde_json::to_string(&WsOutgoing::Error {
                        message: "Invalid token".into(),
                    })
                    .unwrap();
                    let _ = socket.send(Message::Text(error)).await;
                }
            }
        }
    }

    let Some(token) = current_token else {
        return;
    };

    

    // --- Cleanup on disconnect ---
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut redis_conn = state.redis.clone();

    let _: () = redis_conn
        .del(format!("online:{}", &token))
        .await
        .unwrap_or_default();

    match sqlx::query("UPDATE \"user\" SET last_sync = $1 WHERE token = $2")
        .bind(now)
        .bind(&token)
        .execute(&state.db)
        .await
    {
        Ok(result) => {
            if result.rows_affected() == 0 {
                eprintln!(
                    "WS Warning: last_sync не обновлен. Пользователь с токеном {} не найден в БД",
                    &token
                );
            } else {
                println!("WS: Состояние юзера {} успешно сохранено", &token);
            }
        }
        Err(e) => {
            eprintln!(
                "WS Error: Критическая ошибка при сохранении last_sync для {}: {}",
                &token, e
            );
        }
    }
}
