use axum::{Json, http::{StatusCode}};
use redis::{AsyncCommands};
use std::{time::{SystemTime, UNIX_EPOCH}};
use sqlx;

use crate::{SharedState};
use super::{ 
    config::{ServiceCreateReq, ServiceCreateRes, ServiceGetInfoRes}
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
        callback_url
    } = payload;

	let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .as_secs() as i64;

	let uuid = format!("svc_{}_{}", creator_id.id, now);

    sqlx::query(
        "INSERT INTO service (creator_id, name, url_img, callback_url, reg_date)
         VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(creator_id.id)
    .bind(&name)
    .bind(&url_img)
    .bind(&callback_url)
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let response = ServiceCreateRes {
        uuid: uuid.clone(),
        first_name: name,
        description: "".to_string(),
        balance: 0,
        callback_url: callback_url,
        history: vec![],
        url_img,
        reg_date: chrono::Utc::now().timestamp(),
    };

    let json_data = serde_json::to_string(&response)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis
        .set_ex(
            format!("service:{}", uuid),
            json_data,
            3600,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    

    Ok(Json(response))
}

pub async fn process_get_info_service(
    state: &SharedState,
    id: &str,
) -> Result<Json<ServiceGetInfoRes>, (StatusCode, String)>{
    let mut redis = state.redis.clone();

    let key = format!("service: {}", id);

    if let Ok(Some(cached_data)) = redis.get::<_, Option<String>>(&key).await {
        if let Ok(response) = serde_json::from_str::<ServiceGetInfoRes>(&cached_data) {
            return Ok(Json(response));
        }
    };

    let service = sqlx::query_as::<_, ServiceGetInfoRes>(
        "SELECT uuid, name as first_name, description, balance, callback_url, url_img, reg_date
        FROM services WHERE uuid + $1"
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Ошибка БД: {}", e)))?
    .ok_or((StatusCode::NOT_FOUND, "Сервис не найден".into()))?;

    if let Ok(json) = serde_json::to_string(&service) {
        let _: () = redis.set_ex(&key, json, 3600).await.unwrap_or_default();
    }

    Ok(Json(service))
}