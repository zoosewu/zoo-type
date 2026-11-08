//! `apps.toml`：依 App 覆寫設定（需求 4.4）
//!
//! 同一筆規則可以同時寫 Windows 執行檔名與 macOS Bundle ID，兩平台共用一份檔案、同步也不衝突。
//!
//! ```toml
//! [[app]]
//! name = "VS Code"
//! windows = "Code.exe"
//! macos = "com.microsoft.VSCode"
//! punctuation = "half"
//!
//! [[app]]
//! name = "Elden Ring"
//! windows = "eldenring.exe"
//! game_mode = "english"
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{Config, ConfigError, GameMode, InputMode, OnFocus, Output, Width};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AppRule {
    /// 顯示用名稱
    pub name: String,
    /// Windows 執行檔名稱（不分大小寫），例如 `Code.exe`
    pub windows: Option<String>,
    /// macOS Bundle ID，例如 `com.microsoft.VSCode`
    pub macos: Option<String>,
    pub default_mode: Option<InputMode>,
    pub punctuation: Option<Width>,
    pub output: Option<Output>,
    pub on_focus: Option<OnFocus>,
    pub game_mode: Option<GameMode>,
    /// `false`：這個 App 不做啟發式偵測（例如遊戲聊天框是自己畫的，被誤判成不能打字）
    pub detect: Option<bool>,
    /// 組字提示列（None：純終端機開，其他沿用全域設定）；快捷鍵切換時自動寫入
    pub composition_bar: Option<bool>,
}

impl AppRule {
    fn matches(&self, app: &str) -> bool {
        self.windows
            .as_deref()
            .is_some_and(|w| w.eq_ignore_ascii_case(app))
            || self.macos.as_deref() == Some(app)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AppRules {
    #[serde(rename = "app")]
    pub apps: Vec<AppRule>,
}

impl AppRules {
    pub fn parse(text: &str) -> Result<AppRules, ConfigError> {
        toml::from_str(text).map_err(ConfigError::Parse)
    }

    pub fn load(path: &Path) -> Result<AppRules, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => AppRules::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppRules::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    /// 存回檔案（暫存檔再改名）；設定程式使用
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// 第一筆符合的規則
    pub fn find(&self, app: &str) -> Option<&AppRule> {
        self.apps.iter().find(|r| r.matches(app))
    }
}

/// 全域設定套用 App 規則之後的結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effective {
    pub default_mode: InputMode,
    pub width: Width,
    pub output: Output,
    pub on_focus: OnFocus,
    pub game_mode: GameMode,
    pub detect_legacy: bool,
    pub detect_fullscreen: bool,
}

impl Effective {
    pub fn resolve(config: &Config, rule: Option<&AppRule>) -> Effective {
        let base = &config.input;
        let r = rule.cloned().unwrap_or_default();
        Effective {
            default_mode: r.default_mode.unwrap_or(base.default_mode),
            width: r.punctuation.unwrap_or(base.punctuation),
            output: r.output.unwrap_or(base.output),
            on_focus: r.on_focus.unwrap_or(base.on_focus),
            game_mode: r.game_mode.unwrap_or(GameMode::Off),
            detect_legacy: r.detect.unwrap_or(true) && config.detect.legacy_fullscreen,
            detect_fullscreen: r.detect.unwrap_or(true) && config.detect.exclusive_fullscreen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [[app]]
        name = "VS Code"
        windows = "Code.exe"
        macos = "com.microsoft.VSCode"
        default_mode = "english"
        punctuation = "half"

        [[app]]
        windows = "eldenring.exe"
        game_mode = "english"
    "#;

    #[test]
    fn finds_by_exe_case_insensitive_or_bundle_id() {
        let rules = AppRules::parse(SAMPLE).unwrap();
        assert_eq!(rules.find("code.exe").unwrap().name, "VS Code");
        assert_eq!(rules.find("com.microsoft.VSCode").unwrap().name, "VS Code");
        assert!(rules.find("notepad.exe").is_none());
    }

    #[test]
    fn rule_overrides_only_what_it_sets() {
        let rules = AppRules::parse(SAMPLE).unwrap();
        let mut config = Config::default();
        config.input.output = Output::Simplified;
        let e = Effective::resolve(&config, rules.find("Code.exe"));
        assert_eq!(e.default_mode, InputMode::English);
        assert_eq!(e.width, Width::Half);
        assert_eq!(e.output, Output::Simplified, "沒寫的欄位沿用全域設定");
        assert_eq!(e.game_mode, GameMode::Off);
        let g = Effective::resolve(&config, rules.find("eldenring.exe"));
        assert_eq!(g.game_mode, GameMode::English);
    }

    #[test]
    fn detect_can_be_disabled_globally_or_per_app() {
        let rules = AppRules::parse("[[app]]
windows = 'chat.exe'
detect = false").unwrap();
        let config = Config::default();
        let e = Effective::resolve(&config, None);
        assert!(e.detect_legacy && e.detect_fullscreen);
        let e = Effective::resolve(&config, rules.find("chat.exe"));
        assert!(!e.detect_legacy && !e.detect_fullscreen);
        let mut off = Config::default();
        off.detect.exclusive_fullscreen = false;
        let e = Effective::resolve(&off, None);
        assert!(e.detect_legacy && !e.detect_fullscreen);
    }

    #[test]
    fn save_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.toml");
        let rules = AppRules::parse(SAMPLE).unwrap();
        rules.save(&path).unwrap();
        assert_eq!(AppRules::load(&path).unwrap(), rules);
    }

    #[test]
    fn typo_in_field_is_an_error() {
        assert!(AppRules::parse("[[app]]\nwindow = 'a.exe'").is_err());
    }
}
