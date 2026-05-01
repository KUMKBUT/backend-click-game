use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};

use super::{
    config::{
        ServiceCreateReq, ServiceCreateRes, ServiceGetInfoRes, ServiceMaintenanceSwitch,
        ServiceTransferToUserReq, ServiceTransferToUserRes,
    },
    handlers::{
        process_create_service, process_get_info_service, process_switch_maintance_status,
        process_transfer_to_user,
    },
};
use crate::SharedState;
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
