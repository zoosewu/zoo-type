//! 輸入法選單（Windows 輸入法圖示的右鍵選單、macOS 輸入法選單）的內容，兩個平台共用：
//! 平台只照這份清單畫選單，按下後執行對應的 [`MenuAction`]。

use serde::{Deserialize, Serialize};

/// 選單項目按下後要做的事
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuAction {
    /// 開啟設定的「關於」頁（有新版本、下載中、更新失敗時）
    OpenAbout,
    /// 開啟「關於」頁並立即檢查更新
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
            OpenAbout,
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

/// GitHub repo（Release 頁面、原始碼、回報問題）
pub const REPO_URL: &str = "https://github.com/zoosewu/zoo-type";

/// 更新走到哪一步：選單、圖示提示與設定的「關於」頁都照這個顯示
/// （規範：zoo-release 的 docs/UPDATE.md、docs/UI.md；用詞照 UI.md 第 3 節）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateState {
    /// 這個版本不管更新（開發版、ZCT_HOME 隔離測試）
    #[default]
    Off,
    /// 還沒檢查過
    Idle,
    Checking,
    /// 已是最新版；`checked` 是上次成功檢查的時間（「今天 14:31」）
    Latest {
        checked: Option<String>,
    },
    /// 手動檢查失敗（自動檢查失敗不顯示）
    CheckFailed,
    /// 有新版本；`feature`：主版或次版較新、沒有略過 → 選單也提示（修補版只在「關於」頁顯示）
    Available {
        version: String,
        feature: bool,
    },
    Downloading {
        version: String,
        percent: u8,
    },
    /// 安裝程式在背景執行
    Installing(String),
    /// 下載或驗證失敗（原因）
    Failed(String),
    /// 新版已經裝好；這個 App 還載著舊版，重新開啟才會換新
    Installed(String),
}

/// 「關於」頁狀態列旁的按鈕
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateButton {
    /// 開啟這個版本的 Release 頁面
    Notes,
    /// 下載並安裝
    Install,
    Cancel,
    /// 再檢查一次
    Retry,
    /// 開啟 Release 頁面手動下載
    Manual,
}

impl UpdateButton {
    pub fn label(self) -> &'static str {
        match self {
            UpdateButton::Notes => "版本說明",
            UpdateButton::Install => "更新",
            UpdateButton::Cancel => "取消",
            UpdateButton::Retry => "重試",
            UpdateButton::Manual => "手動下載",
        }
    }
}

impl UpdateState {
    /// 選單項目：（動作, 文字, 能不能按）
    fn item(&self) -> Option<(MenuAction, String, bool)> {
        use UpdateState::*;
        let about = MenuAction::OpenAbout;
        Some(match self {
            Off => return None,
            Checking => (MenuAction::CheckUpdate, "檢查中…".into(), false),
            Available {
                feature: true,
                version,
            } => (about, format!("有新版本 {}", tagged(version)), true),
            Downloading { .. } | Failed(_) => (about, self.text()?, true),
            Installing(_) | Installed(_) => (about, self.text()?, false),
            Idle | Latest { .. } | CheckFailed | Available { .. } => {
                (MenuAction::CheckUpdate, "檢查更新".into(), true)
            }
        })
    }

    /// 「關於」頁的狀態列；沒事可說是 `None`
    pub fn text(&self) -> Option<String> {
        use UpdateState::*;
        Some(match self {
            Off | Idle => return None,
            Checking => "檢查中…".into(),
            Latest { checked: None } => "已是最新版".into(),
            Latest { checked: Some(t) } => format!("已是最新版（上次檢查：{t}）"),
            CheckFailed => "無法檢查更新".into(),
            Available { version, .. } => format!("有新版本 {}", tagged(version)),
            Downloading { version, percent } => {
                format!("正在下載 {}　{percent}%", tagged(version))
            }
            Installing(v) => format!("正在安裝 {}…", tagged(v)),
            Failed(reason) => format!("更新失敗：{reason}"),
            Installed(v) => format!("已更新到 {}，重新開啟這個 App 後生效", tagged(v)),
        })
    }

    /// 狀態列旁的按鈕
    pub fn buttons(&self) -> &'static [UpdateButton] {
        use UpdateButton::*;
        match self {
            UpdateState::Available { .. } => &[Notes, Install],
            UpdateState::Downloading { .. } => &[Cancel],
            UpdateState::CheckFailed => &[Retry],
            UpdateState::Failed(_) => &[Manual],
            _ => &[],
        }
    }

    /// 「版本說明」「手動下載」開啟的頁面
    pub fn release_page(&self) -> String {
        match self {
            UpdateState::Available { version, .. } | UpdateState::Downloading { version, .. } => {
                format!("{REPO_URL}/releases/tag/{}", tagged(version))
            }
            _ => format!("{REPO_URL}/releases"),
        }
    }

    /// 還在進行（檢查、下載、安裝）：「關於」頁持續更新狀態、「檢查更新」停用
    pub fn busy(&self) -> bool {
        matches!(
            self,
            UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Installing(_)
        )
    }
}

/// 版號一律帶 v（「v0.7.0」）
fn tagged(version: &str) -> String {
    format!("v{}", version.trim_start_matches('v'))
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
    // 選單上有提示的才接在圖示提示後面
    let update = match &state.update {
        UpdateState::Available { feature: true, .. }
        | UpdateState::Downloading { .. }
        | UpdateState::Installing(_)
        | UpdateState::Installed(_) => state.update.text(),
        _ => None,
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
    fn update_item_follows_the_zoo_release_wording() {
        use UpdateState::*;
        let (check, about) = (MenuAction::CheckUpdate, MenuAction::OpenAbout);
        assert_eq!(first(Idle), (check, "檢查更新".into(), true));
        assert_eq!(first(Checking), (check, "檢查中…".into(), false));
        assert_eq!(
            first(Latest { checked: None }),
            (check, "檢查更新".into(), true)
        );
        assert_eq!(first(CheckFailed), (check, "檢查更新".into(), true));
        let available = |feature| Available {
            version: "0.2.0".into(),
            feature,
        };
        assert_eq!(
            first(available(true)),
            (about, "有新版本 v0.2.0".into(), true),
            "主版或次版較新：選單提示，點了開啟「關於」頁"
        );
        assert_eq!(
            first(available(false)),
            (check, "檢查更新".into(), true),
            "修補版只在「關於」頁顯示"
        );
        assert_eq!(
            first(Downloading {
                version: "0.2.0".into(),
                percent: 68
            }),
            (about, "正在下載 v0.2.0　68%".into(), true)
        );
        assert_eq!(
            first(Installing("0.2.0".into())),
            (about, "正在安裝 v0.2.0…".into(), false)
        );
        assert_eq!(
            first(Failed("簽章驗證失敗".into())),
            (about, "更新失敗：簽章驗證失敗".into(), true)
        );
        assert_eq!(
            first(Installed("0.2.0".into())),
            (
                about,
                "已更新到 v0.2.0，重新開啟這個 App 後生效".into(),
                false
            )
        );
        // 開發版沒有更新項目
        assert!(items(&MenuState::default()).iter().all(|i| !matches!(
            i,
            MenuItem::Action {
                action: MenuAction::CheckUpdate | MenuAction::OpenAbout,
                ..
            }
        )));
    }

    #[test]
    fn about_page_status_and_buttons() {
        use UpdateButton::*;
        use UpdateState::*;
        let checked = Latest {
            checked: Some("今天 14:31".into()),
        };
        assert_eq!(
            checked.text().unwrap(),
            "已是最新版（上次檢查：今天 14:31）"
        );
        assert_eq!(CheckFailed.text().unwrap(), "無法檢查更新");
        assert_eq!(CheckFailed.buttons(), [Retry]);
        let available = Available {
            version: "0.1.11".into(),
            feature: false,
        };
        assert_eq!(available.text().unwrap(), "有新版本 v0.1.11");
        assert_eq!(available.buttons(), [Notes, Install]);
        assert_eq!(
            available.release_page(),
            "https://github.com/zoosewu/zoo-type/releases/tag/v0.1.11"
        );
        let failed = Failed("無法連線".into());
        assert_eq!(failed.buttons(), [Manual]);
        assert_eq!(
            failed.release_page(),
            "https://github.com/zoosewu/zoo-type/releases"
        );
        assert!(Checking.busy() && !available.busy());
        let labels: Vec<_> = [Notes, Install, Cancel, Retry, Manual]
            .iter()
            .map(|b| b.label())
            .collect();
        assert_eq!(labels, ["版本說明", "更新", "取消", "重試", "手動下載"]);
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
        state.update = UpdateState::Latest { checked: None };
        assert_eq!(
            tooltip(&state),
            format!("{name}：中文"),
            "檢查的結果只在「關於」頁顯示"
        );
        state.update = UpdateState::Available {
            version: "0.2.0".into(),
            feature: true,
        };
        assert_eq!(tooltip(&state), format!("{name}：中文　有新版本 v0.2.0"));
    }

    #[test]
    fn action_codes_round_trip() {
        for code in 0..7 {
            assert_eq!(MenuAction::from_code(code).unwrap().code(), code);
        }
        assert!(MenuAction::from_code(7).is_none());
    }
}
