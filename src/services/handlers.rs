use axum::{Json, http::StatusCode};
use redis::{AsyncCommands};
use std::{time::{SystemTime, UNIX_EPOCH}};
use sqlx::Row;

use crate::{SharedState};
use super::{ 
    config::{ServiceCreateReq, ServiceCreateRes, ServiceGetInfoRes, ServiceTransferToUserReq, ServiceTransferToUserRes, ServiceMaintenanceSwitch}
};
use crate::{helpers::{get_game_user, get_game_user_id}};

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
        creator_id: creator_id.id,
        description: "".to_string(),
        balance: 0,
        callback_url,
        history: vec![],
        url_img,
        reg_date: chrono::Utc::now().timestamp(),
        maintenance: true,
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

    let key = format!("service:{}", id);

    if let Ok(Some(cached_data)) = redis.get::<_, Option<String>>(&key).await
        && let Ok(response) = serde_json::from_str::<ServiceGetInfoRes>(&cached_data) 
    {
        return Ok(Json(response));
    }

    let service = sqlx::query_as::<_, ServiceGetInfoRes>(
        "SELECT uuid, name as first_name, description, balance, callback_url, url_img, reg_date
        FROM services WHERE uuid = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Ошибка БД: {}", e)))?
    .ok_or((StatusCode::NOT_FOUND, "Сервис не найден".into()))?;

    if let Ok(json) = serde_json::to_string(&service) {
        let _: () = redis.set_ex(&key, json, 3600).await.unwrap_or_default();
    }

    Ok(Json(service))
}

pub async fn process_transfer_to_user(
    state: &SharedState,
    id: &str,
    payload: ServiceTransferToUserReq,
) -> Result<Json<ServiceTransferToUserRes>, (StatusCode, String)>{
    let mut redis = state.redis.clone();
    
    if payload.ammount <= 0 {
        return Err((StatusCode::BAD_REQUEST, "Сумма должна быть больше 0!".into()));
    }

    let receiver = get_game_user_id(&state.db, &mut redis, payload.reciever_id)
        .await?
        .ok_or((StatusCode::UNAUTHORIZED, "Получатель не найден.".into()))?;

    let mut tx = state.db.begin().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let service_row = sqlx::query("SELECT balance FROM service WHERE uuid = $1 FOR UPDATE")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Ошибка блокировки сервиса: {}", e)))?;

    let current_service_balance: i64 = service_row.get("balance");

    if current_service_balance < payload.ammount {
        return Err((StatusCode::PAYMENT_REQUIRED, "На балансе сервиса недостаточно средств!".into()));
    }

    let _ = sqlx::query(r#"SELECT id FROM "user" WHERE id = $1 FOR UPDATE"#)
        .bind(receiver.id) 
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Ошибка блокировки получателя: {}", e)))?;

    sqlx::query("UPDATE service SET balance = balance - $1 WHERE uuid = $2")
        .bind(payload.ammount)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query(r#"UPDATE "user" SET balance = balance + $1 WHERE id = $2"#)
        .bind(payload.ammount)
        .bind(receiver.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tx.commit().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis.del(format!("service:{}", id)).await.unwrap_or_default();
    let _: () = redis.del(format!("user_id:{}", receiver.id)).await.unwrap_or_default();
    let _: () = redis.del(format!("user:{}", receiver.token)).await.unwrap_or_default();
    
    let response = ServiceTransferToUserRes{
        status: "success".to_string(),
        message: format!("Успешно доставлено: {} пользователю: {}", payload.ammount, receiver.first_name)
    };

    Ok(Json(response))
}
pub async fn process_switch_maintance_status(
    state: &SharedState,
    id: &str,
) -> Result<Json<ServiceMaintenanceSwitch>, (StatusCode, String)>{
    let mut redis = state.redis.clone();

    let cache_key = format!("service:{}", id);

    if let Ok(Some(cached_data)) = redis.get::<_, Option<String>>(&cache_key).await
        && let Ok(response) = serde_json::from_str::<ServiceMaintenanceSwitch>(&cached_data) 
    {
        return Ok(Json(response));
    }

    let service = sqlx::query_as::<_, ServiceCreateRes>(
        r#"
        UPDATE service
        SET maintenance = NOT maintenance
        WHERE uuid = $1
        RETURNING uuid, url_img, name AS first_name, description, balance, callback_url, reg_date, maintenance, creator_id
        "#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or((StatusCode::NOT_FOUND, "Service not found".into()))?;

    let json_data = serde_json::to_string(&service)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _: () = redis
        .set_ex(&cache_key, json_data, 3600)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ServiceMaintenanceSwitch {
        status: "success".into(),
        maintenance: service.maintenance,
    }))
}
