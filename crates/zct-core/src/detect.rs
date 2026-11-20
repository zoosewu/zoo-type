//! 偵測「不在輸入狀態」：此時按鍵全部交給 App（Shift 也不切換中英）
//!
//! 平台只收集訊號（Windows：TSF compartment、游標、螢幕；macOS：安全輸入、選取範圍、視窗），
//! 判斷規則在這裡，兩個平台共用。任一條成立即視為不在輸入狀態：
//! 1. 系統明確告知（一律啟用）：Windows 沒有焦點文件或 context、App 停用輸入法；macOS 安全輸入（密碼欄位）
//! 2. 看不出是文字輸入框 + 沒有文字游標 + 前景視窗蓋滿整個螢幕：無邊框全螢幕遊戲。
//!    必須同時蓋滿螢幕，否則 IntelliJ 等舊式介面、沒有系統游標的程式會被誤判而無法打中文
//! 3. 有程式獨佔全螢幕（只有 Windows 查得到）

/// 平台收集到的訊號
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signals {
    /// 系統明確告知不能輸入
    pub blocked: bool,
    /// 看得出是文字輸入框：Windows 支援 TSF 的 context（不是 transitory）；
    /// macOS 有選取範圍、正在組字或是終端機
    pub text_context: bool,
    pub has_caret: bool,
    /// 前景視窗蓋滿它所在的整個螢幕
    pub covers_screen: bool,
    pub exclusive_fullscreen: bool,
}

impl Default for Signals {
    /// 一般的文字輸入框
    fn default() -> Self {
        Signals {
            blocked: false,
            text_context: true,
            has_caret: true,
            covers_screen: false,
            exclusive_fullscreen: false,
        }
    }
}

/// 設定與 App 規則決定要不要做的啟發式偵測（見 [`crate::Session::heuristics`]）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Heuristics {
    /// 規則 2
    pub fullscreen_without_caret: bool,
    /// 規則 3
    pub exclusive_fullscreen: bool,
}

pub fn not_input(s: &Signals, h: Heuristics) -> bool {
    s.blocked
        || (h.fullscreen_without_caret && !s.text_context && !s.has_caret && s.covers_screen)
        || (h.exclusive_fullscreen && s.exclusive_fullscreen)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Heuristics = Heuristics {
        fullscreen_without_caret: true,
        exclusive_fullscreen: true,
    };
    const NONE: Heuristics = Heuristics {
        fullscreen_without_caret: false,
        exclusive_fullscreen: false,
    };

    #[test]
    fn normal_text_field_is_input() {
        assert!(!not_input(&Signals::default(), ALL));
    }

    #[test]
    fn system_signals_always_apply() {
        let s = Signals {
            blocked: true,
            ..Default::default()
        };
        assert!(not_input(&s, NONE));
    }

    #[test]
    fn fullscreen_game_without_caret_is_not_input() {
        let game = Signals {
            text_context: false,
            has_caret: false,
            covers_screen: true,
            ..Default::default()
        };
        assert!(not_input(&game, ALL));
        assert!(!not_input(&game, NONE), "可以用設定關掉");
        let legacy_edit = Signals {
            has_caret: true,
            ..game
        };
        assert!(
            !not_input(&legacy_edit, ALL),
            "舊式程式的輸入框有游標，照常輸入"
        );
        let editor = Signals {
            text_context: true,
            ..game
        };
        assert!(
            !not_input(&editor, ALL),
            "全螢幕的編輯器有文字 context，照常輸入"
        );
    }

    #[test]
    fn windowed_app_without_caret_is_still_input() {
        // IntelliJ 等 Java / Qt 程式：舊式介面、沒有系統游標，但不是全螢幕
        let ide = Signals {
            text_context: false,
            has_caret: false,
            ..Default::default()
        };
        assert!(!not_input(&ide, ALL));
    }

    #[test]
    fn exclusive_fullscreen_is_not_input() {
        let s = Signals {
            exclusive_fullscreen: true,
            ..Default::default()
        };
        assert!(not_input(&s, ALL));
        assert!(!not_input(&s, NONE));
    }
}
