//! 狀態提示文字（兩個平台共用同一套；使用者決定 2026-11-20）

use zct_core::{InputMode, KeyResult, Output, Width};

pub const AI_NO_TYPO: &str = "AI：沒有發現錯字";
pub const AI_FAILED: &str = "AI 修正失敗，請檢查連線設定";
pub const AI_START_FAILED: &str = "無法啟動 AI 修正";

pub fn on_off(label: &str, on: bool) -> String {
    format!("{label}：{}", if on { "開" } else { "關" })
}

/// 一次按鍵的提示；同時發生多個變化時只顯示最重要的一個：
/// 遊戲模式 > session 自己的提示 > 中英 > 全半形 > 繁簡
pub fn hint(r: &KeyResult, game: Option<bool>) -> Option<String> {
    if let Some(on) = game {
        return Some(on_off("遊戲模式", on));
    }
    if let Some(n) = &r.notice {
        return Some(n.clone());
    }
    if let Some(m) = r.mode_changed {
        return Some(
            match m {
                InputMode::Chinese => "中",
                InputMode::English => "A",
            }
            .into(),
        );
    }
    if let Some(w) = r.width_changed {
        return Some(
            match w {
                Width::Full => "全形",
                Width::Half => "半形",
            }
            .into(),
        );
    }
    r.output_changed.map(|o| {
        match o {
            Output::Traditional => "動",
            Output::Simplified => "动",
        }
        .into()
    })
}
