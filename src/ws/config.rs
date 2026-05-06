use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct RafflePlayer {
    pub user_id: i64,
    pub first_name: String,
    pub total_amount: i64,
}
#[derive(Serialize, Deserialize)]
pub struct RaffleRoomWin {
    pub time: i32,
    pub bets: Vec<RafflePlayer>,
    pub winning_ticket: i64,
    pub winner_id: i64,
    pub winner_first_name: String,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct RaffleRoom {
    pub time: i32,
    pub bets: Vec<RafflePlayer>,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsIncoming {
    #[serde(rename = "auth")]
    Auth { token: String },
    #[serde(rename = "ping")]
    Ping,

    // --- Raffle Config --- 

    #[serde(rename = "raffle_join")]
    RaffleJoin,
    #[serde(rename = "raffle_bet")]
    RaffleBet {
        ammount: i64,
    },
    #[serde(rename = "raffle_exit")]
    RaffleExit,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsOutgoing {
    #[serde(rename = "auth_success")]
    AuthSuccess,
    #[serde(rename = "pong")]
    Pong,

    // --- Raffle Config --- 

    #[serde(rename = "raffle_join_success")]
    RaffleJoinSuccess {
        players: Vec<RafflePlayer>,
    },
    #[serde(rename = "raffle_update")]
    RaffleUpdate {
        players: Vec<RafflePlayer>,
        total_bank: i64,
        seconds_left: i32,
    },
    #[serde(rename = "raffle_bet_success")]
    RaffleBetSuccess {
        new_balance: i64,
        players: Vec<RafflePlayer>,
    },
    #[serde(rename = "raffle_time")]
    RaffleTime { 
        seconds_left: i32 
    },
    #[serde(rename = "raffle_finished")]
    RaffleFinished {
        winner_id: i64,
        winner_first_name: String,
        winner_img: String,
        win_amount: i64,
        winning_ticket: i64,
    },
    #[serde(rename = "error")]
    Error { message: String },
}
