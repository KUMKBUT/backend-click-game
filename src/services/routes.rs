use axum::{Json, extract::State, http::{HeaderMap, StatusCode}};

use crate::{SharedState};
use super::{
    handlers::{process_create_service}, 
    config::{ServiceCreateReq, ServiceCreateRes}
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