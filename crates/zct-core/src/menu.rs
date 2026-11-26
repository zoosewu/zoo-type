//! 輸入法選單（Windows 輸入法圖示的右鍵選單、macOS 輸入法選單）的內容，兩個平台共用：
//! 平台只照這份清單畫選單，按下後執行對應的 [`MenuAction`]。

use serde::{Deserialize, Serialize};

/// 選單項目按下後要做的事
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuAction {
    InstallUpdate,
    /// 立即檢查更新（在背景查；有新版時選單換成「更新到 vX」）
    CheckUpdate,
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
            CheckUpdate,
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

/// 更新走到哪一步：選單、圖示提示與設定畫面都照這個顯示
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateState {
    /// 這個版本不管更新（開發版、ZCT_HOME 隔離測試）
    #[default]
    Off,
    Idle,
    /// 上次檢查：已是最新版
    Latest,
    /// 上次檢查失敗（原因）
    Failed(String),
    Checking,
    /// 下載新版中：已下載的百分比
    Downloading(u8),
    /// 已下載、驗證過，等使用者按「更新到 vX」
    Ready(String),
    Installing(String),
    /// 新版已經裝好；這個 App 還載著舊版，重新開啟才會換新
    Installed(String),
}

impl UpdateState {
    /// 選單項目：（動作, 文字, 能不能按）
    fn item(&self) -> Option<(MenuAction, String, bool)> {
        use UpdateState::*;
        Some(match self {
            Off => return None,
            Idle | Latest | Failed(_) => (MenuAction::CheckUpdate, "檢查更新".into(), true),
            Ready(v) => (
                MenuAction::InstallUpdate,
                format!("更新到 v{}", bare(v)),
                true,
            ),
            _ => (MenuAction::CheckUpdate, self.text()?, false),
        })
    }

    /// 進行中或有結果時的說明（圖示提示、設定畫面）；沒事可說是 `None`
    pub fn text(&self) -> Option<String> {
        use UpdateState::*;
        Some(match self {
            Off | Idle => return None,
            Latest => "已是最新版".into(),
            Failed(e) => format!("檢查更新失敗：{e}"),
            Checking => "檢查更新中…".into(),
            Downloading(p) => format!("下載更新 {p}%"),
            Ready(v) => format!("可以更新到 v{}", bare(v)),
            Installing(v) => format!("正在安裝 v{}…", bare(v)),
            Installed(v) => format!("已更新到 v{}，重新開啟這個 App 後生效", bare(v)),
        })
    }

    /// 還在進行（檢查、下載、安裝）：設定畫面持續更新狀態
    pub fn busy(&self) -> bool {
        matches!(
            self,
            UpdateState::Checking | UpdateState::Downloading(_) | UpdateState::Installing(_)
        )
    }
}

fn bare(version: &str) -> &str {
    version.trim_start_matches('v')
}

/// 畫選單需要的狀態
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuState {
    pub chinese: bool,
    pub simplified: bool,
    pub game: bool,
    pub update: UpdateState,
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
    if let Some((action, label, enabled)) = state.update.item() {
        items.push(MenuItem::Action {
            action,
            label,
            checked: false,
            enabled,
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

/// 輸入法圖示的提示文字；更新有進展時接在後面（滑過圖示就看得到）
pub fn tooltip(state: &MenuState) -> String {
    let mode = match (state.chinese, state.simplified) {
        (false, _) => "英文",
        (true, true) => "中文・简体",
        (true, false) => "中文",
    };
    let name = crate::channel::CURRENT.name;
    let update = match &state.update {
        UpdateState::Latest | UpdateState::Failed(_) => None,
        u => u.text(),
    };
    match update {
        Some(update) => format!("{name}：{mode}　{update}"),
        None => format!("{name}：{mode}"),
    }
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

    fn first(update: UpdateState) -> (MenuAction, String, bool) {
        let state = MenuState {
            update,
            ..Default::default()
        };
        match items(&state).into_iter().next().unwrap() {
            MenuItem::Action {
                action,
                label,
                enabled,
                ..
            } => (action, label, enabled),
            MenuItem::Separator => panic!(),
        }
    }

    #[test]
    fn update_item_shows_each_step_first_in_the_menu() {
        use UpdateState::*;
        let check = MenuAction::CheckUpdate;
        assert_eq!(first(Idle), (check, "檢查更新".into(), true));
        assert_eq!(first(Latest), (check, "檢查更新".into(), true));
        assert_eq!(
            first(Failed("離線".into())),
            (check, "檢查更新".into(), true)
        );
        assert_eq!(first(Checking), (check, "檢查更新中…".into(), false));
        assert_eq!(
            first(Downloading(42)),
            (check, "下載更新 42%".into(), false)
        );
        assert_eq!(
            first(Ready("0.1.6".into())),
            (MenuAction::InstallUpdate, "更新到 v0.1.6".into(), true)
        );
        assert_eq!(
            first(Installing("v0.1.6".into())),
            (check, "正在安裝 v0.1.6…".into(), false)
        );
        assert_eq!(
            first(Installed("0.1.6".into())),
            (
                check,
                "已更新到 v0.1.6，重新開啟這個 App 後生效".into(),
                false
            )
        );
        // 開發版沒有更新項目
        assert!(items(&MenuState::default()).iter().all(|i| !matches!(
            i,
            MenuItem::Action {
                action: MenuAction::CheckUpdate | MenuAction::InstallUpdate,
                ..
            }
        )));
    }

    #[test]
    fn tooltip_tells_how_the_update_is_going() {
        let name = crate::channel::CURRENT.name;
        let mut state = MenuState {
            chinese: true,
            update: UpdateState::Idle,
            ..Default::default()
        };
        assert_eq!(tooltip(&state), format!("{name}：中文"));
        state.update = UpdateState::Latest;
        assert_eq!(
            tooltip(&state),
            format!("{name}：中文"),
            "檢查的結果只在設定畫面顯示"
        );
        state.update = UpdateState::Installing("0.1.6".into());
        assert_eq!(tooltip(&state), format!("{name}：中文　正在安裝 v0.1.6…"));
    }

    #[test]
    fn action_codes_round_trip() {
        for code in 0..7 {
            assert_eq!(MenuAction::from_code(code).unwrap().code(), code);
        }
        assert!(MenuAction::from_code(7).is_none());
    }
}
