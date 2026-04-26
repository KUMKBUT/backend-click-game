use axum::{Json, extract::{State, Path}, http::{HeaderMap, StatusCode}};

use crate::{SharedState};
use super::{
    handlers::{process_create_service, process_get_info_service}, 
    config::{ServiceCreateReq, ServiceCreateRes, ServiceGetInfoRes}
};
use crate::helpers::extract_token;

pub async fn api_service_create_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ServiceCreateReq>,
) -> Result<Json<ServiceCreateRes>,( StatusCode, String)> {
    let token = extract_token(&headers)?;

    process_create_service(&state, token, payload).await
}

pub async fn api_service_get_info_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<ServiceGetInfoRes>,( StatusCode, String)> {
    process_get_info_service(&state, &id).await
}
