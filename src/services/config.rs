use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, sqlx::Type)]
pub enum ServiceHistoryStatus {
    #[serde(rename = "to_service")]
    ToService,
    #[serde(rename = "from_service")]
    FromService,
}

#[derive(Serialize, Deserialize, Debug, Clone, sqlx::FromRow)] 
pub struct ServiceHistoryItem {
	pub id: i64,
	pub uuid: String,
    pub status: ServiceHistoryStatus,
	pub ammount: i64,
	pub date: i64
}
#[derive(Serialize, Deserialize, sqlx::FromRow, Debug, Clone)]
pub struct ServiceCreateRes {
	pub uuid: String,
	pub first_name: String,
	pub creator_id: i64,
	pub description: String,
	pub balance: i64,
	pub callback_url: String, 
  pub url_img: String,
	pub reg_date: i64,
  pub maintenance: bool,
  #[sqlx(skip)]
  pub history: Vec<ServiceHistoryItem>,
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
#[derive(Deserialize)]
pub struct ServiceTransferToUserReq {
	pub ammount: i64,
	pub reciever_id: i64,
}
#[derive(Serialize)]
pub struct ServiceTransferToUserRes {
	pub status: String,
	pub message: String,
}
#[derive(Serialize, Deserialize, sqlx::FromRow, Debug, Clone)]
pub struct ServiceMaintenanceSwitch {
    pub status: String,
    pub maintenance: bool,
}
