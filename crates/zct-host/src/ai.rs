//! AI 同音字修正的背景請求
//!
//! 每個請求帶著 session 的「代數」：按下其他鍵後代數改變，舊請求的結果直接丟掉，
//! 不會在使用者已經改了內容之後才跳出來。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};

use zct_core::Engine;

/// （原文, 修正結果或錯誤）
pub type AiReply = (String, Result<String, String>);

/// 結果回來時要立即顯示的內容（給會主動更新畫面的平台，例如 Windows 候選窗）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiDisplay {
    /// 修正後的整句，按 Enter 或 1 採用
    Suggestion(String),
    Notice(String),
}

/// 結果回來時通知平台：（session ID, 要顯示的內容）
pub type Notify = Box<dyn Fn(u64, AiDisplay) + Send + Sync>;

pub(crate) struct Request {
    pub(crate) endpoint: zct_ai::Endpoint,
    pub(crate) engine: Arc<Engine>,
    pub(crate) original: String,
    pub(crate) syllables: Vec<Option<String>>,
    pub(crate) generation: Arc<AtomicU64>,
    pub(crate) id: u64,
}

pub(crate) fn spawn(
    req: Request,
    notify: Option<Arc<Notify>>,
) -> Result<Receiver<AiReply>, String> {
    let (send, receive) = mpsc::channel();
    let mine = req.generation.load(Ordering::Acquire);
    std::thread::Builder::new()
        .name("ime-ai".into())
        .spawn(move || {
            let result = zct_ai::correct(&req.endpoint, &req.original)
                .map(|fixed| {
                    req.engine
                        .keep_same_reading(&req.original, &req.syllables, &fixed)
                })
                .map_err(|e| e.to_string());
            if req.generation.load(Ordering::Acquire) != mine {
                return;
            }
            if let Err(e) = &result {
                log::warn!("[{}] AI：{e}", req.id);
            }
            if let Some(notify) = notify {
                notify(
                    req.id,
                    match &result {
                        Ok(fixed) if *fixed != req.original => AiDisplay::Suggestion(fixed.clone()),
                        Ok(_) => AiDisplay::Notice(crate::notice::AI_NO_TYPO.into()),
                        Err(_) => AiDisplay::Notice(crate::notice::AI_FAILED.into()),
                    },
                );
            }
            let _ = send.send((req.original, result));
        })
        .map_err(|e| e.to_string())?;
    Ok(receive)
}
