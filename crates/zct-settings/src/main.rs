//! ZCT 設定程式（Slint）
//!
//! - `config.toml`、`apps.toml`：直接讀寫檔案，服務程序在 1 秒內（主題）或 App 重新連線時（其他）套用
//! - 詞庫、統計、備份：透過 IPC 交給服務程序處理（服務程序是學習資料唯一的擁有者，避免互相覆蓋）

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{Model, ModelRc, SharedString, StandardListViewItem, VecModel};
use zct_core::config::{Color, GameMode, InputMode, Layout, OnFocus, Output, ShiftToggle, Width};
use zct_core::hotkey::{Binding, Hotkey};
use zct_core::paths::DataRoot;
use zct_core::symbols::{PrimarySymbols, SecondarySymbols};
use zct_core::{AppRule, AppRules, Config, PhraseAction, PhraseEntry};
use zct_ipc::pipe::Client;
use zct_ipc::{Command, PROTOCOL_VERSION, Request, Response};

slint::include_modules!();

// ---- 設定檔 ↔ 介面 ----

fn general_from(c: &Config) -> General {
    General {
        default_mode: (c.input.default_mode == InputMode::English) as i32,
        punctuation: (c.input.punctuation == Width::Half) as i32,
        output: (c.input.output == Output::Simplified) as i32,
        on_focus: (c.input.on_focus == OnFocus::Reset) as i32,
        shift: match c.toggle.shift {
            ShiftToggle::Both => 0,
            ShiftToggle::Left => 1,
            ShiftToggle::Right => 2,
            ShiftToggle::Off => 3,
        },
        tap_ms: c.toggle.shift_tap_max_ms as i32,
        per_page: c.candidates.per_page as i32,
        sort_freq: c.candidates.sort_by_frequency,
        detect_legacy: c.detect.legacy_fullscreen,
        detect_fullscreen: c.detect.exclusive_fullscreen,
        auto_english: c.mixed.auto_english,
        english_candidate: c.mixed.english_candidate,
        auto_space: c.mixed.auto_space,
        primary: match c.punctuation.effective().0 {
            PrimarySymbols::Backtick => 0,
            PrimarySymbols::RightAlt => 1,
            PrimarySymbols::Both => 2,
        },
        secondary: (c.punctuation.effective().1 == SecondarySymbols::Layer) as i32,
    }
}

fn apply_general(c: &mut Config, g: &General) {
    c.input.default_mode = if g.default_mode == 1 { InputMode::English } else { InputMode::Chinese };
    c.input.punctuation = if g.punctuation == 1 { Width::Half } else { Width::Full };
    c.input.output = if g.output == 1 { Output::Simplified } else { Output::Traditional };
    c.input.on_focus = if g.on_focus == 1 { OnFocus::Reset } else { OnFocus::Remember };
    c.toggle.shift = match g.shift {
        1 => ShiftToggle::Left,
        2 => ShiftToggle::Right,
        3 => ShiftToggle::Off,
        _ => ShiftToggle::Both,
    };
    c.toggle.shift_tap_max_ms = g.tap_ms.max(0) as u64;
    c.candidates.per_page = g.per_page.clamp(1, 10) as usize;
    c.candidates.sort_by_frequency = g.sort_freq;
    c.detect.legacy_fullscreen = g.detect_legacy;
    c.detect.exclusive_fullscreen = g.detect_fullscreen;
    c.mixed.auto_english = g.auto_english;
    c.mixed.english_candidate = g.english_candidate;
    c.mixed.auto_space = g.auto_space;
    c.punctuation.primary = match g.primary {
        1 => PrimarySymbols::RightAlt,
        2 => PrimarySymbols::Both,
        _ => PrimarySymbols::Backtick,
    };
    c.punctuation.secondary = if g.secondary == 1 { SecondarySymbols::Layer } else { SecondarySymbols::Shift };
    // 舊版欄位已轉換，不再保留
    c.punctuation.shift = None;
    c.punctuation.right_alt = None;
}

fn ai_from(c: &Config) -> AiSettings {
    AiSettings {
        enabled: c.ai.enabled,
        endpoint: c.ai.endpoint.clone().into(),
        model: c.ai.model.clone().into(),
        timeout: c.ai.timeout_seconds as i32,
    }
}

fn apply_ai(c: &mut Config, a: &AiSettings) {
    c.ai.enabled = a.enabled;
    c.ai.endpoint = a.endpoint.trim().to_string();
    c.ai.model = a.model.trim().to_string();
    c.ai.timeout_seconds = a.timeout.max(1) as u64;
}

fn sync_from(c: &Config) -> SyncSettings {
    SyncSettings {
        enabled: c.sync.enabled,
        remote: c.sync.remote.clone().into(),
        interval: c.sync.interval_minutes as i32,
    }
}

fn apply_sync(c: &mut Config, s: &SyncSettings) {
    c.sync.enabled = s.enabled;
    c.sync.remote = s.remote.trim().to_string();
    c.sync.interval_minutes = s.interval.max(1) as u64;
}

fn binding_text(b: &Binding) -> SharedString {
    b.0.map(|h| h.to_string()).unwrap_or_default().into()
}

fn parse_binding(s: &str) -> Result<Binding, String> {
    if s.trim().is_empty() {
        Ok(Binding(None))
    } else {
        s.parse::<Hotkey>().map(|h| Binding(Some(h)))
    }
}

fn hotkeys_from(c: &Config) -> Hotkeys {
    let h = &c.hotkeys;
    Hotkeys {
        width: binding_text(&h.width),
        output: binding_text(&h.output),
        game: binding_text(&h.game_mode),
        forget: binding_text(&h.forget),
        pin: binding_text(&h.pin),
        emoji: binding_text(&h.emoji),
        ai: binding_text(&h.ai),
        bar: binding_text(&h.composition_bar),
        symbols: binding_text(&h.symbols),
    }
}

fn apply_hotkeys(c: &mut Config, h: &Hotkeys) -> Result<(), String> {
    c.hotkeys.width = parse_binding(&h.width)?;
    c.hotkeys.output = parse_binding(&h.output)?;
    c.hotkeys.game_mode = parse_binding(&h.game)?;
    c.hotkeys.forget = parse_binding(&h.forget)?;
    c.hotkeys.pin = parse_binding(&h.pin)?;
    c.hotkeys.emoji = parse_binding(&h.emoji)?;
    c.hotkeys.ai = parse_binding(&h.ai)?;
    c.hotkeys.composition_bar = parse_binding(&h.bar)?;
    c.hotkeys.symbols = parse_binding(&h.symbols)?;
    Ok(())
}

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

fn apply_theme(c: &mut Config, t: &Theme) -> Result<(), String> {
    let color = |s: &SharedString| s.parse::<Color>();
    c.theme.layout = if t.layout == 1 { Layout::Horizontal } else { Layout::Vertical };
    c.theme.font = t.font.trim().to_string();
    c.theme.font_size = t.size.clamp(8, 48) as f32;
    c.theme.background = color(&t.background)?;
    c.theme.text = color(&t.text)?;
    c.theme.number = color(&t.number)?;
    c.theme.highlight = color(&t.highlight)?;
    Ok(())
}

/// 0 = 沿用全域；其餘依序對應選項
fn pick<T: Copy>(i: i32, options: &[T]) -> Option<T> {
    usize::try_from(i - 1).ok().and_then(|i| options.get(i).copied())
}

fn index_of<T: PartialEq>(v: Option<T>, options: &[T]) -> i32 {
    v.and_then(|v| options.iter().position(|o| *o == v))
        .map_or(0, |i| i as i32 + 1)
}

const MODES: [InputMode; 2] = [InputMode::Chinese, InputMode::English];
const WIDTHS: [Width; 2] = [Width::Full, Width::Half];
const OUTPUTS: [Output; 2] = [Output::Traditional, Output::Simplified];
const FOCUS: [OnFocus; 2] = [OnFocus::Remember, OnFocus::Reset];
const DETECT: [bool; 2] = [true, false];

fn row_from(r: &AppRule) -> AppRuleRow {
    AppRuleRow {
        name: r.name.clone().into(),
        windows: r.windows.clone().unwrap_or_default().into(),
        macos: r.macos.clone().unwrap_or_default().into(),
        default_mode: index_of(r.default_mode, &MODES),
        punctuation: index_of(r.punctuation, &WIDTHS),
        output: index_of(r.output, &OUTPUTS),
        on_focus: index_of(r.on_focus, &FOCUS),
        game: match r.game_mode {
            Some(GameMode::English) => 1,
            Some(GameMode::Keep) => 2,
            _ => 0,
        },
        detect: index_of(r.detect, &DETECT),
        bar: index_of(r.composition_bar, &DETECT),
    }
}

fn rule_from(r: &AppRuleRow) -> AppRule {
    let text = |s: &SharedString| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    AppRule {
        name: r.name.trim().to_string(),
        windows: text(&r.windows),
        macos: text(&r.macos),
        default_mode: pick(r.default_mode, &MODES),
        punctuation: pick(r.punctuation, &WIDTHS),
        output: pick(r.output, &OUTPUTS),
        on_focus: pick(r.on_focus, &FOCUS),
        game_mode: match r.game {
            1 => Some(GameMode::English),
            2 => Some(GameMode::Keep),
            _ => None,
        },
        detect: pick(r.detect, &DETECT),
        composition_bar: pick(r.bar, &DETECT),
    }
}

fn table_rows(rows: &VecModel<AppRuleRow>) -> ModelRc<ModelRc<StandardListViewItem>> {
    let item = |s: &SharedString| StandardListViewItem::from(s.clone());
    let v: Vec<ModelRc<StandardListViewItem>> = rows
        .iter()
        .map(|r| ModelRc::new(VecModel::from(vec![item(&r.name), item(&r.windows), item(&r.macos)])))
        .collect();
    ModelRc::new(VecModel::from(v))
}

// ---- 服務程序連線 ----

struct Service {
    client: Option<Client>,
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
            return Service { client: Some(c) };
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
                    return Service { client: Some(c) };
                }
            }
        }
        Service { client: None }
    }

    fn call(&mut self, req: Request) -> Result<Response, String> {
        if self.client.is_none() {
            self.client = Self::connect();
        }
        let client = self.client.as_mut().ok_or("服務程序未執行")?;
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

struct State {
    data: DataRoot,
    config: Config,
    service: Service,
    phrases: Vec<PhraseEntry>,
    filter: String,
}

fn to_phrase(p: &PhraseEntry) -> Phrase {
    Phrase {
        word: p.word.clone().into(),
        bopomofo: p.bopomofo.clone().into(),
        state: if p.is_banned() {
            2
        } else if p.is_pinned() {
            1
        } else {
            0
        },
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

/// 視窗標題（settings.slint 的 title）；第二次開啟時用來找到已經開著的視窗
const WINDOW_TITLE: &str = "動物園輸入法 設定";

/// 只允許一個設定程式：已經開著時把那個視窗叫到前面，回傳 false
///
/// 變更資料位置後會自己重新開啟（`--notice`），那時舊的還在結束中，等它一下。
#[cfg(windows)]
fn single_instance(wait_for_old: bool) -> bool {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};
    use windows::core::{HSTRING, PCWSTR};
    let tries = if wait_for_old { 25 } else { 1 };
    for _ in 0..tries {
        unsafe {
            let Ok(handle) = CreateMutexW(None, false, &HSTRING::from(r"Local\ZooType-settings")) else {
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
        if let Ok(hwnd) = FindWindowW(PCWSTR::null(), &HSTRING::from(WINDOW_TITLE)) {
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !single_instance(std::env::args().any(|a| a == "--notice")) {
        return Ok(());
    }
    // 改名前的 %APPDATA%\zct 搬到 %APPDATA%\ZooType（服務程序也會做，先開哪個都一樣）
    let _ = zct_core::paths::migrate_legacy();
    let data = DataRoot::discover().ok_or("找不到資料根目錄")?;
    let ui = SettingsWindow::new()?;
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

    let (config, config_error) = match Config::load(&data.config()) {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e.to_string())),
    };
    let (rules, apps_error) = match AppRules::load(&data.root().join("apps.toml")) {
        Ok(a) => (a, None),
        Err(e) => (AppRules::default(), Some(e.to_string())),
    };
    ui.set_general(general_from(&config));
    ui.set_hotkeys(hotkeys_from(&config));
    ui.set_theme(theme_from(&config));
    ui.set_sync(sync_from(&config));
    ui.set_ai(ai_from(&config));
    ui.set_has_api_key(zct_ipc::secret::read_ai_key().is_some());
    let apps = Rc::new(VecModel::from(rules.apps.iter().map(row_from).collect::<Vec<_>>()));
    ui.set_apps(ModelRc::from(apps.clone()));
    ui.set_apps_rows(table_rows(&apps));

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

    let state = Rc::new(RefCell::new(State {
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
    {
        // 第一次用到時才載入（約 5MB）
        let readings: RefCell<Option<Option<zct_core::readings::Readings>>> = RefCell::new(None);
        ui.on_suggest_reading(move |word| {
            let mut cache = readings.borrow_mut();
            let r = cache.get_or_insert_with(load_readings);
            r.as_ref()
                .and_then(|r| r.reading_of(&word))
                .unwrap_or_default()
                .into()
        });
    }

    // 儲存設定
    {
        let (ui_weak, state, apps) = (ui.as_weak(), state.clone(), apps.clone());
        ui.on_save(move || {
            let ui = ui_weak.unwrap();
            let mut st = state.borrow_mut();
            let mut config = st.config.clone();
            apply_general(&mut config, &ui.get_general());
            apply_sync(&mut config, &ui.get_sync());
            apply_ai(&mut config, &ui.get_ai());
            let result = apply_hotkeys(&mut config, &ui.get_hotkeys())
                .and_then(|_| apply_theme(&mut config, &ui.get_theme()))
                .and_then(|_| {
                    config.save(&st.data.config()).map_err(|e| e.to_string())?;
                    let rules = AppRules {
                        apps: apps.iter().map(|r| rule_from(&r)).collect(),
                    };
                    rules
                        .save(&st.data.root().join("apps.toml"))
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(()) => {
                    st.config = config;
                    ui.set_status("已儲存。外觀立即生效，其他設定在切換 App 後生效".into());
                }
                Err(e) => ui.set_status(format!("無法儲存：{e}").into()),
            }
        });
    }

    // App 規則
    {
        let (ui_weak, apps) = (ui.as_weak(), apps.clone());
        ui.on_select_app(move |i| {
            let ui = ui_weak.unwrap();
            if let Some(row) = apps.row_data(i as usize) {
                ui.set_app_edit(row);
            }
        });
    }
    {
        let (ui_weak, apps) = (ui.as_weak(), apps.clone());
        ui.on_apply_app(move || {
            let ui = ui_weak.unwrap();
            let i = ui.get_app_index();
            if i >= 0 {
                apps.set_row_data(i as usize, ui.get_app_edit());
                ui.set_apps_rows(table_rows(&apps));
                ui.set_status("已套用，按「儲存設定」寫入檔案".into());
            }
        });
    }
    {
        let (ui_weak, apps) = (ui.as_weak(), apps.clone());
        ui.on_add_app(move || {
            let ui = ui_weak.unwrap();
            let row = row_from(&AppRule {
                name: "新規則".into(),
                ..Default::default()
            });
            apps.push(row.clone());
            ui.set_apps_rows(table_rows(&apps));
            ui.set_app_index(apps.row_count() as i32 - 1);
            ui.set_app_edit(row);
        });
    }
    {
        let (ui_weak, apps) = (ui.as_weak(), apps.clone());
        ui.on_remove_app(move |i| {
            let ui = ui_weak.unwrap();
            if i >= 0 && (i as usize) < apps.row_count() {
                apps.remove(i as usize);
                ui.set_apps_rows(table_rows(&apps));
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
        }
    };
    let refresh = {
        let (ui_weak, state, show_phrases) = (ui.as_weak(), state.clone(), show_phrases.clone());
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
        let ui_weak = ui.as_weak();
        ui.on_save_api_key(move |key| {
            let ui = ui_weak.unwrap();
            let target = zct_ipc::secret::AI_API_KEY;
            let msg = if key.is_empty() {
                zct_ipc::secret::delete(target);
                "已清除 API Key".to_string()
            } else {
                match zct_ipc::secret::write(target, key.trim()) {
                    Ok(()) => "API Key 已存入 Windows 認證管理員".to_string(),
                    Err(e) => format!("無法儲存 API Key：{e}"),
                }
            };
            ui.set_has_api_key(zct_ipc::secret::read_ai_key().is_some());
            ui.set_ai_status(msg.into());
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_test_ai(move || {
            let ui = ui_weak.unwrap();
            let a = ui.get_ai();
            let ep = zct_ai::Endpoint {
                base_url: a.endpoint.to_string(),
                model: a.model.to_string(),
                api_key: zct_ipc::secret::read_ai_key(),
                timeout: Duration::from_secs(a.timeout.max(1) as u64),
            };
            let msg = match zct_ai::ping(&ep) {
                Ok(reply) => format!("連線成功，模型回應：{reply}"),
                Err(e) => e.to_string(),
            };
            ui.set_ai_status(msg.into());
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
            let Some(action) = state_action(p.state, to) else { return };
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
        let (ui_weak, state) = (ui.as_weak(), state.clone());
        ui.on_restore(move |day| {
            let ui = ui_weak.unwrap();
            let r = state.borrow_mut().service.call(Request::Restore { day: day.to_string() });
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
            let Some(new) = pick_folder(&start) else { return };
            let Some(pointer) = zct_core::paths::pointer_dir() else { return };
            // 先讓服務程序存檔並停止寫入，複製時資料才不會變動
            let online = state.borrow_mut().service.call(Request::Command(Command::PrepareMove)).is_ok();
            let result = zct_core::paths::change_data_root(&pointer, &current, &new);
            // 不論成功與否都重新啟動服務程序（成功：讀新位置；失敗：解除停止寫入）
            if online {
                let _ = state.borrow_mut().service.call(Request::Command(Command::Restart));
            }
            match result {
                Ok(mode) => {
                    let what = match mode {
                        zct_core::paths::ChangeRoot::Copied => "已把資料複製到",
                        zct_core::paths::ChangeRoot::Adopted => "已改用",
                    };
                    let msg = format!("{what} {}（原本的資料保留在 {}）", new.display(), current.display());
                    // 重新開啟設定程式，讀新位置的設定
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).args(["--page", "6", "--notice", &msg]).spawn();
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
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        SIGDN_FILESYSPATH,
    };
    use windows::core::HSTRING;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        let _ = dialog.SetTitle(&HSTRING::from("選擇學習資料位置"));
        if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start.as_os_str()), None) {
            let _ = dialog.SetFolder(&item);
        }
        // 按取消時 Show 回傳錯誤
        dialog.Show(None).ok()?;
        let path = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as _));
        text.map(std::path::PathBuf::from)
    }
}

#[cfg(not(windows))]
fn pick_folder(_start: &std::path::Path) -> Option<std::path::PathBuf> {
    None
}

/// 首選正確率：沒有修正的字數比例
fn accuracy(chars: u64, corrections: u64) -> f32 {
    if chars == 0 {
        return 1.0;
    }
    (1.0 - corrections as f32 / chars as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips_through_ui_structs() {
        let mut c = Config::default();
        c.input.default_mode = InputMode::English;
        c.input.output = Output::Simplified;
        c.toggle.shift = ShiftToggle::Right;
        c.detect.legacy_fullscreen = false;
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

        let mut back = Config::default();
        apply_general(&mut back, &general_from(&c));
        apply_sync(&mut back, &sync_from(&c));
        apply_ai(&mut back, &ai_from(&c));
        apply_hotkeys(&mut back, &hotkeys_from(&c)).unwrap();
        apply_theme(&mut back, &theme_from(&c)).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn invalid_ui_values_are_rejected() {
        let mut c = Config::default();
        let mut h = hotkeys_from(&c);
        h.game = "ctrl+bogus".into();
        assert!(apply_hotkeys(&mut c, &h).is_err());
        let mut t = theme_from(&c);
        t.text = "紅色".into();
        assert!(apply_theme(&mut c, &t).is_err());
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
        assert_eq!(rule_from(&row_from(&rule)), rule);
        assert_eq!(rule_from(&row_from(&AppRule::default())), AppRule::default());
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
