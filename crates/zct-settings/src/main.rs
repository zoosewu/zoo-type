//! ZCT 設定程式（Slint）
//!
//! - `config.toml`、`apps.toml`：直接讀寫檔案，服務程序在 1 秒內套用（組字中的 App 等組字結束）
//! - 詞庫、統計、備份：透過 IPC 交給服務程序處理（服務程序是學習資料唯一的擁有者，避免互相覆蓋）

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{Model, ModelRc, SharedString, StandardListViewItem, VecModel};
use zct_core::config::{Color, Layout};
use zct_core::hotkey::{Binding, Hotkey};
use zct_core::paths::DataRoot;
use zct_core::{AppRule, AppRules, Config, PhraseAction, PhraseEntry};
use zct_ipc::pipe::Client;
use zct_ipc::{Command, PROTOCOL_VERSION, Request, Response};
use zct_preferences::{Action, Backend, Document, Reply, SettingsStore};

slint::include_modules!();

// ---- 設定檔 ↔ 介面 ----

fn parse_binding(s: &str) -> Result<Binding, String> {
    if s.trim().is_empty() {
        Ok(Binding(None))
    } else {
        s.parse::<Hotkey>().map(|h| Binding(Some(h)))
    }
}

/// 外觀頁的即時預覽
fn theme_from(c: &Config) -> Theme {
    let t = &c.theme;
    Theme {
        layout: (t.layout == Layout::Horizontal) as i32,
        font: t.font.clone().into(),
        size: t.font_size.round() as i32,
        background: t.background.to_string().into(),
        text: t.text.to_string().into(),
        number: t.number.to_string().into(),
        highlight: t.highlight.to_string().into(),
    }
}

/// 選中的 App 規則：設定目錄的 App 欄位（與 macOS 同一份）
fn rule_fields(rule: &AppRule) -> Vec<SharedField> {
    let catalog = zct_preferences::catalog(zct_preferences::Platform::Windows);
    let values = serde_json::to_value(rule).expect("serializable AppRule");
    let capabilities =
        serde_json::to_value(&catalog.capabilities).expect("serializable capabilities");
    catalog
        .app_fields
        .iter()
        .map(|f| field_row(f, &values, &capabilities, ""))
        .collect()
}

/// 畫面上的 App 欄位寫回規則（文字留空是沒設定；選項選「自訂」時不動）
fn rule_from_fields(rule: &AppRule, rows: &ModelRc<SharedField>) -> Result<AppRule, String> {
    use zct_preferences::FieldKind;
    let fields = zct_preferences::app_fields();
    let mut json = serde_json::to_value(rule).map_err(|e| e.to_string())?;
    for row in (0..rows.row_count()).filter_map(|i| rows.row_data(i)) {
        let field = fields
            .iter()
            .find(|f| f.path == row.path.as_str())
            .ok_or("App 規則欄位定義不一致")?;
        let value = match field.kind {
            FieldKind::Choice => match field.choices.get(row.selected as usize) {
                Some(choice) => choice.value.clone(),
                None => continue,
            },
            FieldKind::Bool => serde_json::Value::Bool(row.checked),
            _ => {
                let text = row.value.trim();
                if text.is_empty() && field.nullable {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(text.to_owned())
                }
            }
        };
        json[field.path.as_str()] = value;
    }
    serde_json::from_value(json).map_err(|e| e.to_string())
}

fn table_rows(rules: &[AppRule]) -> ModelRc<ModelRc<StandardListViewItem>> {
    let item = |s: &str| StandardListViewItem::from(SharedString::from(s));
    let v: Vec<ModelRc<StandardListViewItem>> = rules
        .iter()
        .map(|r| {
            ModelRc::new(VecModel::from(vec![
                item(&r.name),
                item(r.windows.as_deref().unwrap_or_default()),
                item(r.macos.as_deref().unwrap_or_default()),
            ]))
        })
        .collect();
    ModelRc::new(VecModel::from(v))
}

// ---- 服務程序連線 ----

struct Service {
    client: Option<Client>,
    readings: Option<Option<zct_core::readings::Readings>>,
}

impl Service {
    fn connect() -> Option<Client> {
        let mut c = Client::connect().ok()?;
        let hello = Request::Hello {
            version: PROTOCOL_VERSION,
            pid: std::process::id(),
            exe: "zoo-type-settings.exe".into(),
        };
        match c.call(&hello, Duration::from_secs(2)) {
            Ok(Response::Welcome { .. }) => Some(c),
            _ => None,
        }
    }

    /// 連不上時嘗試啟動執行檔旁的服務程序
    fn start() -> Service {
        if let Some(c) = Self::connect() {
            return Service {
                client: Some(c),
                readings: None,
            };
        }
        if let Some(exe) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("zoo-type-service.exe")))
            .filter(|p| p.exists())
        {
            let _ = std::process::Command::new(exe).spawn();
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(200));
                if let Some(c) = Self::connect() {
                    return Service {
                        client: Some(c),
                        readings: None,
                    };
                }
            }
        }
        Service {
            client: None,
            readings: None,
        }
    }

    fn transport(&mut self, req: Request) -> Result<Response, String> {
        if self.client.is_none() {
            self.client = Self::connect();
        }
        let client = self.client.as_mut().ok_or("輸入法服務未執行")?;
        match client.call(&req, Duration::from_secs(5)) {
            Ok(Response::Error(e)) => Err(e),
            Ok(r) => Ok(r),
            Err(e) => {
                self.client = None;
                Err(e.to_string())
            }
        }
    }
}

impl Backend for Service {
    fn execute(&mut self, action: Action) -> Result<Reply, String> {
        let request = match action {
            Action::SuggestReading { word } => {
                let readings = self.readings.get_or_insert_with(load_readings);
                return Ok(Reply::Reading {
                    value: readings
                        .as_ref()
                        .and_then(|r| r.reading_of(&word))
                        .unwrap_or_default(),
                });
            }
            Action::EditPhrase {
                original,
                word,
                bopomofo,
            } => return zct_preferences::edit_phrase(self, original, word, bopomofo),
            Action::ApiKeyStatus => {
                return Ok(Reply::ApiKeyStatus {
                    present: zct_ipc::secret::read_ai_key().is_some(),
                });
            }
            Action::WriteApiKey { key } => {
                let target = zct_ipc::secret::ai_api_key();
                if key.trim().is_empty() {
                    zct_ipc::secret::delete(&target);
                } else {
                    zct_ipc::secret::write(&target, key.trim()).map_err(|e| e.to_string())?;
                }
                return Ok(Reply::done());
            }
            Action::TestAi { config } => {
                let endpoint = zct_ai::Endpoint {
                    base_url: config.endpoint,
                    model: config.model,
                    api_key: zct_ipc::secret::read_ai_key(),
                    timeout: Duration::from_secs(config.timeout_seconds.max(1)),
                };
                return zct_ai::ping(&endpoint)
                    .map(|message| Reply::AiTest { message })
                    .map_err(|e| e.to_string());
            }
            Action::ListPhrases => Request::ListPhrases,
            Action::AddPhrase { word, bopomofo } => Request::AddPhrase { word, bopomofo },
            Action::Phrase {
                word,
                bopomofo,
                action,
            } => Request::Phrase {
                word,
                bopomofo,
                action,
            },
            Action::Stats => Request::Stats,
            Action::Backups => Request::Backups,
            Action::Restore { day } => Request::Restore { day },
            Action::SyncStatus => {
                let message = DataRoot::discover()
                    .and_then(|d| std::fs::read_to_string(d.local().join("sync-status.txt")).ok())
                    .unwrap_or_else(|| "尚未同步".into());
                return Ok(Reply::SyncStatus {
                    running: false,
                    message,
                });
            }
            Action::SyncNow => Request::SyncNow,
            Action::BackupNow => return Err("Windows 服務會定期備份，尚未提供立即備份命令".into()),
        };
        match self.transport(request)? {
            Response::Ok => Ok(Reply::done()),
            Response::Phrases(items) => Ok(Reply::Phrases {
                items: items
                    .into_iter()
                    .map(zct_preferences::PhraseView::from)
                    .collect(),
            }),
            Response::Stats(items) => Ok(Reply::Stats {
                items: items
                    .into_iter()
                    .map(zct_preferences::DayView::from)
                    .collect(),
            }),
            Response::Backups(items) => Ok(Reply::Backups { items }),
            other => Err(format!("非預期的設定管理回應：{other:?}")),
        }
    }
}
impl Service {
    // Existing Slint callbacks keep their view models; all administrative actions
    // go through the same platform-neutral Backend contract as the IMK frontend.
    fn call(&mut self, request: Request) -> Result<Response, String> {
        let action = match request {
            Request::ListPhrases => Action::ListPhrases,
            Request::AddPhrase { word, bopomofo } => Action::AddPhrase { word, bopomofo },
            Request::Phrase {
                word,
                bopomofo,
                action,
            } => Action::Phrase {
                word,
                bopomofo,
                action,
            },
            Request::Stats => Action::Stats,
            Request::Backups => Action::Backups,
            Request::Restore { day } => Action::Restore { day },
            Request::SyncNow => Action::SyncNow,
            other => return self.transport(other),
        };
        self.execute(action).map(|reply| match reply {
            Reply::Done { .. } => Response::Ok,
            Reply::Phrases { items } => {
                Response::Phrases(items.into_iter().map(|row| row.entry).collect())
            }
            Reply::Stats { items } => {
                Response::Stats(items.into_iter().map(|row| row.entry).collect())
            }
            Reply::Backups { items } => Response::Backups(items),
            Reply::ApiKeyStatus { .. }
            | Reply::AiTest { .. }
            | Reply::Reading { .. }
            | Reply::SyncStatus { .. } => {
                unreachable!("credential replies are never IPC messages")
            }
        })
    }
}

struct State {
    data: DataRoot,
    config: Config,
    revision: String,
    service: Service,
    phrases: Vec<PhraseEntry>,
    filter: String,
}

fn to_phrase(p: &PhraseEntry) -> Phrase {
    let row = zct_preferences::PhraseView::from(p.clone());
    Phrase {
        word: p.word.clone().into(),
        bopomofo: p.bopomofo.clone().into(),
        abbrev: row.kind == "abbrev",
        english: row.kind == "english",
        state: row.state,
    }
}

/// 詞庫下拉選單：從目前狀態到目標狀態要做的動作
fn state_action(from: i32, to: i32) -> Option<PhraseAction> {
    match (from, to) {
        (a, b) if a == b => None,
        (_, 1) => Some(PhraseAction::Pin),
        (_, 2) => Some(PhraseAction::Ban),
        (2, 0) => Some(PhraseAction::Unban),
        (_, 0) => Some(PhraseAction::Unpin),
        _ => None,
    }
}

/// 讀音資料（libchewing 的 tsi.csv）：安裝時在執行檔旁的 dict/，開發時在 assets/dict/
fn load_readings() -> Option<zct_core::readings::Readings> {
    let candidates = [
        std::env::var_os("ZCT_DICT").map(std::path::PathBuf::from),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("dict"))),
        Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/dict")),
    ];
    candidates
        .into_iter()
        .flatten()
        .map(|d| d.join("tsi.csv"))
        .find(|p| p.exists())
        .and_then(|p| zct_core::readings::Readings::load(&p).ok())
}

/// 只允許一個設定程式：已經開著時把那個視窗叫到前面，回傳 false
///
/// 變更資料位置後會自己重新開啟（`--notice`），那時舊的還在結束中，等它一下。
#[cfg(windows)]
fn single_instance(wait_for_old: bool) -> bool {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    use windows::core::{HSTRING, PCWSTR};
    let tries = if wait_for_old { 25 } else { 1 };
    for _ in 0..tries {
        unsafe {
            let mutex = format!(r"Local\{}-settings", zct_core::channel::CURRENT.dir);
            let Ok(handle) = CreateMutexW(None, false, &HSTRING::from(mutex)) else {
                return true;
            };
            if GetLastError() != ERROR_ALREADY_EXISTS {
                // 不關閉 handle：程序結束時系統自動釋放
                let _ = handle;
                return true;
            }
            let _ = CloseHandle(handle);
        }
        if wait_for_old {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    unsafe {
        let title = HSTRING::from(zct_core::channel::settings_title());
        if let Ok(hwnd) = FindWindowW(PCWSTR::null(), &title) {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
    false
}

#[cfg(not(windows))]
fn single_instance(_wait_for_old: bool) -> bool {
    true
}

/// 設定目錄的一個欄位 → 畫面上的一列
fn field_row(
    field: &zct_preferences::Field,
    values: &serde_json::Value,
    capabilities: &serde_json::Value,
    page_title: &str,
) -> SharedField {
    use zct_preferences::FieldKind;
    let locked = field.locked_value(values);
    let mut value = values;
    for key in field.path.split('.') {
        value = &value[key];
    }
    // 被操作習慣鎖定：顯示固定值、不能編輯
    let value = locked.unwrap_or(value);
    let range = field.range.filter(|_| field.integer);
    let chosen = match field.kind {
        FieldKind::Choice if locked.is_none() => field.chosen(values),
        _ => field.choices.iter().position(|c| &c.value == value),
    };
    SharedField {
        path: field.path.clone().into(),
        label: field.label.clone().into(),
        section: page_title.into(),
        header: SharedString::default(),
        kind: match field.kind {
            FieldKind::Bool => 0,
            FieldKind::Choice => 1,
            FieldKind::Number if range.is_some() => 3,
            FieldKind::Hotkey => 4,
            FieldKind::Color => 5,
            _ => 2,
        },
        placeholder: field.placeholder.clone().into(),
        // 沒設定（null）顯示空白
        value: match value {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
        .into(),
        checked: value.as_bool().unwrap_or(false),
        // 目前的設定不符合任何選項（手動改過設定檔）：多一個「自訂」，選著它儲存時不動
        options: ModelRc::new(VecModel::from(
            field
                .choices
                .iter()
                .map(|c| SharedString::from(&c.label))
                .chain(
                    chosen
                        .is_none()
                        .then(|| zct_preferences::CUSTOM_CHOICE.into()),
                )
                .collect::<Vec<_>>(),
        )),
        selected: chosen.unwrap_or(field.choices.len()) as i32,
        number: value.as_i64().unwrap_or_default() as i32,
        minimum: range.map_or(0, |r| r.min as i32),
        maximum: range.map_or(0, |r| r.max as i32),
        step: range.map_or(1, |r| r.step as i32),
        enabled: locked.is_none()
            && field
                .feature
                .as_ref()
                .is_none_or(|f| capabilities[f].as_bool().unwrap_or(false)),
        locked: locked.is_some(),
        help: field.help.clone().into(),
    }
}

/// 全部設定頁：目錄裡所有的欄位（被鎖定的顯示固定值、不能編輯）
fn shared_fields(config: &Config) -> Vec<SharedField> {
    let catalog = zct_preferences::catalog(zct_preferences::Platform::Windows);
    let values = serde_json::to_value(config).expect("serializable Config");
    let capabilities =
        serde_json::to_value(&catalog.capabilities).expect("serializable capabilities");
    catalog
        .fields
        .iter()
        .map(|field| {
            let title = catalog
                .pages
                .iter()
                .find(|p| p.id == field.page)
                .map(|p| p.title.as_str())
                .unwrap_or_default();
            field_row(field, &values, &capabilities, title)
        })
        .collect()
}

/// 一頁的欄位：分區標題、說明、進階都照設定目錄（與 macOS 同一份）。
/// 不在設定畫面與被操作習慣鎖定的欄位不顯示；`advanced`：進階展開
fn page_fields(config: &Config, page: &str, advanced: bool) -> Vec<SharedField> {
    let catalog = zct_preferences::catalog(zct_preferences::Platform::Windows);
    let values = serde_json::to_value(config).expect("serializable Config");
    let capabilities =
        serde_json::to_value(&catalog.capabilities).expect("serializable capabilities");
    let fields = catalog
        .fields
        .iter()
        .filter(|f| f.page == page && !f.hidden && f.locked_value(&values).is_none());
    let mut rows = Vec::new();
    let mut section = "";
    for field in fields.clone().filter(|f| !f.advanced) {
        let mut row = field_row(field, &values, &capabilities, "");
        if field.section != section {
            section = &field.section;
            row.header = section.into();
        }
        rows.push(row);
    }
    if fields.clone().any(|f| f.advanced) {
        rows.push(SharedField {
            kind: 9,
            label: if advanced { "▾ 進階" } else { "▸ 進階" }.into(),
            ..Default::default()
        });
        if advanced {
            rows.extend(
                fields
                    .filter(|f| f.advanced)
                    .map(|f| field_row(f, &values, &capabilities, "")),
            );
        }
    }
    rows
}

/// 畫面上的列寫回設定（依欄位路徑；被鎖定的列保留自己的設定）
fn apply_rows(config: &Config, rows: &ModelRc<SharedField>) -> Result<Config, String> {
    use zct_preferences::FieldKind;
    let catalog = zct_preferences::catalog(zct_preferences::Platform::Windows);
    let mut json = serde_json::to_value(config).map_err(|e| e.to_string())?;
    for row in (0..rows.row_count()).filter_map(|i| rows.row_data(i)) {
        if row.locked || row.kind == 9 {
            continue;
        }
        let field = catalog
            .fields
            .iter()
            .find(|f| f.path == row.path.as_str())
            .ok_or("設定欄位定義不一致")?;
        if field.kind == FieldKind::Choice {
            // 選項連同順便寫入的欄位；「自訂」不動
            for (path, value) in field.writes(row.selected as usize) {
                let keys: Vec<_> = path.split('.').collect();
                json[keys[0]][keys[1]] = value;
            }
            continue;
        }
        let value = match field.kind {
            FieldKind::Bool => serde_json::Value::Bool(row.checked),
            FieldKind::Number if row.kind == 3 => serde_json::Value::from(row.number),
            FieldKind::Number | FieldKind::Json => serde_json::from_str(row.value.as_str())
                .map_err(|e| format!("{}：{e}", field.label))?,
            _ => serde_json::Value::String(row.value.to_string()),
        };
        let keys: Vec<_> = field.path.split('.').collect();
        json[keys[0]][keys[1]] = value;
    }
    let config: Config = serde_json::from_value(json).map_err(|e| e.to_string())?;
    zct_preferences::validate(&config)
}

/// 由設定目錄產生的頁面：（頁面, 讀列表, 換列表）
type PageModel = (
    &'static str,
    fn(&SettingsWindow) -> ModelRc<SharedField>,
    fn(&SettingsWindow, ModelRc<SharedField>),
);

const PAGES: [PageModel; 5] = [
    (
        "general",
        SettingsWindow::get_general_fields,
        SettingsWindow::set_general_fields,
    ),
    (
        "hotkeys",
        SettingsWindow::get_hotkey_fields,
        SettingsWindow::set_hotkey_fields,
    ),
    (
        "appearance",
        SettingsWindow::get_appearance_fields,
        SettingsWindow::set_appearance_fields,
    ),
    (
        "sync",
        SettingsWindow::get_sync_fields,
        SettingsWindow::set_sync_fields,
    ),
    (
        "ai",
        SettingsWindow::get_ai_fields,
        SettingsWindow::set_ai_fields,
    ),
];

/// 各頁畫面上的值寫回設定
fn apply_pages(ui: &SettingsWindow, config: &Config) -> Result<Config, String> {
    PAGES
        .iter()
        .try_fold(config.clone(), |c, (_, get, _)| apply_rows(&c, &get(ui)))
}

/// 依設定重新產生各頁的列表與外觀預覽
fn show_pages(ui: &SettingsWindow, config: &Config) {
    for (page, _, set) in PAGES {
        let advanced = page == "general" && ui.get_show_advanced();
        set(
            ui,
            ModelRc::new(VecModel::from(page_fields(config, page, advanced))),
        );
    }
    ui.set_theme(theme_from(config));
}

/// 換了選項或展開／收合進階：畫面上的值寫進 `config`（暫存，切頁不會遺失），重新產生列表
fn rebuild_general(ui: &SettingsWindow, config: &mut Config, toggle: bool) -> Result<(), String> {
    *config = apply_pages(ui, config)?;
    if toggle {
        ui.set_show_advanced(!ui.get_show_advanced());
    }
    show_pages(ui, config);
    Ok(())
}

fn show_primary_settings(ui: &SettingsWindow, config: &Config) {
    show_pages(ui, config);
    ui.set_auto_update(config.update.auto_check);
    let catalog = zct_preferences::catalog(zct_preferences::Platform::Windows);
    if let Some(page) = catalog.pages.iter().find(|p| p.id == "hotkeys") {
        ui.set_hotkeys_help(page.help.clone().into());
    }
}
fn collect_primary_settings(ui: &SettingsWindow, original: &Config) -> Result<Config, String> {
    let mut config = apply_pages(ui, original)?;
    config.update.auto_check = ui.get_auto_update();
    Ok(config)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !single_instance(std::env::args().any(|a| a == "--notice")) {
        return Ok(());
    }
    // 改名前的 %APPDATA%\zct 搬到 %APPDATA%\ZooType（服務程序也會做，先開哪個都一樣）
    let (data, _) = zct_core::paths::prepare_data()?;
    let ui = SettingsWindow::new()?;
    ui.set_window_title(zct_core::channel::settings_title().into());
    ui.set_app_name(zct_core::channel::CURRENT.name.into());
    ui.set_official(zct_core::channel::CURRENT.id == zct_core::channel::OFFICIAL.id);
    // --page N：直接開啟某一頁（系統匣與開發截圖用）
    // --notice 文字：開啟後在狀態列顯示（變更資料位置後重新開啟時用）
    let mut notice: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--page"
            && let Some(n) = args.next().and_then(|n| n.parse().ok())
        {
            ui.set_page(n);
        } else if a == "--notice" {
            notice = args.next();
        }
    }

    let store = SettingsStore::new(data.clone());
    let (config, config_error) = match store.config() {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e.to_string())),
    };
    let (rules, apps_error) = match store.apps() {
        Ok(a) => (a, None),
        Err(e) => (AppRules::default(), Some(e.to_string())),
    };
    ui.set_shared_fields(ModelRc::new(VecModel::from(shared_fields(&config))));
    show_primary_settings(&ui, &config);
    ui.set_has_api_key(zct_ipc::secret::read_ai_key().is_some());
    let apps = Rc::new(RefCell::new(rules.apps.clone()));
    ui.set_apps_rows(table_rows(&apps.borrow()));

    let service = Service::start();
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    ui.set_online(service.client.is_some());
    ui.set_data_path(data.root().display().to_string().into());
    ui.set_data_path_locked(std::env::var_os(zct_core::paths::ENV_HOME).is_some());
    ui.set_status(
        config_error
            .or(apps_error)
            .map(|e| format!("{e}（儲存會以目前畫面上的設定覆寫）"))
            .or(notice)
            .unwrap_or_default()
            .into(),
    );

    let revision = store.revision()?;
    let state = Rc::new(RefCell::new(State {
        revision,
        data,
        config,
        service,
        phrases: Vec::new(),
        filter: String::new(),
    }));

    ui.on_parse_color(|s| {
        s.parse::<Color>()
            .map(|c| {
                let (r, g, b) = c.rgb();
                slint::Color::from_rgb_u8(r, g, b)
            })
            .unwrap_or(slint::Color::from_argb_u8(0, 0, 0, 0))
    });
    ui.on_hotkey_valid(|s| parse_binding(&s).is_ok());
    ui.on_is_english_word(|s| zct_core::abbrev::is_abbrev(s.trim()));
    {
        let state = state.clone();
        ui.on_suggest_reading(move |word| {
            match state.borrow_mut().service.execute(Action::SuggestReading {
                word: word.to_string(),
            }) {
                Ok(Reply::Reading { value }) => value.into(),
                _ => SharedString::default(),
            }
        });
    }

    // A catalog-generated page exposes every Config field, including future additions.
    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_open_shared(move || {
            let ui = ui_weak.unwrap();
            if ui.get_page() == 9 {
                return;
            }
            let st = state.borrow();
            match collect_primary_settings(&ui, &st.config) {
                Ok(config) => {
                    ui.set_shared_fields(ModelRc::new(VecModel::from(shared_fields(&config))));
                    ui.set_page(9);
                }
                Err(e) => ui.set_status(e.into()),
            }
        });
    }
    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_leave_shared(move || {
            let ui = ui_weak.unwrap();
            if ui.get_page() != 9 {
                return true;
            }
            let mut st = state.borrow_mut();
            match apply_rows(&st.config, &ui.get_shared_fields()) {
                Ok(config) => {
                    show_primary_settings(&ui, &config);
                    st.config = config;
                    true
                }
                Err(e) => {
                    ui.set_status(e.into());
                    false
                }
            }
        });
    }

    // 全部設定頁換了選項：依畫面上的值重新套用鎖定（只更新有鎖定的列，正在操作的那一列不動）
    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_shared_choice_changed(move || {
            let ui = ui_weak.unwrap();
            let rows = ui.get_shared_fields();
            let st = state.borrow();
            let Ok(config) = apply_rows(&st.config, &rows) else {
                return;
            };
            let catalog = zct_preferences::catalog(zct_preferences::Platform::Windows);
            for (i, row) in shared_fields(&config).into_iter().enumerate() {
                if catalog.fields.get(i).is_some_and(|f| f.lock.is_some()) {
                    rows.set_row_data(i, row);
                }
            }
        });
    }

    // 一般頁換了選項（例如操作習慣）或展開／收合進階：用畫面上的值重新產生列表
    for toggle in [false, true] {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        let rebuild = move || {
            let ui = ui_weak.unwrap();
            let mut st = state.borrow_mut();
            if let Err(e) = rebuild_general(&ui, &mut st.config, toggle) {
                ui.set_status(e.into());
            }
        };
        if toggle {
            ui.on_toggle_advanced(rebuild);
        } else {
            ui.on_general_changed(rebuild);
        }
    }

    // 外觀頁改了值：更新預覽（不重新產生列表，打字時才不會失去焦點；格式還不對時先不更新）
    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_appearance_edited(move || {
            let ui = ui_weak.unwrap();
            if let Ok(config) = apply_pages(&ui, &state.borrow().config) {
                ui.set_theme(theme_from(&config));
            }
        });
    }

    // 儲存設定
    {
        let (ui_weak, state, apps) = (ui.as_weak(), state.clone(), apps.clone());
        ui.on_save(move || {
            let ui = ui_weak.unwrap();
            let mut st = state.borrow_mut();
            let result = (|| {
                let config = if ui.get_page() == 9 {
                    apply_rows(&st.config, &ui.get_shared_fields())?
                } else {
                    collect_primary_settings(&ui, &st.config)?
                };
                let document = Document {
                    config,
                    apps: AppRules {
                        apps: apps.borrow().clone(),
                    },
                };
                SettingsStore::new(st.data.clone()).save(&document, Some(&st.revision))
            })();
            match result {
                Ok(snapshot) => {
                    st.config = snapshot.document.config;
                    st.revision = snapshot.revision;
                    show_primary_settings(&ui, &st.config);
                    ui.set_status("已儲存。外觀立即生效，其他設定在切換 App 後生效".into());
                }
                Err(e) => ui.set_status(format!("無法儲存：{e}").into()),
            }
        });
    }

    // App 規則：表單由設定目錄的 App 欄位產生
    {
        let (ui_weak, apps) = (ui.as_weak(), apps.clone());
        ui.on_select_app(move |i| {
            let ui = ui_weak.unwrap();
            if let Some(rule) = apps.borrow().get(i as usize) {
                ui.set_app_fields(ModelRc::new(VecModel::from(rule_fields(rule))));
            }
        });
    }
    {
        let (ui_weak, apps) = (ui.as_weak(), apps.clone());
        ui.on_apply_app(move || {
            let ui = ui_weak.unwrap();
            let Ok(i) = usize::try_from(ui.get_app_index()) else {
                return;
            };
            let mut rules = apps.borrow_mut();
            let Some(rule) = rules.get_mut(i) else { return };
            match rule_from_fields(rule, &ui.get_app_fields()) {
                Ok(edited) => {
                    *rule = edited;
                    ui.set_apps_rows(table_rows(&rules));
                    ui.set_status("已套用，按「儲存設定」寫入檔案".into());
                }
                Err(e) => ui.set_status(e.into()),
            }
        });
    }
    {
        let (ui_weak, apps) = (ui.as_weak(), apps.clone());
        ui.on_add_app(move || {
            let ui = ui_weak.unwrap();
            let rule = AppRule {
                name: "新規則".into(),
                ..Default::default()
            };
            ui.set_app_fields(ModelRc::new(VecModel::from(rule_fields(&rule))));
            let mut rules = apps.borrow_mut();
            rules.push(rule);
            ui.set_apps_rows(table_rows(&rules));
            ui.set_app_index(rules.len() as i32 - 1);
        });
    }
    {
        let (ui_weak, apps) = (ui.as_weak(), apps.clone());
        ui.on_remove_app(move |i| {
            let ui = ui_weak.unwrap();
            let mut rules = apps.borrow_mut();
            if let Ok(i) = usize::try_from(i)
                && i < rules.len()
            {
                rules.remove(i);
                ui.set_apps_rows(table_rows(&rules));
                ui.set_app_index(-1);
            }
        });
    }

    // 詞庫、統計、備份
    let show_phrases = {
        let ui_weak = ui.as_weak();
        move |st: &State| {
            let ui = ui_weak.unwrap();
            let f = st.filter.trim();
            let list: Vec<Phrase> = st
                .phrases
                .iter()
                .filter(|p| f.is_empty() || p.word.contains(f) || p.bopomofo.contains(f))
                .map(to_phrase)
                .collect();
            ui.set_phrases(ModelRc::new(VecModel::from(list)));
            // 清單換了，編輯中的列號就不對了
            ui.set_phrase_edit_row(-1);
        }
    };
    // 「版本說明」「手動下載」要開的 Release 頁面
    let update_page = Rc::new(RefCell::new(String::new()));
    let refresh = {
        let (ui_weak, state, show_phrases) = (ui.as_weak(), state.clone(), show_phrases.clone());
        let update_page = update_page.clone();
        move || {
            let ui = ui_weak.unwrap();
            let mut st = state.borrow_mut();
            let mut error = None;
            match st.service.call(Request::ListPhrases) {
                Ok(Response::Phrases(p)) => st.phrases = p,
                Ok(_) => {}
                Err(e) => error = Some(e),
            }
            show_phrases(&st);
            if let Ok(Response::Stats(stats)) = st.service.call(Request::Stats) {
                let recent: Vec<_> = stats.iter().rev().take(30).rev().collect();
                let days: Vec<Day> = recent
                    .iter()
                    .map(|d| Day {
                        label: d.day.clone().into(),
                        chars: d.chars as i32,
                        corrections: d.corrections as i32,
                        accuracy: accuracy(d.chars, d.corrections),
                    })
                    .collect();
                let (chars, corr) = recent
                    .iter()
                    .fold((0, 0), |(c, k), d| (c + d.chars, k + d.corrections));
                let summary = match recent.last() {
                    Some(t) => format!(
                        "今天 {} 字、修正 {} 次（{:.1}%）　近 {} 天 {} 字、修正 {} 次（{:.1}%）",
                        t.chars,
                        t.corrections,
                        accuracy(t.chars, t.corrections) * 100.0,
                        recent.len(),
                        chars,
                        corr,
                        accuracy(chars, corr) * 100.0
                    ),
                    None => "還沒有統計資料".into(),
                };
                ui.set_days(ModelRc::new(VecModel::from(days)));
                ui.set_summary(summary.into());
            }
            if let Ok(Response::Backups(b)) = st.service.call(Request::Backups) {
                let b: Vec<SharedString> = b.into_iter().map(Into::into).collect();
                ui.set_backups(ModelRc::new(VecModel::from(b)));
            }
            if let Ok(Response::Status { update, .. }) = st.service.call(Request::Status) {
                show_update(&ui, &update, &update_page);
            }
            ui.set_online(st.service.client.is_some());
            let sync_status = std::fs::read_to_string(st.data.local().join("sync-status.txt"))
                .unwrap_or_else(|_| "尚未同步".into());
            ui.set_sync_status(sync_status.into());
            if let Some(e) = error {
                ui.set_status(e.into());
            }
        }
    };
    ui.on_refresh(refresh.clone());
    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_save_api_key(move |key| {
            let ui = ui_weak.unwrap();
            let mut st = state.borrow_mut();
            let message = match st.service.execute(Action::WriteApiKey {
                key: key.to_string(),
            }) {
                Ok(_) => {
                    if key.is_empty() {
                        "已清除 API Key".into()
                    } else {
                        "API Key 已存入平台憑證庫".into()
                    }
                }
                Err(e) => format!("無法儲存 API Key：{e}"),
            };
            if let Ok(Reply::ApiKeyStatus { present }) = st.service.execute(Action::ApiKeyStatus) {
                ui.set_has_api_key(present);
            }
            ui.set_ai_status(message.into());
        });
    }
    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_test_ai(move || {
            let ui = ui_weak.unwrap();
            // 用畫面上的 AI 設定測試（還沒儲存也可以）
            let config = match apply_rows(&state.borrow().config, &ui.get_ai_fields()) {
                Ok(config) => config,
                Err(e) => {
                    ui.set_ai_status(e.into());
                    return;
                }
            };
            let message = match state
                .borrow_mut()
                .service
                .execute(Action::TestAi { config: config.ai })
            {
                Ok(Reply::AiTest { message }) => format!("連線成功，模型回應：{message}"),
                Ok(_) => "管理回應種類錯誤".into(),
                Err(e) => e,
            };
            ui.set_ai_status(message.into());
        });
    }
    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_sync_now(move || {
            let r = state.borrow_mut().service.call(Request::SyncNow);
            ui_weak.unwrap().set_sync_status(
                match r {
                    Ok(_) => "同步中…（完成後按「重新整理狀態」）".to_string(),
                    Err(e) => e,
                }
                .into(),
            );
        });
    }
    // 檢查、下載、安裝都在服務程序背景進行：送出後每秒問一次走到哪一步（與輸入法圖示選單同一份狀態）
    let update_poll = slint::Timer::default();
    {
        let (ui_weak, state, page) = (ui.as_weak(), state.clone(), update_page.clone());
        update_poll.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(1),
            move || {
                let Some(ui) = ui_weak.upgrade() else { return };
                if !ui.get_checking_update() {
                    return;
                }
                match state.borrow_mut().service.call(Request::Status) {
                    Ok(Response::Status { update, .. }) => show_update(&ui, &update, &page),
                    Ok(_) => {}
                    Err(e) => {
                        ui.set_checking_update(false);
                        show_update_status(&ui, &e, None, None);
                    }
                }
            },
        );
    }
    // 送出更新指令後，狀態由上面的計時器接手
    let send_update = {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        move |command: Command| {
            let ui = ui_weak.unwrap();
            match state.borrow_mut().service.call(Request::Command(command)) {
                Ok(_) => ui.set_checking_update(true),
                Err(e) => show_update_status(&ui, &e, None, None),
            }
        }
    };
    {
        let send = send_update.clone();
        ui.on_check_update(move || send(Command::CheckUpdate));
    }
    {
        let (send, page) = (send_update.clone(), update_page.clone());
        ui.on_update_action(move |id| match id.as_str() {
            "install" => send(Command::InstallUpdate),
            "cancel" => send(Command::CancelUpdate),
            "retry" => send(Command::CheckUpdate),
            // 版本說明、手動下載：開啟 Release 頁面
            _ => open_url(&page.borrow()),
        });
    }
    ui.on_open_url(|url| open_url(&url));
    show_other_projects(&ui, &state.borrow().data);
    refresh();
    {
        let (state, show_phrases) = (state.clone(), show_phrases.clone());
        ui.on_filter_phrases(move |t| {
            let mut st = state.borrow_mut();
            st.filter = t.to_string();
            show_phrases(&st);
        });
    }
    {
        let (ui_weak, state, refresh) = (ui.as_weak(), state.clone(), refresh.clone());
        ui.on_add_phrase(move |word, reading| {
            let r = state.borrow_mut().service.call(Request::AddPhrase {
                word: word.to_string(),
                bopomofo: reading.to_string(),
            });
            refresh();
            ui_weak.unwrap().set_status(match r {
                Ok(_) => format!("已加入：{word}").into(),
                Err(e) => e.into(),
            });
        });
    }
    {
        let (ui_weak, state, refresh) = (ui.as_weak(), state.clone(), refresh.clone());
        ui.on_phrase_state(move |p, to| {
            let Some(action) = state_action(p.state, to) else {
                return;
            };
            let r = state.borrow_mut().service.call(Request::Phrase {
                word: p.word.to_string(),
                bopomofo: p.bopomofo.to_string(),
                action,
            });
            refresh();
            if let Err(e) = r {
                ui_weak.unwrap().set_status(e.into());
            }
        });
    }
    {
        let (ui_weak, state, refresh) = (ui.as_weak(), state.clone(), refresh.clone());
        ui.on_remove_phrase(move |p| {
            let r = state.borrow_mut().service.call(Request::Phrase {
                word: p.word.to_string(),
                bopomofo: p.bopomofo.to_string(),
                action: PhraseAction::Remove,
            });
            refresh();
            ui_weak.unwrap().set_status(match r {
                Ok(_) => format!("已移除：{}", p.word).into(),
                Err(e) => e.into(),
            });
        });
    }
    {
        // 編輯：加入新的那筆、再移除原本的；封鎖的詞換了寫法仍然封鎖
        let (ui_weak, state, refresh) = (ui.as_weak(), state.clone(), refresh.clone());
        ui.on_update_phrase(move |old, word, reading| {
            let (word, reading) = (word.trim().to_owned(), reading.trim().to_owned());
            if old.word == word.as_str() && old.bopomofo == reading.as_str() {
                return;
            }
            let original = PhraseEntry {
                word: old.word.to_string(),
                bopomofo: old.bopomofo.to_string(),
                boost: if old.state == 2 {
                    PhraseEntry::BANNED
                } else if old.state == 1 {
                    PhraseEntry::PINNED
                } else {
                    PhraseEntry::NORMAL
                },
            };
            let r = state.borrow_mut().service.execute(Action::EditPhrase {
                original,
                word: word.clone(),
                bopomofo: reading,
            });
            refresh();
            ui_weak.unwrap().set_status(match r {
                Ok(_) => format!("已更新：{word}").into(),
                Err(e) => e.into(),
            });
        });
    }
    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_restore(move |day| {
            let ui = ui_weak.unwrap();
            let r = state.borrow_mut().service.call(Request::Restore {
                day: day.to_string(),
            });
            ui.set_restore_pending("".into());
            match r {
                Ok(_) => {
                    // 服務程序會自行結束，下次打字時重新啟動並載入還原的資料
                    state.borrow_mut().service.client = None;
                    ui.set_online(false);
                    ui.set_status(format!("已還原到 {day}，下次打字時生效").into());
                }
                Err(e) => ui.set_status(format!("還原失敗：{e}").into()),
            }
        });
    }

    {
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_change_data_root(move || {
            let ui = ui_weak.unwrap();
            let current = state.borrow().data.root().to_path_buf();
            let start = current.parent().unwrap_or(&current).to_path_buf();
            let Some(new) = pick_folder(&start) else {
                return;
            };
            let Some(pointer) = zct_core::paths::pointer_dir() else {
                return;
            };
            // 先讓服務程序存檔並停止寫入，複製時資料才不會變動
            let online = state
                .borrow_mut()
                .service
                .call(Request::Command(Command::PrepareMove))
                .is_ok();
            let result = zct_preferences::relocate_directory(&pointer, &current, &new);
            // 不論成功與否都重新啟動服務程序（成功：讀新位置；失敗：解除停止寫入）
            if online {
                let _ = state
                    .borrow_mut()
                    .service
                    .call(Request::Command(Command::Restart));
            }
            match result {
                Ok(mode) => {
                    let what = match mode {
                        zct_core::paths::ChangeRoot::Copied => "已把資料複製到",
                        zct_core::paths::ChangeRoot::Adopted => "已改用",
                    };
                    let msg = format!(
                        "{what} {}（原本的資料保留在 {}）",
                        new.display(),
                        current.display()
                    );
                    // 重新開啟設定程式，讀新位置的設定
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe)
                            .args(["--page", "6", "--notice", &msg])
                            .spawn();
                    }
                    std::process::exit(0);
                }
                Err(e) => ui.set_status(format!("無法變更資料位置：{e}").into()),
            }
        });
    }

    ui.run()?;
    Ok(())
}

/// Windows 原生的資料夾選擇視窗；取消時回傳 None
#[cfg(windows)]
fn pick_folder(start: &std::path::Path) -> Option<std::path::PathBuf> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::core::HSTRING;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .ok()?;
        let _ = dialog.SetTitle(&HSTRING::from("選擇資料位置"));
        if let Ok(item) =
            SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start.as_os_str()), None)
        {
            let _ = dialog.SetFolder(&item);
        }
        // 按取消時 Show 回傳錯誤
        dialog.Show(None).ok()?;
        let path = dialog
            .GetResult()
            .ok()?
            .GetDisplayName(SIGDN_FILESYSPATH)
            .ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as _));
        text.map(std::path::PathBuf::from)
    }
}

#[cfg(not(windows))]
fn pick_folder(_start: &std::path::Path) -> Option<std::path::PathBuf> {
    None
}

/// 更新走到哪一步（與輸入法圖示選單同一份狀態）；`page` 記下「版本說明」「手動下載」要開的頁面
fn show_update(ui: &SettingsWindow, update: &zct_core::menu::UpdateState, page: &RefCell<String>) {
    ui.set_checking_update(update.busy());
    let status = update.status();
    show_update_status(
        ui,
        status.as_ref().map_or("", |s| &s.text),
        status.as_ref().and_then(|s| s.detail.as_deref()),
        status.as_ref().and_then(|s| s.progress),
    );
    let actions: Vec<UpdateAction> = update
        .buttons()
        .iter()
        .map(|b| UpdateAction {
            id: serde_json::to_value(b)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
                .into(),
            label: b.label().into(),
        })
        .collect();
    ui.set_update_actions(ModelRc::new(VecModel::from(actions)));
    *page.borrow_mut() = update.release_page();
}

/// 「關於」頁的狀態列（文字、次要文字、下載進度）
fn show_update_status(ui: &SettingsWindow, text: &str, detail: Option<&str>, progress: Option<u8>) {
    ui.set_update_status(text.into());
    ui.set_update_detail(detail.unwrap_or_default().into());
    ui.set_update_progress(progress.map_or(-1, i32::from));
}

/// 用預設瀏覽器開啟網址
fn open_url(url: &str) {
    if !url.starts_with("https://") {
        return;
    }
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(not(windows))]
    let r = std::process::Command::new("open").arg(url).spawn();
    if let Err(e) = r {
        eprintln!("無法開啟 {url}：{e}");
    }
}

/// 作者的其他專案：在背景讀 apex-code 的 API，讀取失敗就不顯示這個區塊
fn show_other_projects(ui: &SettingsWindow, data: &DataRoot) {
    let ui_weak = ui.as_weak();
    let cache = data.local().join("cache").join("projects");
    std::thread::spawn(move || {
        let Ok(projects) = zct_host::projects::fetch(&cache) else {
            return;
        };
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let items: Vec<OtherProject> = projects
                .into_iter()
                .map(|p| {
                    let cover = p
                        .cover
                        .as_deref()
                        .and_then(|c| slint::Image::load_from_path(c).ok());
                    OtherProject {
                        name: p.name.into(),
                        tagline: p.tagline.into(),
                        url: p.url.into(),
                        has_cover: cover.is_some(),
                        cover: cover.unwrap_or_default(),
                    }
                })
                .collect();
            ui.set_other_projects(ModelRc::new(VecModel::from(items)));
        });
    });
}

/// 首選正確率：沒有修正的字數比例
fn accuracy(chars: u64, corrections: u64) -> f32 {
    zct_preferences::accuracy(chars, corrections) as f32
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_catalog_form_preserves_all_config_fields() {
        let mut config = Config::default();
        config.theme.font_size = 16.5;
        config
            .punctuation
            .full
            .insert("shift+,".into(), "，".into());
        config.ai.model = "shared-model".into();
        config.sync.remote = "git@example.com:private/input.git".into();
        let rows = ModelRc::new(VecModel::from(shared_fields(&config)));
        let roundtrip = apply_rows(&config, &rows).unwrap();
        assert_eq!(roundtrip, config);
        // 外觀頁：小數的字級原樣保留
        let appearance = ModelRc::new(VecModel::from(page_fields(&config, "appearance", false)));
        assert_eq!(
            apply_rows(&config, &appearance).unwrap().theme.font_size,
            16.5
        );
    }

    use super::*;
    use zct_core::config::{
        AfterSelect, EnglishWords, GameMode, Habit, InputMode, OnFocus, Output, SelectSide,
        ShiftToggle, TypingDisplay, Width,
    };

    fn rows(fields: Vec<SharedField>) -> ModelRc<SharedField> {
        ModelRc::new(VecModel::from(fields))
    }

    fn row(rows: &ModelRc<SharedField>, path: &str) -> Option<SharedField> {
        (0..rows.row_count())
            .filter_map(|i| rows.row_data(i))
            .find(|r| r.path == path)
    }

    #[test]
    fn habit_locks_selection_fields_but_keeps_own_values() {
        let mut config = Config::default();
        config.candidates.habit = Habit::Windows;
        config.candidates.select = SelectSide::After;
        config.candidates.after_select = AfterSelect::Backward;
        // 全部設定頁：顯示固定值、不能編輯
        let all = rows(shared_fields(&config));
        let select = row(&all, "candidates.select").unwrap();
        assert!(select.locked && !select.enabled);
        assert_eq!(select.selected, 1, "顯示 Windows 的「游標前的詞」");
        assert_eq!(row(&all, "candidates.after_select").unwrap().selected, 0);
        assert_eq!(
            apply_rows(&config, &all).unwrap(),
            config,
            "自己的設定不會被固定值蓋掉"
        );
        // 一般頁：不顯示，存檔保留自己的設定
        let general = rows(page_fields(&config, "general", true));
        assert!(row(&general, "candidates.select").is_none());
        assert!(row(&general, "candidates.after_select").is_none());
        assert_eq!(apply_rows(&config, &general).unwrap(), config);
        // 切回自訂：顯示自己的設定
        let mut custom = config.clone();
        custom.candidates.habit = Habit::Custom;
        let general = rows(page_fields(&custom, "general", false));
        let select = row(&general, "candidates.select").unwrap();
        assert!(!select.locked && select.enabled);
        assert_eq!(select.selected, 0, "自己的「游標後的詞」");
    }

    #[test]
    fn english_choice_also_sets_typing_and_shows_custom_for_hand_edits() {
        let field = |config: &Config| {
            page_fields(config, "general", false)
                .into_iter()
                .find(|r| r.path == "mixed.english")
                .unwrap()
        };
        let mut config = Config::default();
        assert_eq!(field(&config).selected, 1, "中文優先");
        assert!(
            page_fields(&config, "general", true).iter().all(|r| ![
                "mixed.typing",
                "mixed.english_candidate",
                "mixed.auto_space"
            ]
            .contains(&r.path.as_str())),
            "這三項不在設定畫面"
        );
        // 選英文優先：同時寫入組字區顯示字母
        let mut row = field(&config);
        row.selected = 2;
        let saved = apply_rows(&config, &rows(vec![row])).unwrap();
        assert_eq!(saved.mixed.english, EnglishWords::EnglishFirst);
        assert_eq!(saved.mixed.typing, TypingDisplay::Keys);
        // 手動改成中文優先＋字母：顯示「自訂」，儲存時兩個欄位都不動
        config.mixed.typing = TypingDisplay::KeysAndChars;
        let row = field(&config);
        assert_eq!(row.selected, 3);
        assert_eq!(row.options.row_data(3).unwrap(), "自訂");
        assert_eq!(apply_rows(&config, &rows(vec![row])).unwrap(), config);
    }

    #[test]
    fn general_page_follows_the_catalog_sections_and_advanced() {
        let config = Config::default();
        let basic = page_fields(&config, "general", false);
        let headers: Vec<_> = basic
            .iter()
            .filter(|r| !r.header.is_empty())
            .map(|r| r.header.to_string())
            .collect();
        assert_eq!(headers, ["輸入", "候選字", "中英混打"]);
        assert_eq!(basic.last().unwrap().kind, 9, "最後是進階展開列");
        assert!(basic.iter().all(|r| r.path != "input.output"), "進階收合");
        let all = page_fields(&config, "general", true);
        assert!(all.iter().any(|r| r.path == "input.output"));
        let per_page = all
            .iter()
            .find(|r| r.path == "candidates.per_page")
            .unwrap();
        assert_eq!(
            (per_page.kind, per_page.minimum, per_page.maximum),
            (3, 1, 10)
        );
    }

    #[test]
    fn config_roundtrips_through_ui_structs() {
        let mut c = Config::default();
        c.input.default_mode = InputMode::English;
        c.input.output = Output::Simplified;
        c.toggle.shift = ShiftToggle::Right;
        c.toggle.shift_tap_max_ms = 450;
        c.candidates.per_page = 7;
        c.detect.fullscreen = false;
        c.mixed.english = EnglishWords::EnglishFirst;
        c.mixed.typing = TypingDisplay::Keys;
        c.hotkeys.output = Binding(Some("ctrl+alt+s".parse().unwrap()));
        c.hotkeys.pin = Binding(None);
        c.theme.layout = Layout::Horizontal;
        c.theme.background = Color(0x202020);
        c.mixed.auto_space = false;
        c.hotkeys.emoji = Binding(None);
        c.sync.enabled = true;
        c.sync.remote = "git@example.com:me/zct.git".into();
        c.sync.interval_minutes = 30;
        c.ai.enabled = true;
        c.ai.model = "qwen3.5:2b".into();
        c.hotkeys.ai = Binding(None);

        // 每一頁畫面上的值寫回預設設定：得到原本的設定
        let back = PAGES.iter().fold(Config::default(), |back, (page, ..)| {
            apply_rows(&back, &rows(page_fields(&c, page, true))).unwrap()
        });
        assert_eq!(back, c);
    }

    #[test]
    fn invalid_ui_values_are_rejected() {
        let c = Config::default();
        let edited = |page: &str, path: &str, value: &str| {
            let rows = rows(page_fields(&c, page, false));
            let i = (0..rows.row_count())
                .find(|&i| rows.row_data(i).unwrap().path == path)
                .unwrap();
            let mut row = rows.row_data(i).unwrap();
            row.value = value.into();
            rows.set_row_data(i, row);
            apply_rows(&c, &rows)
        };
        assert!(edited("hotkeys", "hotkeys.game_mode", "ctrl+bogus").is_err());
        assert!(edited("appearance", "theme.text", "紅色").is_err());
        assert!(edited("hotkeys", "hotkeys.game_mode", "ctrl+shift+f9").is_ok());
    }

    #[test]
    fn app_rule_roundtrips() {
        let rule = AppRule {
            name: "VS Code".into(),
            windows: Some("Code.exe".into()),
            macos: None,
            default_mode: Some(InputMode::English),
            punctuation: Some(Width::Half),
            output: None,
            on_focus: Some(OnFocus::Reset),
            game_mode: Some(GameMode::Keep),
            detect: Some(false),
            composition_bar: Some(true),
        };
        // 表單（設定目錄的 App 欄位）讀進來再寫回：得到原本的規則
        let roundtrip = |r: &AppRule| {
            rule_from_fields(
                &AppRule::default(),
                &ModelRc::new(VecModel::from(rule_fields(r))),
            )
            .unwrap()
        };
        assert_eq!(roundtrip(&rule), rule);
        assert_eq!(roundtrip(&AppRule::default()), AppRule::default());
    }

    #[test]
    fn dropdown_maps_to_actions() {
        assert_eq!(state_action(0, 0), None);
        assert_eq!(state_action(1, 0), Some(PhraseAction::Unpin));
        assert_eq!(state_action(2, 0), Some(PhraseAction::Unban));
        assert_eq!(state_action(2, 1), Some(PhraseAction::Pin));
        assert_eq!(state_action(0, 2), Some(PhraseAction::Ban));
    }

    #[test]
    fn readings_suggest_common_pronunciation() {
        let Some(r) = load_readings() else { return };
        assert_eq!(r.reading_of("天氣").as_deref(), Some("ㄊㄧㄢ ㄑㄧˋ"));
        assert_eq!(r.reading_of("銀行").as_deref(), Some("ㄧㄣˊ ㄏㄤˊ"));
    }

    #[test]
    fn accuracy_is_bounded() {
        assert_eq!(accuracy(0, 0), 1.0);
        assert_eq!(accuracy(100, 5), 0.95);
        assert_eq!(accuracy(1, 9), 0.0);
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use i_slint_backend_testing::ElementHandle;
    use slint::platform::{Key, WindowEvent};

    fn key(ui: &SettingsWindow, text: impl Into<SharedString>) {
        let text = text.into();
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }

    fn field(ui: &SettingsWindow, placeholder: &str) -> ElementHandle {
        i_slint_backend_testing::ElementQuery::from_root(ui)
            .match_descendants()
            .match_predicate({
                let p = placeholder.to_owned();
                move |e| e.accessible_placeholder_text().as_deref() == Some(p.as_str())
            })
            .find_first()
            .unwrap_or_else(|| panic!("找不到 {placeholder}"))
    }

    fn button_labels(ui: &SettingsWindow) -> Vec<String> {
        i_slint_backend_testing::ElementQuery::from_root(ui)
            .match_descendants()
            .match_accessible_role(i_slint_backend_testing::AccessibleRole::Button)
            .find_all()
            .into_iter()
            .filter_map(|b| b.accessible_label().map(|l| l.to_string()))
            .collect()
    }

    /// 「關於」頁照共用的更新狀態顯示狀態列與按鈕（用詞照 zoo-release 的 UI.md）
    #[test]
    fn about_page_shows_update_status_and_actions() {
        use zct_core::menu::UpdateState;
        i_slint_backend_testing::init_no_event_loop();
        let ui = SettingsWindow::new().unwrap();
        ui.set_online(true);
        ui.set_official(true);
        ui.set_page(8);
        ui.show().unwrap();
        let page = RefCell::new(String::new());
        let available = UpdateState::Available {
            version: "0.2.0".into(),
            feature: true,
        };
        show_update(&ui, &available, &page);
        assert_eq!(ui.get_update_status(), "有新版本 v0.2.0");
        let labels = button_labels(&ui);
        for want in ["檢查更新", "版本說明", "更新"] {
            assert!(labels.iter().any(|l| l == want), "{want}：{labels:?}");
        }
        assert_eq!(
            *page.borrow(),
            "https://github.com/zoosewu/zoo-type/releases/tag/v0.2.0"
        );
        // 下載中：「檢查更新」停用，只有「取消」
        show_update(
            &ui,
            &UpdateState::Downloading {
                version: "0.2.0".into(),
                percent: 68,
            },
            &page,
        );
        assert!(ui.get_checking_update());
        // 進度條與百分比分開畫（與橫幅相同的進度）
        assert_eq!(ui.get_update_status(), "正在下載 v0.2.0");
        assert_eq!(ui.get_update_progress(), 68);
        let labels = button_labels(&ui);
        assert!(labels.iter().any(|l| l == "取消"), "{labels:?}");
        assert!(!labels.iter().any(|l| l == "更新"), "{labels:?}");
        // 已是最新版：上次檢查時間是次要文字；進度條消失
        show_update(
            &ui,
            &UpdateState::Latest {
                checked: Some("今天 14:31".into()),
            },
            &page,
        );
        assert_eq!(ui.get_update_status(), "已是最新版");
        assert_eq!(ui.get_update_detail(), "（上次檢查：今天 14:31）");
        assert_eq!(ui.get_update_progress(), -1);
        // 作者的其他專案：沒有資料就不顯示標題
        let has_section = |ui: &SettingsWindow| {
            i_slint_backend_testing::ElementQuery::from_root(ui)
                .match_descendants()
                .match_predicate(|e| e.accessible_label().as_deref() == Some("作者的其他專案"))
                .find_first()
                .is_some()
        };
        assert!(!has_section(&ui));
        ui.set_other_projects(ModelRc::new(VecModel::from(vec![OtherProject {
            name: "zing-explore".into(),
            tagline: "檔案總管".into(),
            url: "https://github.com/zoosewu/zing-explore".into(),
            ..Default::default()
        }])));
        assert!(has_section(&ui));
    }

    fn dictionary_page() -> SettingsWindow {
        i_slint_backend_testing::init_no_event_loop();
        let ui = SettingsWindow::new().unwrap();
        ui.set_online(true);
        ui.set_page(4);
        ui.show().unwrap();
        ui
    }

    fn type_text(ui: &SettingsWindow, s: &str) {
        for c in s.chars() {
            key(ui, c.to_string());
        }
    }

    #[test]
    fn dictionary_tab_and_enter_move_through_the_form() {
        let ui = dictionary_page();
        let added = Rc::new(RefCell::new(None));
        ui.on_add_phrase({
            let added = added.clone();
            move |w, r| *added.borrow_mut() = Some((w.to_string(), r.to_string()))
        });
        let (word, reading) = (field(&ui, "詞"), field(&ui, "注音或縮寫"));
        word.mock_single_click(slint::platform::PointerEventButton::Left);
        type_text(&ui, "ab");
        key(&ui, Key::Tab);
        type_text(&ui, "cd");
        assert_eq!(
            reading.accessible_value().as_deref(),
            Some("cd"),
            "Tab 從詞跳到讀音"
        );
        key(&ui, Key::Return);
        assert_eq!(
            *added.borrow(),
            Some(("ab".into(), "cd".into())),
            "讀音欄按 Enter 加入"
        );
        assert_eq!(word.accessible_value().as_deref(), Some(""));
        // 加入後回到詞欄；詞欄按 Enter 跳到讀音
        type_text(&ui, "ef");
        key(&ui, Key::Return);
        type_text(&ui, "gh");
        assert_eq!(word.accessible_value().as_deref(), Some("ef"));
        assert_eq!(reading.accessible_value().as_deref(), Some("gh"));
    }

    #[test]
    fn dictionary_adds_english_word_with_empty_reading() {
        let ui = dictionary_page();
        ui.on_is_english_word(|s| zct_core::abbrev::is_abbrev(s.trim()));
        let added = Rc::new(RefCell::new(None));
        ui.on_add_phrase({
            let added = added.clone();
            move |w, r| *added.borrow_mut() = Some((w.to_string(), r.to_string()))
        });
        field(&ui, "詞").mock_single_click(slint::platform::PointerEventButton::Left);
        type_text(&ui, "github");
        key(&ui, Key::Return);
        key(&ui, Key::Return);
        assert_eq!(*added.borrow(), Some(("github".into(), "".into())));

        // 清單裡的英文單字：讀音欄寫「英文單字」，狀態只有一般 / 封鎖
        ui.set_phrases(ModelRc::new(VecModel::from(vec![to_phrase(
            &PhraseEntry {
                word: "github".into(),
                bopomofo: String::new(),
                boost: PhraseEntry::BANNED,
            },
        )])));
        let found = |text: &str| {
            i_slint_backend_testing::ElementQuery::from_root(&ui)
                .match_descendants()
                .match_predicate({
                    let t = text.to_owned();
                    move |e| e.accessible_label().as_deref() == Some(t.as_str())
                })
                .find_first()
                .is_some()
        };
        assert!(found("英文單字"));
        let state = i_slint_backend_testing::ElementQuery::from_root(&ui)
            .match_descendants()
            .match_predicate(|e| {
                e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Combobox)
            })
            .find_first()
            .expect("狀態選單");
        assert_eq!(state.accessible_value().as_deref(), Some("封鎖"));
    }

    #[test]
    fn dictionary_edit_updates_the_entry() {
        let ui = dictionary_page();
        ui.set_phrases(ModelRc::new(VecModel::from(vec![Phrase {
            word: "動物".into(),
            bopomofo: "ㄉㄨㄥˋ ㄨˋ".into(),
            state: 0,
            abbrev: false,
            english: false,
        }])));
        let updated = Rc::new(RefCell::new(None));
        ui.on_update_phrase({
            let updated = updated.clone();
            move |old, w, r| {
                *updated.borrow_mut() = Some((old.word.to_string(), w.to_string(), r.to_string()))
            }
        });
        let edit = i_slint_backend_testing::ElementQuery::from_root(&ui)
            .match_descendants()
            .match_predicate(|e| e.accessible_label().as_deref() == Some("編輯"))
            .find_first()
            .expect("編輯按鈕");
        edit.invoke_accessible_default_action();
        // 那一列變成輸入框，帶著原本的內容
        let value = |v: &str| {
            i_slint_backend_testing::ElementQuery::from_root(&ui)
                .match_descendants()
                .match_predicate({
                    let v = v.to_owned();
                    move |e| {
                        e.accessible_role()
                            == Some(i_slint_backend_testing::AccessibleRole::TextInput)
                            && e.accessible_value().as_deref() == Some(v.as_str())
                    }
                })
                .find_first()
                .unwrap_or_else(|| panic!("找不到內容是 {v} 的輸入框"))
        };
        let (word, reading) = (value("動物"), value("ㄉㄨㄥˋ ㄨˋ"));
        // 狀態選單留在原位（編輯中不能改），切換時版面不跳動
        let state = i_slint_backend_testing::ElementQuery::from_root(&ui)
            .match_descendants()
            .match_predicate(|e| {
                e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Combobox)
            })
            .find_first()
            .expect("編輯中仍顯示狀態選單");
        assert_eq!(state.accessible_value().as_deref(), Some("一般"));
        assert_eq!(state.accessible_enabled(), Some(false));
        word.set_accessible_value("動物園");
        reading.set_accessible_value("ㄉㄨㄥˋ ㄨˋ ㄩㄢˊ");
        reading.mock_single_click(slint::platform::PointerEventButton::Left);
        key(&ui, Key::Return);
        assert_eq!(
            *updated.borrow(),
            Some(("動物".into(), "動物園".into(), "ㄉㄨㄥˋ ㄨˋ ㄩㄢˊ".into()))
        );
        assert_eq!(ui.get_phrase_edit_row(), -1);
    }
}

#[cfg(test)]
mod general_page_ui {
    use super::*;
    use i_slint_backend_testing::ElementQuery;
    use zct_core::config::{EnglishWords, TypingDisplay};

    /// 在一般頁點選單選值（事件裡會重新產生列表），存檔要寫進設定
    #[test]
    fn choosing_in_the_generated_page_is_saved() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = SettingsWindow::new().unwrap();
        let config = Rc::new(RefCell::new(Config::default()));
        show_primary_settings(&ui, &config.borrow());
        // 和 main 一樣：選單的 selected 事件裡重新產生列表
        {
            let (ui_weak, config) = (ui.as_weak(), config.clone());
            ui.on_general_changed(move || {
                rebuild_general(&ui_weak.unwrap(), &mut config.borrow_mut(), false).unwrap();
            });
        }
        ui.set_page(0);
        ui.window()
            .set_size(slint::LogicalSize::new(1000.0, 4000.0));
        ui.show().unwrap();
        let combo = ElementQuery::from_root(&ui)
            .match_descendants()
            .match_accessible_role(i_slint_backend_testing::AccessibleRole::Combobox)
            .find_all()
            .into_iter()
            .find(|c| c.accessible_value().as_deref() == Some("中文優先"))
            .expect("英文單字判斷的選單");
        combo.mock_single_click(slint::platform::PointerEventButton::Left);
        ElementQuery::from_root(&ui)
            .match_descendants()
            .match_predicate(|e| e.accessible_label().as_deref() == Some("英文優先"))
            .find_all()
            .last()
            .expect("選單的選項")
            .mock_single_click(slint::platform::PointerEventButton::Left);
        let saved = collect_primary_settings(&ui, &config.borrow()).unwrap();
        assert_eq!(saved.mixed.english, EnglishWords::EnglishFirst);
        assert_eq!(
            saved.mixed.typing,
            TypingDisplay::Keys,
            "英文優先：組字區顯示字母"
        );
    }
}

#[cfg(test)]
mod scrolling {
    use super::*;
    use i_slint_backend_testing::ElementQuery;

    /// 一般頁捲到最下面，最後一列要完整出現在狀態列上方
    /// （換行的說明文字曾讓捲動區域少算高度，最後幾列捲不到）
    #[test]
    fn general_page_scrolls_to_the_last_row() {
        i_slint_backend_testing::init_no_event_loop();
        for height in [450.0, 500.0, 600.0, 700.0] {
            let ui = SettingsWindow::new().unwrap();
            ui.set_show_advanced(true);
            show_primary_settings(&ui, &Config::default());
            ui.set_page(0);
            ui.window().set_size(slint::LogicalSize::new(820.0, height));
            ui.show().unwrap();
            let position = slint::LogicalPosition::new(500.0, height / 2.0);
            for _ in 0..200 {
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                        position,
                        delta_x: 0.0,
                        delta_y: -120.0,
                    });
            }
            let find = |label: &str| {
                ElementQuery::from_root(&ui)
                    .match_descendants()
                    .match_predicate({
                        let label = label.to_owned();
                        move |e| e.accessible_label().as_deref() == Some(label.as_str())
                    })
                    .find_first()
                    .unwrap_or_else(|| panic!("{height}: 看不到「{label}」"))
            };
            let last = find("自訂半形標點（JSON）");
            let status = find("輸入法服務未執行，無法使用詞庫與學習");
            let bottom = last.absolute_position().y + last.size().height;
            assert!(
                bottom <= status.absolute_position().y,
                "{height}: 最後一列被擋住（{bottom} > {}）",
                status.absolute_position().y
            );
        }
    }
}
