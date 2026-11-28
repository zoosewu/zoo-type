//! 連網（更新檢查、下載、「作者的其他專案」）：只送出 GET，不帶任何使用者資料

use std::time::Duration;

/// 慢的網路也能下載完：不限總時間，連線或任何一次讀取卡住太久才放棄
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .build()
        .into()
}

pub fn request(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    agent()
        .get(url)
        .header("User-Agent", "zoo-type")
        .call()
        .map_err(|e| e.to_string())
}

pub fn get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    request(url)?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| e.to_string())
}
