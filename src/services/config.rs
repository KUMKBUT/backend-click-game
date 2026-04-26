use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)] 
pub struct ServiceHistoryItem {
	pub id: i64,
	pub uuid: String,
    pub status: String,
	pub ammount: i64,
	pub date: i64
}
#[derive(Serialize)]
pub struct ServiceCreateRes {
	pub uuid: String,
	pub first_name: String,
	pub description: String,
	pub balance: i64,
	pub history: Vec<ServiceHistoryItem>,
	pub url_img: String,
	pub reg_date: i64,
}
#[derive(Deserialize)]
pub struct ServiceCreateReq {
	pub name: String,
	pub url_img: String,
}