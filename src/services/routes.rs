use axum::{
    Json,
    extract::{State, Path},
    http::{HeaderMap, StatusCode},
};

use super::{
    config::{
        ServiceCreateReq, ServiceCreateRes, ServiceGetInfoRes, ServiceMaintenanceSwitch,
        ServiceTransferToUserReq, ServiceTransferToUserRes, ServiceSetCallbackUrlReq,
        ServiceSetCallbackUrlRes, ServiceGetHistoryRes
    },
    handlers::{
        process_create_service, process_get_info_service, process_switch_maintance_status,
        process_transfer_to_user, process_set_callback_url, process_get_history_service
    },
};
use crate::{SharedState};
use crate::helpers::extract_token;

pub async fn api_service_create_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ServiceCreateReq>,
) -> Result<Json<ServiceCreateRes>, (StatusCode, String)> {
    let token = extract_token(&headers)?;

    process_create_service(&state, token, payload).await
}

pub async fn api_service_get_info_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<ServiceGetInfoRes>, (StatusCode, String)> {
    let id = extract_token(&headers)?;

    process_get_info_service(&state, id).await
}

pub async fn api_service_transfer_to_user_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ServiceTransferToUserReq>,
) -> Result<Json<ServiceTransferToUserRes>, (StatusCode, String)> {
    let id = extract_token(&headers)?;

    process_transfer_to_user(&state, id, payload).await
}

pub async fn api_service_switch_maintance_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<ServiceMaintenanceSwitch>, (StatusCode, String)> {
    let id = extract_token(&headers)?;

    process_switch_maintance_status(&state, id).await
}

pub async fn api_service_set_callback_url(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ServiceSetCallbackUrlReq>,
) -> Result<Json<ServiceSetCallbackUrlRes>, (StatusCode, String)> {
    let token = extract_token(&headers)?;

    process_set_callback_url(&state, token, payload).await
}

pub async fn api_service_get_history(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(limit_param): Path<String>,
) -> Result<Json<ServiceGetHistoryRes>, (StatusCode, String)> {
    let uuid = extract_token(&headers)?;

    let limit = limit_param
        .trim_start_matches(':') 
        .parse::<usize>()
        .unwrap_or(15);

    process_get_history_service(&state, uuid, limit).await
}
