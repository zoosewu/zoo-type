//! 輸入法選單（Windows 輸入法圖示的右鍵選單、macOS 輸入法選單）的內容，兩個平台共用：
//! 平台只照這份清單畫選單，按下後執行對應的 [`MenuAction`]。

use serde::Serialize;

/// 選單項目按下後要做的事
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuAction {
    InstallUpdate,
    ToggleMode,
    ToggleOutput,
    ToggleGame,
    OpenSettings,
    /// 重新啟動輸入法（重新讀設定與詞庫）
    Restart,
}

impl MenuAction {
    /// C ABI 與 IPC 用的編號
    pub fn code(self) -> u32 {
        self as u32
    }

    pub fn from_code(code: u32) -> Option<MenuAction> {
        use MenuAction::*;
        [
            InstallUpdate,
            ToggleMode,
            ToggleOutput,
            ToggleGame,
            OpenSettings,
            Restart,
        ]
        .into_iter()
        .find(|a| a.code() == code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuItem {
    Action {
        action: MenuAction,
        label: String,
        checked: bool,
        enabled: bool,
    },
    Separator,
}

/// 畫選單需要的狀態
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuState {
    pub chinese: bool,
    pub simplified: bool,
    pub game: bool,
    /// 已下載、可以安裝的新版
    pub update: Option<String>,
    /// 正在檢查或安裝更新（「更新到 vX」暫時不能按）
    pub updating: bool,
}

fn action(action: MenuAction, label: impl Into<String>, checked: bool) -> MenuItem {
    MenuItem::Action {
        action,
        label: label.into(),
        checked,
        enabled: true,
    }
}

pub fn items(state: &MenuState) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if let Some(version) = &state.update {
        items.push(MenuItem::Action {
            action: MenuAction::InstallUpdate,
            label: format!("更新到 v{}", version.trim_start_matches('v')),
            checked: false,
            enabled: !state.updating,
        });
        items.push(MenuItem::Separator);
    }
    items.push(action(MenuAction::ToggleMode, "中文模式", state.chinese));
    items.push(action(
        MenuAction::ToggleOutput,
        "簡體輸出",
        state.simplified,
    ));
    items.push(action(MenuAction::ToggleGame, "遊戲模式", state.game));
    items.push(MenuItem::Separator);
    items.push(action(MenuAction::OpenSettings, "設定…", false));
    items.push(action(
        MenuAction::Restart,
        format!("重新啟動{}", crate::channel::CURRENT.name),
        false,
    ));
    items
}

/// 輸入法圖示的提示文字
pub fn tooltip(state: &MenuState) -> String {
    let mode = match (state.chinese, state.simplified) {
        (false, _) => "英文",
        (true, true) => "中文・简体",
        (true, false) => "中文",
    };
    format!("{}：{mode}", crate::channel::CURRENT.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(state: &MenuState) -> Vec<String> {
        items(state)
            .into_iter()
            .map(|i| match i {
                MenuItem::Action { label, .. } => label,
                MenuItem::Separator => "—".into(),
            })
            .collect()
    }

    #[test]
    fn menu_lists_the_same_items_on_both_platforms() {
        let state = MenuState {
            chinese: true,
            ..Default::default()
        };
        let name = crate::channel::CURRENT.name;
        assert_eq!(
            labels(&state),
            [
                "中文模式",
                "簡體輸出",
                "遊戲模式",
                "—",
                "設定…",
                &format!("重新啟動{name}")
            ]
        );
        let MenuItem::Action { checked, .. } = &items(&state)[0] else {
            panic!()
        };
        assert!(checked, "中文模式打勾");
    }

    #[test]
    fn update_item_comes_first_and_waits_while_busy() {
        let state = MenuState {
            update: Some("0.1.6".into()),
            updating: true,
            ..Default::default()
        };
        let MenuItem::Action {
            action,
            label,
            enabled,
            ..
        } = &items(&state)[0]
        else {
            panic!()
        };
        assert_eq!(
            (*action, label.as_str(), *enabled),
            (MenuAction::InstallUpdate, "更新到 v0.1.6", false)
        );
        assert_eq!(items(&state)[1], MenuItem::Separator);
    }

    #[test]
    fn action_codes_round_trip() {
        for code in 0..6 {
            assert_eq!(MenuAction::from_code(code).unwrap().code(), code);
        }
        assert!(MenuAction::from_code(6).is_none());
    }
}
