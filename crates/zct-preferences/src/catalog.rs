use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zct_core::{AppRule, Config};

use crate::Capabilities;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Macos,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Bool,
    Text,
    Number,
    Choice,
    Color,
    Json,
    Hotkey,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub label: String,
    pub value: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub path: String,
    pub label: String,
    pub page: String,
    pub kind: FieldKind,
    pub choices: Vec<Choice>,
    pub default: Value,
    pub nullable: bool,
    pub integer: bool,
    pub help: String,
    /// Runtime capability, independent of whether a portable setting may be edited.
    pub feature: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub help: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub pages: Vec<Page>,
    pub fields: Vec<Field>,
    pub app_fields: Vec<Field>,
    pub capabilities: Capabilities,
}
fn choices(items: &[(&str, &str)]) -> Vec<Choice> {
    items
        .iter()
        .map(|(label, value)| Choice {
            label: (*label).into(),
            value: json!(value),
        })
        .collect()
}
fn field(
    path: &str,
    label: &str,
    page: &str,
    kind: FieldKind,
    options: &[(&str, &str)],
    defaults: &Value,
) -> Field {
    let mut value = defaults;
    for key in path.split('.') {
        value = &value[key];
    }
    Field {
        path: path.into(),
        label: label.into(),
        page: page.into(),
        kind,
        choices: choices(options),
        default: value.clone(),
        nullable: false,
        integer: value.as_number().is_some_and(|n| !n.is_f64()),
        help: String::new(),
        feature: None,
    }
}
pub fn catalog(platform: Platform) -> Catalog {
    use FieldKind::*;
    let defaults = serde_json::to_value(Config::default()).expect("serializable Config");
    let mut fields = Vec::new();
    let mut add =
        |path: &str, label: &str, page: &str, kind: FieldKind, options: &[(&str, &str)]| {
            fields.push(field(path, label, page, kind, options, &defaults))
        };
    add(
        "input.default_mode",
        "預設模式",
        "general",
        Choice,
        &[("中文", "chinese"), ("英文", "english")],
    );
    add(
        "input.punctuation",
        "標點",
        "general",
        Choice,
        &[("全形", "full"), ("半形", "half")],
    );
    add(
        "input.output",
        "文字輸出",
        "general",
        Choice,
        &[("繁體", "traditional"), ("簡體", "simplified")],
    );
    add(
        "input.on_focus",
        "切換 App 時",
        "general",
        Choice,
        &[("記住狀態", "remember"), ("恢復預設", "reset")],
    );
    add(
        "toggle.shift",
        "單按 Shift 切中英",
        "general",
        Choice,
        &[
            ("左右皆可", "both"),
            ("左 Shift", "left"),
            ("右 Shift", "right"),
            ("關閉", "off"),
        ],
    );
    add(
        "toggle.shift_tap_max_ms",
        "按住上限（毫秒）",
        "general",
        Number,
        &[],
    );
    add(
        "candidates.per_page",
        "每頁候選字數",
        "general",
        Number,
        &[],
    );
    add(
        "candidates.sort_by_frequency",
        "依常用程度與個人學習排序",
        "general",
        Bool,
        &[],
    );
    add(
        "candidates.language_model",
        "智慧選字",
        "general",
        Bool,
        &[],
    );
    add(
        "candidates.select",
        "選字對象",
        "general",
        Choice,
        &[("游標後的詞", "after"), ("游標前的詞", "before")],
    );
    add(
        "candidates.after_select",
        "選字後游標",
        "general",
        Choice,
        &[
            ("移到詞後面", "forward"),
            ("不動", "stay"),
            ("移到詞前面", "backward"),
        ],
    );
    add(
        "mixed.auto_english",
        "自動辨識英文單字",
        "general",
        Bool,
        &[],
    );
    add(
        "mixed.english_candidate",
        "候選窗包含英文單字",
        "general",
        Bool,
        &[],
    );
    add(
        "mixed.auto_space",
        "中英文之間自動加空格",
        "general",
        Bool,
        &[],
    );
    add(
        "detect.legacy_fullscreen",
        "偵測舊式全螢幕程式",
        "general",
        Bool,
        &[],
    );
    add(
        "detect.exclusive_fullscreen",
        "偵測獨佔全螢幕",
        "general",
        Bool,
        &[],
    );
    add(
        "punctuation.primary",
        "主要符號",
        "general",
        Choice,
        &[
            ("` + 鍵", "backtick"),
            ("右 Alt / Option + 鍵", "right_alt"),
            ("兩種皆可", "both"),
        ],
    );
    add(
        "punctuation.secondary",
        "次要符號",
        "general",
        Choice,
        &[("Shift + 鍵", "shift"), ("符號層 + Shift + 鍵", "layer")],
    );
    add(
        "punctuation.full",
        "自訂全形標點（JSON）",
        "general",
        Json,
        &[],
    );
    add(
        "punctuation.half",
        "自訂半形標點（JSON）",
        "general",
        Json,
        &[],
    );
    for (key, label) in [
        ("width", "全形／半形"),
        ("output", "繁體／簡體"),
        ("game_mode", "遊戲模式"),
        ("forget", "忘記學習"),
        ("pin", "設為優先"),
        ("emoji", "Emoji"),
        ("ai", "AI 修正"),
        ("composition_bar", "組字提示列"),
        ("symbols", "符號選單"),
        ("reconvert", "重新選字"),
    ] {
        add(&format!("hotkeys.{key}"), label, "hotkeys", Hotkey, &[]);
    }
    add(
        "theme.layout",
        "排列方向",
        "appearance",
        Choice,
        &[("直式", "vertical"), ("橫式", "horizontal")],
    );
    add("theme.font", "字型", "appearance", Text, &[]);
    add("theme.font_size", "字級", "appearance", Number, &[]);
    for (key, label) in [
        ("background", "背景"),
        ("text", "文字"),
        ("number", "候選編號"),
        ("highlight", "選取背景"),
    ] {
        add(&format!("theme.{key}"), label, "appearance", Color, &[]);
    }
    add(
        "theme.composition_bar",
        "另外顯示組字提示列",
        "appearance",
        Bool,
        &[],
    );
    add("sync.enabled", "啟用 git 同步", "sync", Bool, &[]);
    add("sync.remote", "私人遠端 repo", "sync", Text, &[]);
    add(
        "sync.interval_minutes",
        "同步間隔（分鐘）",
        "sync",
        Number,
        &[],
    );
    add("ai.enabled", "啟用 AI 修正", "ai", Bool, &[]);
    add("ai.endpoint", "OpenAI 相容 API 網址", "ai", Text, &[]);
    add("ai.model", "模型", "ai", Text, &[]);
    add("ai.timeout_seconds", "逾時（秒）", "ai", Number, &[]);
    add("update.auto_check", "自動檢查更新", "about", Bool, &[]);
    for field in &mut fields {
        if field.path == "hotkeys.composition_bar" {
            field.feature = Some("composition_bar_hotkey".into());
        }
        if field.path.starts_with("detect.") {
            field.feature = Some("game_detection".into());
        }
        if field.path.starts_with("sync.") {
            field.feature = Some("sync".into());
        }
        if field.path.starts_with("ai.") || field.path == "hotkeys.ai" {
            field.feature = Some("ai".into());
        }
        if field.path == "update.auto_check" {
            field.feature = Some("auto_update".into());
        }
        if field.kind == Hotkey {
            field.help = "留空表示不綁定；ctrl、shift、alt（Option）、win（Command）加按鍵，例如 shift+space。".into();
        }
        if field.path == "theme.font" {
            field.help = "未安裝指定字型時使用目前平台的系統字型。".into();
        }
    }
    let pages = [
        ("general", "一般", "輸入、符號、中英混打與候選排序。"),
        ("hotkeys", "快捷鍵", "兩平台共用相同的快捷鍵語法。"),
        ("appearance", "外觀", "排列、字型、配色與即時預覽。"),
        (
            "apps",
            "App 規則",
            "同一筆規則可同時指定 Windows 執行檔與 macOS Bundle ID。",
        ),
        ("dictionary", "詞庫", "使用者詞庫、縮寫與英文單字。"),
        ("learning", "學習", "每日統計、備份與還原。"),
        ("sync", "同步", "設定資料位置與 git 同步。"),
        ("ai", "AI", "OpenAI 相容 API 及模型設定。"),
        ("about", "關於", "版本、更新及資料位置。"),
    ]
    .map(|(id, title, help)| Page {
        id: id.into(),
        title: title.into(),
        help: help.into(),
    })
    .to_vec();
    Catalog {
        version: 1,
        pages,
        fields,
        app_fields: app_fields(),
        capabilities: Capabilities::for_platform(platform),
    }
}
pub fn app_fields() -> Vec<Field> {
    use FieldKind::*;
    let defaults = serde_json::to_value(AppRule::default()).expect("serializable AppRule");
    let specs = [
        ("name", "名稱", Text, vec![]),
        ("windows", "Windows 執行檔", Text, vec![]),
        ("macos", "macOS Bundle ID", Text, vec![]),
        (
            "default_mode",
            "預設模式",
            Choice,
            vec![("中文", "chinese"), ("英文", "english")],
        ),
        (
            "punctuation",
            "標點",
            Choice,
            vec![("全形", "full"), ("半形", "half")],
        ),
        (
            "output",
            "文字輸出",
            Choice,
            vec![("繁體", "traditional"), ("簡體", "simplified")],
        ),
        (
            "on_focus",
            "切換 App 時",
            Choice,
            vec![("記住狀態", "remember"), ("恢復預設", "reset")],
        ),
        (
            "game_mode",
            "遊戲模式",
            Choice,
            vec![
                ("關閉", "off"),
                ("鎖定英文", "english"),
                ("保留狀態", "keep"),
            ],
        ),
        ("detect", "啟發式偵測", Choice, vec![]),
        ("composition_bar", "組字提示列", Choice, vec![]),
    ];
    specs
        .into_iter()
        .map(|(path, label, kind, options)| {
            let mut f = field(path, label, "apps", kind, &options, &defaults);
            f.nullable = path != "name";
            if path == "detect" {
                f.feature = Some("game_detection".into());
            }
            if f.nullable && kind == Choice {
                f.choices.insert(
                    0,
                    crate::Choice {
                        label: "沿用全域設定".into(),
                        value: Value::Null,
                    },
                );
                if options.is_empty() {
                    f.choices.extend([
                        crate::Choice {
                            label: "開啟".into(),
                            value: json!(true),
                        },
                        crate::Choice {
                            label: "關閉".into(),
                            value: json!(false),
                        },
                    ]);
                }
            }
            f
        })
        .collect()
}
