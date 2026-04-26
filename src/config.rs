use serde::{Deserialize, Serialize};

pub fn get_upgrade_config(id: &str) -> Option<UpgradeInfo> {
    match id {
        // --- КАТЕГОРИЯ: GPU ---
        "gpu_1" => Some(UpgradeInfo { id: "gpu_1", name: "GT 710", upgrade_type: "auto", base_price: 16, base_power: 1, price_growth: 1.6 }),
        "gpu_2" => Some(UpgradeInfo { id: "gpu_2", name: "GTX 1050 Ti", upgrade_type: "auto", base_price: 256, base_power: 8, price_growth: 1.6 }),
        "gpu_3" => Some(UpgradeInfo { id: "gpu_3", name: "RTX 2060 Super", upgrade_type: "auto", base_price: 4096, base_power: 64, price_growth: 1.6 }),
        "gpu_4" => Some(UpgradeInfo { id: "gpu_4", name: "RTX 4070 Ti", upgrade_type: "auto", base_price: 65536, base_power: 512, price_growth: 1.6 }),
        "gpu_5" => Some(UpgradeInfo { id: "gpu_5", name: "RTX 4090 OC", upgrade_type: "auto", base_price: 1048576, base_power: 4096, price_growth: 1.6 }),
        "gpu_6" => Some(UpgradeInfo { id: "gpu_6", name: "Antminer S21 Pro", upgrade_type: "auto", base_price: 16777216, base_power: 32768, price_growth: 1.6 }),
        "gpu_7" => Some(UpgradeInfo { id: "gpu_7", name: "RTX 5090 Ti Prototype", upgrade_type: "auto", base_price: 268435456, base_power: 262144, price_growth: 1.6 }),

        // --- КАТЕГОРИЯ: CPU ---
        "cpu_1" => Some(UpgradeInfo { id: "cpu_1", name: "Intel Celeron", upgrade_type: "auto", base_price: 32, base_power: 1, price_growth: 1.6 }),
        "cpu_2" => Some(UpgradeInfo { id: "cpu_2", name: "Core i3-10100", upgrade_type: "auto", base_price: 512, base_power: 8, price_growth: 1.6 }),
        "cpu_3" => Some(UpgradeInfo { id: "cpu_3", name: "Core i7-13700K", upgrade_type: "auto", base_price: 8192, base_power: 64, price_growth: 1.6 }),
        "cpu_4" => Some(UpgradeInfo { id: "cpu_4", name: "Ryzen 9 7950X", upgrade_type: "auto", base_price: 131072, base_power: 512, price_growth: 1.6 }),
        "cpu_5" => Some(UpgradeInfo { id: "cpu_5", name: "Threadripper 3990X", upgrade_type: "auto", base_price: 2097152, base_power: 4096, price_growth: 1.6 }),
        "cpu_6" => Some(UpgradeInfo { id: "cpu_6", name: "Quantum Cluster", upgrade_type: "auto", base_price: 33554432, base_power: 32768, price_growth: 1.6 }),
        "cpu_7" => Some(UpgradeInfo { id: "cpu_7", name: "EPYC 9654 Farm", upgrade_type: "auto", base_price: 536870912, base_power: 262144, price_growth: 1.6 }),

        // --- КАТЕГОРИЯ: MOUSE ---
        "mouse_1" => Some(UpgradeInfo { id: "mouse_1", name: "Офисная мышь", upgrade_type: "click", base_price: 64, base_power: 1, price_growth: 1.6 }),
        "mouse_2" => Some(UpgradeInfo { id: "mouse_2", name: "Игровая X7", upgrade_type: "click", base_price: 1024, base_power: 8, price_growth: 1.6 }),
        "mouse_3" => Some(UpgradeInfo { id: "mouse_3", name: "Logitech G502", upgrade_type: "click", base_price: 16384, base_power: 64, price_growth: 1.6 }),
        "mouse_4" => Some(UpgradeInfo { id: "mouse_4", name: "Razer DeathAdder", upgrade_type: "click", base_price: 262144, base_power: 512, price_growth: 1.6 }),
        "mouse_5" => Some(UpgradeInfo { id: "mouse_5", name: "Custom Clicker v2", upgrade_type: "click", base_price: 4194304, base_power: 4096, price_growth: 1.6 }),
        "mouse_6" => Some(UpgradeInfo { id: "mouse_6", name: "Neural Link", upgrade_type: "click", base_price: 67108864, base_power: 32768, price_growth: 1.6 }),
        "mouse_7" => Some(UpgradeInfo { id: "mouse_7", name: "Telepathic Command", upgrade_type: "click", base_price: 173741824, base_power: 262144, price_growth: 1.6 }),

        _ => None,
    }
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, Clone)]
pub struct GameUser {
    pub id: i64,
    pub token: String,
    pub first_name: String,
    pub per_click: i64,
    pub auto_click: i64,
    pub balance: i64,
    pub last_sync: i64,
}

#[derive(Deserialize)]
pub struct ClickPayload {
    pub data: i64,
}

#[derive(Serialize)]
pub struct SyncResponse {
    pub balance: i64,
    pub last_sync: i64,
}

#[derive(Deserialize)]
pub struct BuyUpgradePayload {
    pub upgrade_id: String,
}

#[derive(Serialize, Clone)]
pub struct UpgradeInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub upgrade_type: &'static str,
    pub base_price: i64,
    pub base_power: i64,
    pub price_growth: f64,
}
#[derive(Serialize, Deserialize)] 
pub struct TopUserItem {
    pub id: i64,
    pub first_name: String,
    pub per_click: i64,
    pub auto_click: i64,
    pub balance: i64,
}

#[derive(Serialize, Deserialize)]
pub struct TopUsers {
    pub users: Vec<TopUserItem>,
}
#[derive(Deserialize)]
pub struct TransferReq {
    pub ammount: i64,
    pub recipient: i64,
}
#[derive(Serialize)]
pub struct TransferRes {
    pub status: String,
    pub new_balance: i64,
}
// Ws config 
#[derive(Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsIncoming {
    #[serde(rename = "auth")]
    Auth { token: String },
    #[serde(rename = "ping")]
    Ping,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsOutgoing {
    #[serde(rename = "auth_success")]
    AuthSuccess,
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "error")]
    Error { message: String },
}
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