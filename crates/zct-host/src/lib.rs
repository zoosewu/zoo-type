//! 兩個平台共用的主機層
//!
//! 平台（Windows 服務程序、macOS FFI）只負責 UI 與 IO：收按鍵、畫候選窗、讀寫系統憑證、播放提示音。
//! 「怎麼處理」都在這裡：
//! - 每個程序一個 [`Host`]：資料目錄、引擎、設定熱載入、存檔與備份、git 同步、凍結與恢復、遊戲模式
//! - 每個輸入 context 一個 [`HostSession`]：AI 請求、組字提示列、中英狀態記憶、提示文字

mod ai;
pub mod notice;
mod runtime;
mod session;
pub mod settings;
pub mod update;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub use ai::{AiDisplay, AiReply, Notify};
pub use session::HostSession;
pub use settings::Settings;
use zct_core::paths::DataRoot;
use zct_core::{AppRules, Config, Engine, InputMode};
pub use zct_preferences::Platform;

/// 預設開啟組字提示列的純終端機（App 自己不顯示組字，或常被 TUI 藏起來）：
/// Windows 是執行檔名稱，macOS 是 bundle ID，不分大小寫
pub const TERMINALS: &[&str] = &[
    "WindowsTerminal.exe",
    "OpenConsole.exe",
    "conhost.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "mintty.exe",
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "dev.warp.Warp-Stable",
    "com.github.wez.wezterm",
];

pub fn is_terminal(app: &str) -> bool {
    TERMINALS.iter().any(|t| t.eq_ignore_ascii_case(app))
}

/// 這個 App 要不要顯示組字提示列：App 規則 > 純終端機 > 全域設定
pub fn composition_bar_for(app: &str, config: &Config, apps: &AppRules) -> bool {
    apps.find(app)
        .and_then(|r| r.composition_bar)
        .unwrap_or_else(|| is_terminal(app) || config.theme.composition_bar)
}

/// 平台提供的 AI 金鑰來源（例如 Windows 認證管理員）；沒有時用 [`Host::set_ai_key`] 設定的值
pub type KeySource = Box<dyn Fn() -> Option<String> + Send + Sync>;

/// 一個程序一個：所有 session 共用的引擎、設定與全域狀態
pub struct Host {
    platform: Platform,
    data: RwLock<DataRoot>,
    /// 系統詞庫資料夾（重新開啟引擎時用）
    dictionary: PathBuf,
    sync: Mutex<zct_preferences::SyncWorker>,
    /// 已安裝新版或正在更新、搬移資料：不存檔、不同步、不寫設定
    read_only: AtomicBool,
    settings: Settings,
    engine: RwLock<Arc<Engine>>,
    engine_generation: AtomicU64,
    game: AtomicBool,
    /// 每個 App（Windows：程序；macOS：bundle ID）最後的中英狀態：切到別的輸入法再切回來時沿用
    modes: Mutex<HashMap<String, InputMode>>,
    ai_key: Mutex<Option<String>>,
    ai_key_source: Option<KeySource>,
    ai_notify: Option<Arc<Notify>>,
}

impl Host {
    fn new(platform: Platform, data: DataRoot, dictionary: PathBuf, engine: Arc<Engine>) -> Host {
        Host {
            platform,
            settings: Settings::new(&data, platform),
            data: RwLock::new(data),
            dictionary,
            sync: Mutex::new(zct_preferences::SyncWorker::default()),
            read_only: AtomicBool::new(false),
            engine: RwLock::new(engine),
            engine_generation: AtomicU64::new(0),
            game: AtomicBool::new(false),
            modes: Mutex::new(HashMap::new()),
            ai_key: Mutex::new(None),
            ai_key_source: None,
            ai_notify: None,
        }
    }

    pub fn with_ai_key_source(mut self, source: KeySource) -> Host {
        self.ai_key_source = Some(source);
        self
    }

    /// AI 結果回來時通知平台（不設定的話，平台用 [`HostSession::poll_ai`] 自己取）
    pub fn with_ai_notify(mut self, notify: Notify) -> Host {
        self.ai_notify = Some(Arc::new(notify));
        self
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn engine(&self) -> Arc<Engine> {
        self.engine
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 換一個引擎（還原、更新失敗回復）；現有 session 在沒有組字時改用新引擎
    pub fn replace_engine(&self, engine: Arc<Engine>) {
        *self.engine.write().unwrap_or_else(|e| e.into_inner()) = engine;
        self.engine_generation.fetch_add(1, Ordering::AcqRel);
    }

    /// 設定或引擎變了就會改變；session 據此決定要不要重新建立
    pub fn revision(&self) -> u64 {
        self.settings
            .revision()
            .wrapping_add(self.engine_generation.load(Ordering::Acquire))
    }

    pub fn game(&self) -> bool {
        self.game.load(Ordering::Relaxed)
    }

    /// 切換全域遊戲模式，回傳切換後的狀態
    pub fn toggle_game(&self) -> bool {
        let on = !self.game.fetch_xor(true, Ordering::Relaxed);
        log::info!("{}", notice::on_off("遊戲模式", on));
        on
    }

    pub fn set_ai_key(&self, key: Option<String>) {
        *self.ai_key.lock().unwrap_or_else(|e| e.into_inner()) = key.filter(|k| !k.is_empty());
    }

    fn ai_key(&self) -> Option<String> {
        match &self.ai_key_source {
            Some(source) => source(),
            None => self
                .ai_key
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }

    fn remember_mode(&self, context: &str, mode: InputMode) {
        if context.is_empty() {
            return;
        }
        let mut modes = self.modes.lock().unwrap_or_else(|e| e.into_inner());
        // 已經結束的程序不會再用到；數量多了就整個清掉（只是少了沿用，不影響功能）
        if modes.len() > 256 {
            modes.clear();
        }
        modes.insert(context.to_owned(), mode);
    }

    fn recall_mode(&self, context: &str) -> Option<InputMode> {
        self.modes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(context)
            .copied()
    }
}
