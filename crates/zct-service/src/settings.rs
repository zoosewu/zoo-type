//! `config.toml` 與 `apps.toml`：新連線時若檔案有變更就重新載入
//!
//! 已建立的 session 沿用舊設定；切換到 App（建立新連線）或重開 App 後生效。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use zct_core::paths::DataRoot;
use zct_core::{AppRules, Config};

#[derive(Default, PartialEq, Clone, Copy)]
struct Stamp {
    config: Option<SystemTime>,
    apps: Option<SystemTime>,
}

struct Loaded {
    config: Config,
    apps: AppRules,
    stamp: Stamp,
}

pub struct Settings {
    config_path: PathBuf,
    apps_path: PathBuf,
    loaded: Mutex<Option<Loaded>>,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Settings {
    pub fn new(data: &DataRoot) -> Settings {
        Settings {
            config_path: data.config(),
            apps_path: data.root().join("apps.toml"),
            loaded: Mutex::new(None),
        }
    }

    /// 目前的設定；檔案變更時重新讀取。格式錯誤時沿用上一份正確的設定（第一次則用預設值）
    /// 組字提示列快捷鍵：把這個 App 的設定寫進 apps.toml（沒有規則就新增一條）
    pub fn set_composition_bar(&self, exe: &str, on: bool) -> Result<(), String> {
        let (_, mut apps) = self.current();
        match apps
            .apps
            .iter_mut()
            .find(|r| r.windows.as_deref().is_some_and(|w| w.eq_ignore_ascii_case(exe)))
        {
            Some(rule) => rule.composition_bar = Some(on),
            None => apps.apps.push(zct_core::AppRule {
                name: exe.trim_end_matches(".exe").to_owned(),
                windows: Some(exe.to_owned()),
                composition_bar: Some(on),
                ..Default::default()
            }),
        }
        apps.save(&self.apps_path).map_err(|e| e.to_string())
    }

    pub fn current(&self) -> (Config, AppRules) {
        let stamp = Stamp {
            config: mtime(&self.config_path),
            apps: mtime(&self.apps_path),
        };
        let mut guard = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_none_or(|l| l.stamp != stamp) {
            let prev = guard.take();
            let config = match Config::load(&self.config_path) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("{e}（{}）", self.config_path.display());
                    prev.as_ref().map(|p| p.config.clone()).unwrap_or_default()
                }
            };
            let apps = match AppRules::load(&self.apps_path) {
                Ok(a) => a,
                Err(e) => {
                    log::error!("{e}（{}）", self.apps_path.display());
                    prev.as_ref().map(|p| p.apps.clone()).unwrap_or_default()
                }
            };
            log::info!("載入設定：{} 條 App 規則", apps.apps.len());
            *guard = Some(Loaded {
                config,
                apps,
                stamp,
            });
        }
        let l = guard.as_ref().expect("剛剛載入");
        (l.config.clone(), l.apps.clone())
    }
}
