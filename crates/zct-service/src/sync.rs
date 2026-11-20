//! git 同步（需求 4.8）：呼叫系統的 git，沿用使用者既有的 SSH / 憑證設定
//!
//! 流程：匯出 → commit → pull（merge）→ 匯入 → push。結果寫在 `local/sync-status.txt`。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use zct_core::engine::today;
use zct_core::paths::DataRoot;
use zct_host::Host;

use zct_preferences::git::run_once;

fn write_status(data: &DataRoot, result: &Result<String, String>) {
    let text = match result {
        Ok(s) => format!("{} 成功：{s}", today()),
        Err(e) => format!("{} 失敗：{e}", today()),
    };
    let _ = std::fs::write(data.local().join("sync-status.txt"), text);
}

/// 背景同步執行緒；回傳的 Sender 用來觸發「立即同步」
pub fn spawn(data: DataRoot, host: Arc<Host>) -> std::io::Result<Sender<()>> {
    let (tx, rx): (Sender<()>, Receiver<()>) = channel();
    std::thread::Builder::new()
        .name("sync".into())
        .spawn(move || {
            let root: PathBuf = data.root().to_path_buf();
            loop {
                let cfg = host.settings().current().0.sync;
                let wait = Duration::from_secs(cfg.interval_minutes.max(1).saturating_mul(60));
                let manual = match rx.recv_timeout(wait) {
                    Ok(()) => true,
                    Err(RecvTimeoutError::Timeout) => false,
                    Err(RecvTimeoutError::Disconnected) => return,
                };
                let cfg = host.settings().current().0.sync;
                // 已安裝新版時舊版不同步，避免用舊資料覆蓋
                if (!cfg.enabled && !manual) || crate::version::read_only() {
                    continue;
                }
                // 先存檔，確保匯出的是最新的詞庫
                let engine = host.engine();
                let _ = engine.save();
                let result = run_once(&root, &cfg, &engine);
                match &result {
                    Ok(s) => log::info!("同步：{s}"),
                    Err(e) => log::warn!("同步：{e}"),
                }
                write_status(&data, &result);
            }
        })?;
    Ok(tx)
}
