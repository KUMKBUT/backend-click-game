use axum::{Json, http::{HeaderMap, StatusCode, header}, extract::{State}};
use redis::{aio::ConnectionManager, AsyncCommands};
use std::{time::{SystemTime, UNIX_EPOCH}};
use sqlx::postgres::{PgPool};

use crate::{SharedState};
use super::{ 
    config::{ServiceCreateReq, ServiceCreateRes}
};
use crate::{helpers::get_game_user};

// --- Создание сервиса ---
pub async fn process_create_service(
    state: &SharedState,
    token: &str,
    payload: ServiceCreateReq,
) -> Result<Json<ServiceCreateRes>, (StatusCode, String)> {
    let mut redis = state.redis.clone();

    let creator_id = get_game_user(&state.db, &mut redis, token)
        .await?
        .ok_or((StatusCode::UNAUTHORIZED, "Создатель не найден".into()))?;

    let ServiceCreateReq {
        name,
        url_img,
    } = payload;

	let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .as_secs() as i64;

	let uuid = format!("svc_{}_{}", creator_id.id, now);

    sqlx::query(
        "INSERT INTO service (creator_id, name, url_img, balance, reg_date)
         VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(creator_id.id)
    .bind(&name)
    .bind(&url_img)
    .bind(0_i64)
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis
        .set_ex(
            format!("service:{}", uuid),
            name.clone(),
            3600,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let response = ServiceCreateRes {
        uuid,
        first_name: name,
        description: "".to_string(),
        balance: 0,
        history: vec![],
        url_img,
        reg_date: chrono::Utc::now().timestamp(),
    };

    Ok(Json(response))
}