//! `config.toml` 與 `apps.toml`：檔案有變更就重新載入
//!
//! 每次重新載入 `revision` 加一；session 在沒有組字時換成新設定（見 [`crate::HostSession`]）。
//! 檔案最多每 [`CHECK_EVERY`] 檢查一次修改時間，按鍵時呼叫也不會一直讀磁碟。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use zct_core::paths::DataRoot;
use zct_core::{AppRule, AppRules, Config};

use crate::Platform;

/// 兩次檢查檔案修改時間的最短間隔
pub const CHECK_EVERY: Duration = Duration::from_millis(500);

#[derive(Default, PartialEq, Clone, Copy)]
struct Stamp {
    config: Option<SystemTime>,
    apps: Option<SystemTime>,
}

struct Loaded {
    config: Config,
    apps: AppRules,
    stamp: Stamp,
    checked: Instant,
}

pub struct Settings {
    platform: Platform,
    /// （config.toml, apps.toml）；搬移資料位置時會換
    paths: Mutex<(PathBuf, PathBuf)>,
    loaded: Mutex<Option<Loaded>>,
    revision: AtomicU64,
    /// 已安裝新版、這個程序只服務舊 App 時不寫設定檔
    read_only: AtomicBool,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Settings {
    pub fn new(data: &DataRoot, platform: Platform) -> Settings {
        Settings {
            platform,
            paths: Mutex::new(paths(data)),
            loaded: Mutex::new(None),
            revision: AtomicU64::new(0),
            read_only: AtomicBool::new(false),
        }
    }

    /// 資料位置搬到別處：之後讀新位置的檔案，現有 session 在沒有組字時換上
    pub fn set_root(&self, data: &DataRoot) {
        *self.paths.lock().unwrap_or_else(|e| e.into_inner()) = paths(data);
        if let Some(l) = self
            .loaded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            // 一定和新位置的檔案不同：下面一定重新讀取（新位置沒有檔案時也是）
            l.stamp = Stamp {
                config: Some(SystemTime::UNIX_EPOCH),
                apps: Some(SystemTime::UNIX_EPOCH),
            };
            l.checked = Instant::now() - CHECK_EVERY;
        }
        self.check(true);
    }

    fn paths(&self) -> (PathBuf, PathBuf) {
        self.paths.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_read_only(&self, on: bool) {
        self.read_only.store(on, Ordering::Release);
    }

    /// 設定重新載入的次數（先檢查檔案有沒有變更）
    pub fn revision(&self) -> u64 {
        self.check(false);
        self.revision.load(Ordering::Acquire)
    }

    /// 目前的設定；檔案變更時重新讀取。格式錯誤時沿用上一份正確的設定（第一次則用預設值）
    pub fn current(&self) -> (Config, AppRules) {
        self.check(true);
        let guard = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        let l = guard.as_ref().expect("剛剛載入");
        (l.config.clone(), l.apps.clone())
    }

    /// `force`：不管上次什麼時候檢查，都看一次修改時間
    fn check(&self, force: bool) {
        let mut guard = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if !force
            && guard
                .as_ref()
                .is_some_and(|l| l.checked.elapsed() < CHECK_EVERY)
        {
            return;
        }
        let (config_path, apps_path) = self.paths();
        let stamp = Stamp {
            config: mtime(&config_path),
            apps: mtime(&apps_path),
        };
        if let Some(l) = guard.as_mut().filter(|l| l.stamp == stamp) {
            l.checked = Instant::now();
            return;
        }
        let prev = guard.take();
        let config = match Config::load(&config_path) {
            Ok(c) => c,
            Err(e) => {
                log::error!("{e}（{}）", config_path.display());
                prev.as_ref().map(|p| p.config.clone()).unwrap_or_default()
            }
        };
        let apps = match AppRules::load(&apps_path) {
            Ok(a) => a,
            Err(e) => {
                log::error!("{e}（{}）", apps_path.display());
                prev.as_ref().map(|p| p.apps.clone()).unwrap_or_default()
            }
        };
        log::info!("載入設定：{} 條 App 規則", apps.apps.len());
        if prev.is_some() {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        *guard = Some(Loaded {
            config,
            apps,
            stamp,
            checked: Instant::now(),
        });
    }

    /// 組字提示列快捷鍵：把這個 App 的設定寫進 apps.toml（沒有規則就新增一條，其他平台的欄位保留）
    pub fn set_composition_bar(&self, app: &str, on: bool) -> Result<(), String> {
        if self.read_only.load(Ordering::Acquire) {
            return Err("舊版服務程序不寫設定（已安裝新版）".into());
        }
        if app.is_empty() {
            return Err("無法辨識目前的 App".into());
        }
        let (_, mut apps) = self.current();
        let platform = self.platform;
        let same = |r: &AppRule| match platform {
            Platform::Windows => r
                .windows
                .as_deref()
                .is_some_and(|w| w.eq_ignore_ascii_case(app)),
            Platform::Macos => r.macos.as_deref() == Some(app),
        };
        match apps.apps.iter_mut().find(|r| same(r)) {
            Some(rule) => rule.composition_bar = Some(on),
            None => {
                let mut rule = AppRule {
                    composition_bar: Some(on),
                    ..Default::default()
                };
                match platform {
                    Platform::Windows => {
                        rule.name = app.trim_end_matches(".exe").to_owned();
                        rule.windows = Some(app.to_owned());
                    }
                    Platform::Macos => {
                        rule.name = app.to_owned();
                        rule.macos = Some(app.to_owned());
                    }
                }
                apps.apps.push(rule);
            }
        }
        apps.save(&self.paths().1).map_err(|e| e.to_string())
    }
}

fn paths(data: &DataRoot) -> (PathBuf, PathBuf) {
    (data.config(), data.root().join("apps.toml"))
}
