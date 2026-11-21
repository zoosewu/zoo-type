use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zct_core::config::Habit;
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
    /// 另一個欄位選了特定值時，這個欄位固定成對應的值、不顯示（自己設定的值保留著）
    pub lock: Option<Lock>,
    /// 少用的設定：收在頁面最後預設收合的「進階」
    pub advanced: bool,
    /// 頁面內的分區標題；和前一個欄位不同時顯示（進階的欄位不分區）
    pub section: String,
    /// 數字欄位的範圍；儲存時檢查（見 [`crate::validate`]）
    pub range: Option<Range>,
    /// 設定畫面不顯示（只能改設定檔；「全部設定」頁仍列出）
    pub hidden: bool,
    /// 選了某個選項時順便寫入的其他欄位：選項值 → {欄位路徑: 值}。
    /// 目前的設定不符合任何一個選項（手動改過設定檔）時，畫面顯示「自訂」，儲存時不動這些欄位
    pub also: BTreeMap<String, BTreeMap<String, Value>>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Range {
    pub min: i64,
    pub max: i64,
    pub step: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lock {
    /// 控制的欄位
    pub by: String,
    /// 控制欄位的值 → 這個欄位固定的值；不在表裡的值不鎖定
    pub values: BTreeMap<String, Value>,
}
/// 「自訂」：目前的設定不符合任何選項時顯示
pub const CUSTOM_CHOICE: &str = "自訂";

impl Field {
    /// 目前的設定對應到哪一個選項（含 [`Field::also`] 一併寫入的欄位）；手動改過、都不符合時是 None（顯示「自訂」）
    pub fn chosen(&self, config: &Value) -> Option<usize> {
        let value = path_value(config, &self.path);
        self.choices.iter().position(|c| {
            &c.value == value
                && c.value.as_str().is_none_or(|key| {
                    self.also
                        .get(key)
                        .is_none_or(|more| more.iter().all(|(p, v)| path_value(config, p) == v))
                })
        })
    }

    /// 選了第 `index` 個選項要寫入的（欄位, 值），包含順便寫入的欄位；「自訂」（超出範圍）是空的
    pub fn writes(&self, index: usize) -> Vec<(String, Value)> {
        let Some(choice) = self.choices.get(index) else {
            return Vec::new();
        };
        let mut out = vec![(self.path.clone(), choice.value.clone())];
        if let Some(more) = choice.value.as_str().and_then(|key| self.also.get(key)) {
            out.extend(more.iter().map(|(p, v)| (p.clone(), v.clone())));
        }
        out
    }

    /// 依目前的設定，這個欄位被鎖定成的值；沒有鎖定是 None
    pub fn locked_value(&self, config: &Value) -> Option<&Value> {
        let lock = self.lock.as_ref()?;
        let mut by = config;
        for key in lock.by.split('.') {
            by = &by[key];
        }
        lock.values.get(by.as_str()?)
    }
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
        lock: None,
        advanced: false,
        section: String::new(),
        range: None,
        hidden: false,
        also: BTreeMap::new(),
    }
}
/// 一般頁：（欄位, 分區, 說明）。進階的欄位不分區
const GENERAL: &[(&str, &str, &str)] = &[
    ("input.default_mode", "輸入", "新開 App 時的中英狀態。"),
    (
        "input.punctuation",
        "輸入",
        "全形（，。？）或半形（,.?），所有 App 共用。",
    ),
    (
        "toggle.shift",
        "輸入",
        "單按 Shift 切換。組字中切換不會送出，接著打的英文和數字加進組字區：第 Shift 6 Shift 章 → 第6章。",
    ),
    ("candidates.per_page", "候選字", ""),
    (
        "candidates.language_model",
        "候選字",
        "用內建的語言模型判斷整句哪個同音字最通順（例如「下次再說」「在家」），在電腦上執行，不連網路。",
    ),
    (
        "candidates.habit",
        "候選字",
        "Windows：選游標前的詞，選完游標移到詞後面。macOS：選游標後的詞，選完游標不動。",
    ),
    ("candidates.select", "候選字", ""),
    ("candidates.after_select", "候選字", ""),
    (
        "mixed.english",
        "中英混打",
        "中文優先：句首或空白後打單字表裡的英文再按空白，才當作英文（hello、repo）；組字區顯示注音。英文優先：組不成注音就轉成英文（lo → love），之後的按鍵都是字元本身，空白結束這個單字；組字區顯示按下的字母，注音在旁邊的小視窗。用 Shift 打過的英文會記住。",
    ),
    (
        "mixed.english_candidate",
        "中英混打",
        "按鍵也是英文單字時，候選窗多一個英文選項：fu → 七、fu。",
    ),
    (
        "mixed.auto_space",
        "中英混打",
        "中文與英文、數字之間加一個空格。",
    ),
    (
        "mixed.typing",
        "中英混打",
        "注音還沒打完時，組字區顯示注音或按下的字母。",
    ),
    (
        "input.output",
        "",
        "組字區與候選窗一律顯示繁體，送出時才轉成簡體。",
    ),
    ("input.on_focus", "", ""),
    (
        "toggle.shift_tap_max_ms",
        "",
        "按住 Shift 超過這個時間才放開，不算切換。",
    ),
    (
        "punctuation.primary",
        "",
        "鍵帽下排的字元：` 再按 , → ，、` 再按 1 → １、` 按兩下 → `。用右 Alt 時，App 收不到右 Alt 組合鍵。",
    ),
    (
        "punctuation.secondary",
        "",
        "鍵帽上排的符號：Shift + , → ＜、Shift + ' → ＂。選第二種時，Shift + 鍵改打中文標點（，。；「」）。",
    ),
    (
        "candidates.sort_by_frequency",
        "",
        "常用字在前，並參考你的選字紀錄。",
    ),
    (
        "detect.fullscreen",
        "",
        "全螢幕遊戲、簡報播放時，按鍵直接交給 App，Shift 也不切換中英。視窗模式的遊戲偵測不到，請用遊戲模式。",
    ),
    ("punctuation.full", "", ""),
    ("punctuation.half", "", ""),
];

/// 數字欄位：（欄位, 最小, 最大, 間隔）
const RANGES: &[(&str, i64, i64, i64)] = &[
    ("candidates.per_page", 1, 10, 1),
    ("toggle.shift_tap_max_ms", 100, 1000, 50),
    ("theme.font_size", 8, 48, 1),
    ("sync.interval_minutes", 1, 1440, 1),
    ("ai.timeout_seconds", 1, 120, 1),
];

fn path_value<'a>(config: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(config, |v, key| &v[key])
}

/// 設定畫面不顯示的欄位（只能改設定檔）
const HIDDEN: &[&str] = &[
    // 由「英文單字判斷」決定（見 also）
    "mixed.typing",
    "mixed.english_candidate",
    "mixed.auto_space",
];

pub fn catalog(platform: Platform) -> Catalog {
    use FieldKind::*;
    let defaults = serde_json::to_value(Config::default()).expect("serializable Config");
    let mut fields = Vec::new();
    let mut add =
        |path: &str, label: &str, page: &str, kind: FieldKind, options: &[(&str, &str)]| {
            fields.push(field(path, label, page, kind, options, &defaults))
        };
    let right_alt = match platform {
        Platform::Windows => "右 Alt",
        Platform::Macos => "右 Option",
    };
    let primary = [
        "` + 鍵".to_string(),
        format!("{right_alt} + 鍵"),
        format!("` + 鍵 或 {right_alt} + 鍵"),
    ];
    let secondary = [
        "Shift + 鍵".to_string(),
        format!("` 或 {right_alt} + Shift + 鍵"),
    ];
    // 一般頁：順序就是畫面上的順序
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
        "toggle.shift",
        "Shift 切換中英",
        "general",
        Choice,
        &[
            ("左右都可", "both"),
            ("左 Shift", "left"),
            ("右 Shift", "right"),
            ("不使用", "off"),
        ],
    );
    add("candidates.per_page", "每頁字數", "general", Number, &[]);
    add(
        "candidates.language_model",
        "智慧選字",
        "general",
        Bool,
        &[],
    );
    add(
        "candidates.habit",
        "操作習慣",
        "general",
        Choice,
        &[
            ("自訂", "custom"),
            ("Windows", "windows"),
            ("macOS", "macos"),
        ],
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
        "mixed.english",
        "英文單字判斷",
        "general",
        Choice,
        &[
            ("關", "off"),
            ("中文優先", "chinese_first"),
            ("英文優先", "english_first"),
        ],
    );
    add(
        "mixed.english_candidate",
        "候選窗附上英文",
        "general",
        Bool,
        &[],
    );
    add("mixed.auto_space", "中英之間自動空格", "general", Bool, &[]);
    add(
        "mixed.typing",
        "打字時顯示",
        "general",
        Choice,
        &[
            ("注音", "bopomofo"),
            ("字母＋注音小視窗", "keys"),
            ("字母＋注音與常用字", "keys_and_chars"),
        ],
    );
    // 以下收在「進階」
    add(
        "input.output",
        "輸出",
        "general",
        Choice,
        &[("繁體", "traditional"), ("簡體", "simplified")],
    );
    add(
        "input.on_focus",
        "切回 App 時",
        "general",
        Choice,
        &[("保留中英狀態", "remember"), ("重設為預設模式", "reset")],
    );
    add(
        "toggle.shift_tap_max_ms",
        "Shift 按住上限（毫秒）",
        "general",
        Number,
        &[],
    );
    add(
        "punctuation.primary",
        "主要符號",
        "general",
        Choice,
        &[
            (primary[0].as_str(), "backtick"),
            (primary[1].as_str(), "right_alt"),
            (primary[2].as_str(), "both"),
        ],
    );
    add(
        "punctuation.secondary",
        "次要符號",
        "general",
        Choice,
        &[
            (secondary[0].as_str(), "shift"),
            (secondary[1].as_str(), "layer"),
        ],
    );
    add(
        "candidates.sort_by_frequency",
        "依常用程度排序",
        "general",
        Bool,
        &[],
    );
    add(
        "detect.fullscreen",
        "全螢幕遊戲時不處理按鍵",
        "general",
        Bool,
        &[],
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
    const ADVANCED: &[&str] = &[
        "input.output",
        "input.on_focus",
        "toggle.shift_tap_max_ms",
        "candidates.sort_by_frequency",
        "detect.fullscreen",
        "punctuation.primary",
        "punctuation.secondary",
        "punctuation.full",
        "punctuation.half",
    ];
    debug_assert!(ADVANCED.iter().all(|p| fields.iter().any(|f| f.path == *p)));
    for field in &mut fields {
        field.advanced = ADVANCED.contains(&field.path.as_str());
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
        if field.path == "candidates.select" || field.path == "candidates.after_select" {
            let values = Habit::PRESETS
                .iter()
                .filter_map(|&habit| {
                    let (side, after) = habit.selection()?;
                    let value = if field.path == "candidates.select" {
                        json!(side)
                    } else {
                        json!(after)
                    };
                    Some((json!(habit).as_str()?.to_owned(), value))
                })
                .collect();
            field.lock = Some(Lock {
                by: "candidates.habit".into(),
                values,
            });
        }
        if field.kind == Hotkey {
            field.help = "留空表示不綁定；ctrl、shift、alt（Option）、win（Command）加按鍵，例如 shift+space。".into();
        }
        if field.path == "theme.font" {
            field.help = "未安裝指定字型時使用目前平台的系統字型。".into();
        }
        if let Some((section, help)) = GENERAL
            .iter()
            .find(|(p, ..)| *p == field.path)
            .map(|(_, s, h)| (s, h))
        {
            field.section = (*section).into();
            field.help = help.replace("右 Alt", right_alt);
        }
        field.hidden = HIDDEN.contains(&field.path.as_str());
        if field.path == "mixed.english" {
            // 中文優先（與關）組字區顯示注音；英文優先顯示按下的字母、注音在旁邊的小視窗
            let typing = |v: &str| BTreeMap::from([("mixed.typing".to_string(), json!(v))]);
            field.also = BTreeMap::from([
                ("off".to_string(), typing("bopomofo")),
                ("chinese_first".to_string(), typing("bopomofo")),
                ("english_first".to_string(), typing("keys")),
            ]);
        }
        if let Some(&(_, min, max, step)) = RANGES.iter().find(|(p, ..)| *p == field.path) {
            field.range = Some(Range { min, max, step });
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
