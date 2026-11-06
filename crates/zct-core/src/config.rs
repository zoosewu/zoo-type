//! `config.toml`：兩平台共用、會被 git 同步的使用者設定

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::hotkey::{Binding, Hotkey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    #[default]
    Chinese,
    English,
}

impl InputMode {
    pub fn toggled(self) -> InputMode {
        match self {
            InputMode::Chinese => InputMode::English,
            InputMode::English => InputMode::Chinese,
        }
    }
}

/// 中文模式下標點符號的寬度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Width {
    /// 全形：，。？「」
    #[default]
    Full,
    /// 半形：,.?[]
    Half,
}

impl Width {
    pub fn toggled(self) -> Width {
        match self {
            Width::Full => Width::Half,
            Width::Half => Width::Full,
        }
    }
}

/// 送出文字的字集
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    #[default]
    Traditional,
    Simplified,
}

impl Output {
    pub fn toggled(self) -> Output {
        match self {
            Output::Traditional => Output::Simplified,
            Output::Simplified => Output::Traditional,
        }
    }
}

/// 切回某個 App（輸入焦點回來）時的中英狀態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OnFocus {
    /// 保留上次的狀態
    #[default]
    Remember,
    /// 重設為預設狀態
    Reset,
}

/// 遊戲模式的行為
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GameMode {
    /// 不啟用
    #[default]
    Off,
    /// 鎖定英文：所有按鍵直接交給 App
    English,
    /// 保留目前中英狀態，但 Shift 不切換（遊戲聊天室打中文用）
    Keep,
}

/// 哪一側的 Shift 可以切換中英
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ShiftToggle {
    #[default]
    Both,
    Left,
    Right,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InputConfig {
    /// 新的輸入 context 預設的中英狀態
    pub default_mode: InputMode,
    /// 中文模式下標點的預設寬度
    pub punctuation: Width,
    /// 送出文字的字集
    pub output: Output,
    /// 切回 App 時的中英狀態
    pub on_focus: OnFocus,
}

impl Default for InputConfig {
    fn default() -> Self {
        InputConfig {
            default_mode: InputMode::Chinese,
            punctuation: Width::Full,
            output: Output::Traditional,
            on_focus: OnFocus::Remember,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToggleConfig {
    pub shift: ShiftToggle,
    /// 按住 Shift 超過這個時間才放開，不算單按
    pub shift_tap_max_ms: u64,
}

impl Default for ToggleConfig {
    fn default() -> Self {
        ToggleConfig {
            shift: ShiftToggle::Both,
            shift_tap_max_ms: 300,
        }
    }
}

/// 快捷鍵；留空字串代表不綁定
///
/// 綁定的組合鍵即使含 Ctrl / Alt 也會被輸入法攔截，其餘組合鍵一律交給 App（P3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HotkeyConfig {
    /// 切換標點全形 / 半形
    pub width: Binding,
    /// 切換繁體 / 簡體輸出（預設不綁定：常見的 Ctrl+Shift+F 會搶走 VS Code 的全域搜尋）
    pub output: Binding,
    /// 開關遊戲模式（全域）
    pub game_mode: Binding,
    /// 候選窗中：忘記選擇框所在詞的學習紀錄（並取消固頂）
    pub forget: Binding,
    /// 候選窗中：固頂選擇框所在的詞並選它
    pub pin: Binding,
    /// Emoji：以組字內容當關鍵字搜尋；沒有組字時列出常用 Emoji
    pub emoji: Binding,
    /// AI 修正組字內容中的同音錯字
    pub ai: Binding,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        HotkeyConfig {
            width: Binding::key("shift+space"),
            output: Binding(None),
            game_mode: Binding::key("ctrl+shift+f12"),
            forget: Binding::key("shift+delete"),
            pin: Binding::key("shift+enter"),
            // 不用 Ctrl+Alt：RichEdit（Win11 記事本）、Word 內建 Ctrl+Alt+E = €，部分鍵盤配置也把 Ctrl+Alt 當 AltGr
            emoji: Binding::key("ctrl+shift+;"),
            ai: Binding::key("ctrl+shift+'"),
        }
    }
}

/// 自動偵測「不在輸入狀態」（此時按鍵全部交給 App，Shift 不切換中英）
///
/// 系統明確告知的情況（焦點不在輸入欄位、App 停用輸入法）一律偵測，不受這裡的開關影響。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DetectConfig {
    /// 不支援 TSF 的舊式程式 + 沒有系統游標 + 視窗蓋滿螢幕（無邊框全螢幕的 DirectX 遊戲）
    pub legacy_fullscreen: bool,
    /// 有程式獨佔全螢幕（老式全螢幕遊戲）
    pub exclusive_fullscreen: bool,
}

impl Default for DetectConfig {
    fn default() -> Self {
        DetectConfig {
            legacy_fullscreen: true,
            exclusive_fullscreen: true,
        }
    }
}

/// AI 同音字修正（需求 4.13）：OpenAI 相容 API；API Key 存在 Windows 認證管理員，不在設定檔
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AiConfig {
    pub enabled: bool,
    /// 例如 Ollama `http://localhost:11434/v1`、LM Studio `http://localhost:1234/v1`
    pub endpoint: String,
    pub model: String,
    pub timeout_seconds: u64,
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig {
            enabled: false,
            endpoint: "http://localhost:11434/v1".into(),
            model: String::new(),
            timeout_seconds: 15,
        }
    }
}

/// git 同步（需求 4.8）：資料根目錄本身就是 git repo
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyncConfig {
    pub enabled: bool,
    /// 遠端 repo（請用私人 repo：使用者詞庫等同打字內容的摘要）；空字串代表只在本機 commit
    pub remote: String,
    /// 自動同步間隔（分鐘）
    pub interval_minutes: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            enabled: false,
            remote: String::new(),
            interval_minutes: 10,
        }
    }
}

/// 中英混打（需求 4.5：B + C + E）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MixedConfig {
    /// C：按空白時，剛打的按鍵是英文單字且不是合法注音 → 送出英文
    pub auto_english: bool,
    /// B：剛完成的音節按鍵也是英文單字時，候選窗附上英文
    pub english_candidate: bool,
    /// E：中英文之間自動加空格（盤古之白）
    pub auto_space: bool,
}

impl Default for MixedConfig {
    fn default() -> Self {
        MixedConfig {
            auto_english: true,
            english_candidate: true,
            // 空白由使用者自己輸入（使用者回饋）
            auto_space: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CandidateConfig {
    pub per_page: usize,
    /// 候選字依常用程度與個人學習排序（P2：常用字排前面）
    pub sort_by_frequency: bool,
}

impl Default for CandidateConfig {
    fn default() -> Self {
        CandidateConfig {
            per_page: 9,
            sort_by_frequency: true,
        }
    }
}

/// 候選窗排列方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Vertical,
    Horizontal,
}

/// `#RRGGBB` 顏色
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub fn rgb(self) -> (u8, u8, u8) {
        ((self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8)
    }
}

impl std::str::FromStr for Color {
    type Err = String;
    fn from_str(s: &str) -> Result<Color, String> {
        let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
        if hex.len() != 6 {
            return Err(format!("顏色格式應為 #RRGGBB：{s}"));
        }
        u32::from_str_radix(hex, 16)
            .map(Color)
            .map_err(|_| format!("顏色格式應為 #RRGGBB：{s}"))
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:06X}", self.0)
    }
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// 候選窗外觀
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    pub layout: Layout,
    pub font: String,
    /// 候選字字級（DIP），數字約為其 0.75 倍
    pub font_size: f32,
    pub background: Color,
    pub text: Color,
    pub number: Color,
    pub highlight: Color,
    /// 組字提示列：游標上方顯示組字內容、選字目標與字數（App 自己畫的底線不明顯時很有用）
    pub composition_bar: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            layout: Layout::Vertical,
            font: "Microsoft JhengHei UI".into(),
            font_size: 16.0,
            background: Color(0xFFFFFF),
            text: Color(0x1A1A1A),
            number: Color(0x7A7A7A),
            highlight: Color(0xCCE4F7),
            composition_bar: true,
        }
    }
}

/// 自訂標點：`"shift+," = "，"`，覆寫內建的對照
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PunctuationConfig {
    pub full: BTreeMap<String, String>,
    pub half: BTreeMap<String, String>,
}

/// 驗證過的標點對照表
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PunctuationTable {
    pub full: Vec<(Hotkey, String)>,
    pub half: Vec<(Hotkey, String)>,
}

impl PunctuationConfig {
    pub fn compile(&self) -> Result<PunctuationTable, String> {
        let compile = |map: &BTreeMap<String, String>| {
            map.iter()
                .map(|(k, v)| {
                    let hk: Hotkey = k.parse()?;
                    if hk.ctrl || hk.alt || hk.meta {
                        return Err(format!("標點不能使用 Ctrl / Alt / Win 組合鍵：{k}"));
                    }
                    if v.is_empty() {
                        return Err(format!("標點「{k}」的輸出不能是空字串"));
                    }
                    Ok((hk, v.clone()))
                })
                .collect::<Result<Vec<_>, String>>()
        };
        Ok(PunctuationTable {
            full: compile(&self.full)?,
            half: compile(&self.half)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub input: InputConfig,
    pub toggle: ToggleConfig,
    pub hotkeys: HotkeyConfig,
    pub detect: DetectConfig,
    pub mixed: MixedConfig,
    pub sync: SyncConfig,
    pub ai: AiConfig,
    pub candidates: CandidateConfig,
    pub punctuation: PunctuationConfig,
    pub theme: ThemeConfig,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "無法讀取設定檔：{e}"),
            ConfigError::Parse(e) => write!(f, "設定檔格式錯誤：{e}"),
            ConfigError::Invalid(e) => write!(f, "設定檔內容錯誤：{e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let config: Config = toml::from_str(text).map_err(ConfigError::Parse)?;
        config
            .punctuation
            .compile()
            .map_err(ConfigError::Invalid)?;
        if !(8.0..=48.0).contains(&config.theme.font_size) {
            return Err(ConfigError::Invalid(format!(
                "候選字字級應在 8–48 之間：{}",
                config.theme.font_size
            )));
        }
        Ok(config)
    }

    /// 存回檔案（暫存檔再改名）；設定程式使用
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// 檔案不存在時使用預設值；格式錯誤則回報錯誤（不默默忽略使用者的設定）
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Key;

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn partial_override() {
        let c = Config::parse(
            r#"
            [input]
            default_mode = "english"
            output = "simplified"
            [toggle]
            shift = "left"
            "#,
        )
        .unwrap();
        assert_eq!(c.input.default_mode, InputMode::English);
        assert_eq!(c.input.output, Output::Simplified);
        assert_eq!(c.toggle.shift, ShiftToggle::Left);
        assert_eq!(c.toggle.shift_tap_max_ms, 300);
    }

    #[test]
    fn unknown_key_is_an_error() {
        assert!(Config::parse("[toggle]\nshfit = \"left\"").is_err());
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn hotkeys_can_be_rebound_or_cleared() {
        let c = Config::parse("[hotkeys]\nwidth = ''\noutput = 'ctrl+alt+s'").unwrap();
        assert_eq!(c.hotkeys.width, Binding(None));
        assert_eq!(c.hotkeys.output.0.unwrap().key, Key::Char(b's'));
        assert!(Config::parse("[hotkeys]\nwidth = 'ctrl+bogus'").is_err());
    }

    #[test]
    fn theme_colors_and_size_are_validated() {
        let c = Config::parse("[theme]
layout = 'horizontal'
background = '#202020'").unwrap();
        assert_eq!(c.theme.layout, Layout::Horizontal);
        assert_eq!(c.theme.background, Color(0x202020));
        assert!(Config::parse("[theme]
text = 'red'").is_err());
        assert!(Config::parse("[theme]
font_size = 200.0").is_err());
    }

    #[test]
    fn save_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut c = Config::default();
        c.theme.layout = Layout::Horizontal;
        c.hotkeys.output = Binding::key("ctrl+alt+s");
        c.punctuation.full.insert("shift+,".into(), "《".into());
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
    }

    #[test]
    fn punctuation_table_is_validated() {
        let c = Config::parse("[punctuation.full]\n\"shift+,\" = \"《\"").unwrap();
        let t = c.punctuation.compile().unwrap();
        assert_eq!(t.full[0].1, "《");
        // P3：標點不能綁 Ctrl 組合鍵
        assert!(Config::parse("[punctuation.full]\n\"ctrl+,\" = \"，\"").is_err());
        assert!(Config::parse("[punctuation.half]\n\"'\" = \"\"").is_err());
    }
}
