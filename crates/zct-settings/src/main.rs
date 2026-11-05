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
use zct_core::{AppRule, AppRules, Config, PhraseAction, PhraseEntry};
use zct_ipc::pipe::Client;
use zct_ipc::{PROTOCOL_VERSION, Request, Response};

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
    }
}

fn apply_hotkeys(c: &mut Config, h: &Hotkeys) -> Result<(), String> {
    c.hotkeys.width = parse_binding(&h.width)?;
    c.hotkeys.output = parse_binding(&h.output)?;
    c.hotkeys.game_mode = parse_binding(&h.game)?;
    c.hotkeys.forget = parse_binding(&h.forget)?;
    c.hotkeys.pin = parse_binding(&h.pin)?;
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
            exe: "zct-settings.exe".into(),
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
            .and_then(|p| p.parent().map(|d| d.join("zct-service.exe")))
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
        pinned: p.is_pinned(),
        banned: p.is_banned(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = DataRoot::discover().ok_or("找不到資料根目錄")?;
    let ui = SettingsWindow::new()?;
    // --page N：直接開啟某一頁（系統匣與開發截圖用）
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--page"
            && let Some(n) = args.next().and_then(|n| n.parse().ok())
        {
            ui.set_page(n);
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
    let apps = Rc::new(VecModel::from(rules.apps.iter().map(row_from).collect::<Vec<_>>()));
    ui.set_apps(ModelRc::from(apps.clone()));
    ui.set_apps_rows(table_rows(&apps));

    let service = Service::start();
    ui.set_online(service.client.is_some());
    ui.set_status(
        config_error
            .or(apps_error)
            .map(|e| format!("{e}（儲存會以目前畫面上的設定覆寫）"))
            .unwrap_or_else(|| format!("資料位置：{}", data.root().display()))
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

    // 儲存設定
    {
        let (ui_weak, state, apps) = (ui.as_weak(), state.clone(), apps.clone());
        ui.on_save(move || {
            let ui = ui_weak.unwrap();
            let mut st = state.borrow_mut();
            let mut config = st.config.clone();
            apply_general(&mut config, &ui.get_general());
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
            if let Some(e) = error {
                ui.set_status(e.into());
            }
        }
    };
    ui.on_refresh(refresh.clone());
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
        ui.on_phrase_action(move |p, action| {
            let action = match action.as_str() {
                "ban" => PhraseAction::Ban,
                "unban" => PhraseAction::Unban,
                _ => PhraseAction::Unpin,
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

    ui.run()?;
    Ok(())
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

        let mut back = Config::default();
        apply_general(&mut back, &general_from(&c));
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
        };
        assert_eq!(rule_from(&row_from(&rule)), rule);
        assert_eq!(rule_from(&row_from(&AppRule::default())), AppRule::default());
    }

    #[test]
    fn accuracy_is_bounded() {
        assert_eq!(accuracy(0, 0), 1.0);
        assert_eq!(accuracy(100, 5), 0.95);
        assert_eq!(accuracy(1, 9), 0.0);
    }
}
