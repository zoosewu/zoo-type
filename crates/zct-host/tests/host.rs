//! 主機層的行為：兩個平台都靠這些測試保證一致

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zct_core::hotkey::Binding;
use zct_core::paths::DataRoot;
use zct_core::{Config, Engine, InputMode, Key, KeyEvent, KeyResult, Mods, Side};
use zct_host::{AiDisplay, Host, HostSession, Platform};

struct Fixture {
    dir: tempfile::TempDir,
    host: Arc<Host>,
    t: u64,
}

impl Fixture {
    fn new(platform: Platform, config: Config) -> Fixture {
        Fixture::build(platform, config, |h| h)
    }

    fn build(platform: Platform, config: Config, f: impl FnOnce(Host) -> Host) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let data = DataRoot::new(dir.path());
        config.save(&data.config()).unwrap();
        let host = Arc::new(f(Host::new(platform, &data, engine(&data))));
        Fixture {
            dir,
            host,
            t: 1_000,
        }
    }

    fn open(&self, app: &str) -> HostSession {
        HostSession::open(&self.host, 1, app, app).unwrap()
    }

    fn press(&mut self, s: &mut HostSession, key: Key, mods: Mods) -> KeyResult {
        self.t += 30;
        let r = s.key(&KeyEvent::down(key, mods, self.t));
        self.t += 30;
        s.key(&KeyEvent::up(key, Mods::NONE, self.t));
        r
    }

    fn hotkey(&mut self, s: &mut HostSession, b: &Binding) -> KeyResult {
        let h = b.0.expect("預設有綁定");
        let mods = Mods {
            ctrl: h.ctrl,
            alt: h.alt,
            shift: h.shift,
            meta: h.meta,
            ..Mods::NONE
        };
        self.press(s, h.key, mods)
    }

    fn tap_shift(&mut self, s: &mut HostSession) -> KeyResult {
        let k = Key::Shift(Side::Left);
        self.t += 30;
        let r = s.key(&KeyEvent::down(k, Mods::shift(), self.t));
        self.t += 80;
        let up = s.key(&KeyEvent::up(k, Mods::NONE, self.t));
        if up.mode_changed.is_some() { up } else { r }
    }

    fn type_str(&mut self, s: &mut HostSession, keys: &str) {
        for b in keys.bytes() {
            let key = if b == b' ' { Key::Space } else { Key::Char(b) };
            self.press(s, key, Mods::NONE);
        }
    }

    /// 寫設定檔並等到主機看得到（修改時間每 500ms 檢查一次）
    fn write(&self, name: &str, text: &str) {
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(self.dir.path().join(name), text).unwrap();
        std::thread::sleep(zct_host::settings::CHECK_EVERY + Duration::from_millis(100));
    }
}

fn engine(data: &DataRoot) -> Arc<Engine> {
    let dict = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/dict");
    Arc::new(Engine::open(&dict, &data.learning()).expect("需要先執行 scripts/fetch-dict.sh"))
}

#[test]
fn hints_are_the_same_on_both_platforms() {
    for platform in [Platform::Windows, Platform::Macos] {
        let mut f = Fixture::new(platform, Config::default());
        let mut s = f.open("app");
        assert_eq!(f.tap_shift(&mut s).notice.as_deref(), Some("A"));
        assert_eq!(f.tap_shift(&mut s).notice.as_deref(), Some("中"));
        assert_eq!(s.toggle_output().notice.as_deref(), Some("动"));
        assert_eq!(s.toggle_output().notice.as_deref(), Some("動"));
        let b = Config::default().hotkeys.game_mode;
        assert_eq!(f.hotkey(&mut s, &b).notice.as_deref(), Some("遊戲模式：開"));
        assert_eq!(s.toggle_game().notice.as_deref(), Some("遊戲模式：關"));
    }
}

#[test]
fn game_mode_is_shared_by_all_sessions() {
    let mut f = Fixture::new(Platform::Macos, Config::default());
    let mut a = f.open("a");
    let mut b = f.open("b");
    let game = Config::default().hotkeys.game_mode;
    f.hotkey(&mut a, &game);
    assert!(b.game());
    assert!(
        !f.press(&mut b, Key::Char(b'j'), Mods::NONE).eaten,
        "遊戲模式下按鍵交給 App"
    );
    f.hotkey(&mut b, &game);
    assert!(f.press(&mut a, Key::Char(b'j'), Mods::NONE).eaten);
}

#[test]
fn new_settings_apply_when_idle_and_keep_mode() {
    let mut f = Fixture::new(Platform::Windows, Config::default());
    let mut s = f.open("app.exe");
    f.tap_shift(&mut s);
    assert_eq!(s.session().mode(), InputMode::English);
    f.write(
        "config.toml",
        "[mixed]
typing = 'keys'
",
    );
    assert!(s.refresh());
    assert_eq!(s.session().mode(), InputMode::English, "沿用中英");
    f.tap_shift(&mut s);
    f.type_str(&mut s, "su");
    assert_eq!(s.session().preedit().text, "su", "新設定：組字區顯示字母");
}

#[test]
fn new_settings_wait_until_composition_ends() {
    let mut f = Fixture::new(Platform::Windows, Config::default());
    let mut s = f.open("app.exe");
    f.type_str(&mut s, "su");
    f.write(
        "config.toml",
        "[mixed]
typing = 'keys'
",
    );
    assert!(!s.refresh(), "組字中不換");
    f.type_str(&mut s, "3");
    assert_eq!(s.session().preedit().text, "你", "組字中沿用舊設定");
    f.press(&mut s, Key::Enter, Mods::NONE);
    f.type_str(&mut s, "su");
    assert_eq!(s.session().preedit().text, "su", "組字結束後換成新設定");
}

#[test]
fn app_rules_reload_for_the_running_session() {
    let mut f = Fixture::new(Platform::Macos, Config::default());
    let mut s = f.open("org.example.game");
    assert!(f.press(&mut s, Key::Char(b'j'), Mods::NONE).eaten);
    f.press(&mut s, Key::Esc, Mods::NONE);
    f.write(
        "apps.toml",
        "[[app]]\nmacos = 'org.example.game'\ngame_mode = 'english'\n",
    );
    assert!(!f.press(&mut s, Key::Char(b'j'), Mods::NONE).eaten);
}

#[test]
fn mode_is_remembered_per_context() {
    let mut f = Fixture::new(Platform::Macos, Config::default());
    let mut s = f.open("com.apple.TextEdit");
    f.tap_shift(&mut s);
    drop(s);
    let s = f.open("com.apple.TextEdit");
    assert_eq!(s.session().mode(), InputMode::English);
    let other = f.open("com.apple.Safari");
    assert_eq!(other.session().mode(), InputMode::Chinese);
}

#[test]
fn composition_bar_defaults_and_hotkey_writes_the_platform_rule() {
    let cases = [
        (
            Platform::Windows,
            "WindowsTerminal.exe",
            "notepad.exe",
            "windows = \"notepad.exe\"",
        ),
        (
            Platform::Macos,
            "com.apple.Terminal",
            "com.apple.TextEdit",
            "macos = \"com.apple.TextEdit\"",
        ),
    ];
    for (platform, terminal, app, rule) in cases {
        let mut f = Fixture::new(platform, Config::default());
        assert!(f.open(terminal).composition_bar(), "{terminal}");
        let mut s = f.open(app);
        assert!(!s.composition_bar());
        let b = Config::default().hotkeys.composition_bar;
        let r = f.hotkey(&mut s, &b);
        assert_eq!(r.notice.as_deref(), Some("組字提示列：開"));
        assert!(s.composition_bar());
        let apps = std::fs::read_to_string(f.dir.path().join("apps.toml")).unwrap();
        assert!(apps.contains(rule), "{apps}");
        assert!(!s.refresh(), "自己寫的規則不用重建 session");
        assert!(f.open(app).composition_bar(), "下次開啟沿用");
    }
}

#[test]
fn composition_bar_is_not_written_when_read_only() {
    let mut f = Fixture::new(Platform::Windows, Config::default());
    f.host.settings().set_read_only();
    let mut s = f.open("notepad.exe");
    let b = Config::default().hotkeys.composition_bar;
    let r = f.hotkey(&mut s, &b);
    assert!(r.notice.unwrap().starts_with("無法儲存組字提示列"));
    assert!(!s.composition_bar());
    assert!(!f.dir.path().join("apps.toml").exists());
}

#[test]
fn replacing_the_engine_reopens_idle_sessions() {
    let f = Fixture::new(Platform::Macos, Config::default());
    let mut s = f.open("app");
    let fresh = engine(&DataRoot::new(f.dir.path()));
    f.host.replace_engine(fresh.clone());
    assert!(s.refresh());
    assert!(Arc::ptr_eq(&s.session().engine(), &fresh));
}

/// 假的 OpenAI 相容 API：回傳固定內容，並把收到的請求交回來
fn server(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        request.push_str(&String::from_utf8(body).unwrap());
        let json = serde_json::json!({"choices":[{"message":{"content":reply}}]}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
            json.len()
        )
        .unwrap();
        request
    });
    (format!("http://{address}/v1"), worker)
}

fn ai_config(endpoint: String) -> Config {
    let mut config = Config::default();
    config.ai.enabled = true;
    config.ai.endpoint = endpoint;
    config.ai.model = "fixture".into();
    config
}

#[test]
fn ai_runs_in_background_notifies_and_rejects_stale_results() {
    for cancel in [false, true] {
        let (endpoint, server) = server("天氣");
        let shown: Arc<Mutex<Vec<AiDisplay>>> = Arc::default();
        let sink = shown.clone();
        let mut f = Fixture::build(Platform::Windows, ai_config(endpoint), |h| {
            h.with_ai_key_source(Box::new(|| Some("test-only".into())))
                .with_ai_notify(Box::new(move |_, d| sink.lock().unwrap().push(d)))
        });
        let mut s = f.open("app.exe");
        s.reconvert("天汽");
        let b = Config::default().hotkeys.ai;
        f.hotkey(&mut s, &b);
        assert!(s.ai_pending());
        if cancel {
            f.press(&mut s, Key::Esc, Mods::NONE);
        }
        let request = server.join().unwrap();
        assert!(request.contains("Bearer test-only"));
        if cancel {
            std::thread::sleep(Duration::from_millis(200));
            assert!(s.poll_ai().is_none(), "舊請求作廢");
            assert!(shown.lock().unwrap().is_empty(), "也不通知平台");
            continue;
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        let result = loop {
            if let Some(r) = s.poll_ai() {
                break r;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(result.candidates.unwrap().items, ["天氣"]);
        assert_eq!(
            *shown.lock().unwrap(),
            [AiDisplay::Suggestion("天氣".into())]
        );
        assert!(!s.ai_pending());
        assert_eq!(
            f.press(&mut s, Key::Enter, Mods::NONE).commit.as_deref(),
            Some("天氣")
        );
    }
}

#[test]
fn ai_key_set_by_the_platform_is_sent() {
    let (endpoint, server) = server("天汽");
    let mut f = Fixture::new(Platform::Macos, ai_config(endpoint));
    f.host.set_ai_key(Some("from-keychain".into()));
    let mut s = f.open("app");
    s.reconvert("天汽");
    let b = Config::default().hotkeys.ai;
    f.hotkey(&mut s, &b);
    assert!(server.join().unwrap().contains("Bearer from-keychain"));
    let deadline = Instant::now() + Duration::from_secs(15);
    let result = loop {
        if let Some(r) = s.poll_ai() {
            break r;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(result.notice.as_deref(), Some("AI：沒有發現錯字"));
}

#[test]
fn relocation_switches_settings_and_engine() {
    let f = Fixture::new(Platform::Macos, Config::default());
    let mut s = f.open("app");
    let other = tempfile::tempdir().unwrap();
    let data = DataRoot::new(other.path());
    let mut config = Config::default();
    config.mixed.typing = zct_core::config::TypingDisplay::Keys;
    config.save(&data.config()).unwrap();
    let fresh = engine(&data);
    f.host.relocate(&data, fresh.clone());
    assert!(s.refresh());
    assert!(Arc::ptr_eq(&s.session().engine(), &fresh));
    assert_eq!(
        f.host.settings().current().0.mixed.typing,
        config.mixed.typing
    );
}

#[test]
fn composition_bar_hotkey_keeps_the_other_platform_fields() {
    let mut f = Fixture::new(Platform::Macos, Config::default());
    std::fs::write(
        f.dir.path().join("apps.toml"),
        "[[app]]\nname = 'Editor'\nmacos = 'test.Editor'\nwindows = 'editor.exe'\ndetect = false\n",
    )
    .unwrap();
    let mut s = f.open("test.Editor");
    let b = Config::default().hotkeys.composition_bar;
    f.hotkey(&mut s, &b);
    let rules = zct_core::AppRules::load(&f.dir.path().join("apps.toml")).unwrap();
    assert_eq!(rules.apps.len(), 1);
    let rule = &rules.apps[0];
    assert_eq!(rule.windows.as_deref(), Some("editor.exe"));
    assert_eq!(rule.detect, Some(false));
    assert_eq!(rule.composition_bar, Some(true));
}
