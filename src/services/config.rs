use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServiceHistoryStatus {
    #[serde(rename = "to_service")]
    ToService,
    #[serde(rename = "from_service")]
    FromService,
}

#[derive(Serialize, Deserialize)] 
pub struct ServiceHistoryItem {
	pub id: i64,
	pub uuid: String,
    pub status: ServiceHistoryStatus,
	pub ammount: i64,
	pub date: i64
}
#[derive(Serialize)]
pub struct ServiceCreateRes {
	pub uuid: String,
	pub first_name: String,
	pub description: String,
	pub balance: i64,
	pub callback_url: String,
	pub history: Vec<ServiceHistoryItem>,
	pub url_img: String,
	pub reg_date: i64,
}
#[derive(Deserialize)]
pub struct ServiceCreateReq {
	pub name: String,
	pub url_img: String,
	pub callback_url: String,
}
#[derive(Serialize, Deserialize, sqlx::FromRow, Debug, Clone)]
pub struct ServiceGetInfoRes {
	pub uuid: String,
	pub first_name: String,
	pub description: String,
	pub balance: i64,
	pub callback_url: String,
	pub url_img: String,
	pub reg_date: i64,
}