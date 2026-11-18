//! macOS 的 C ABI。所有呼叫在 IMK 主執行緒；handle 不可跨執行緒並行使用。
//! 結果字串由 session 擁有，僅保證在該 session 下一次呼叫前有效。

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use zct_core::paths::DataRoot;
use zct_core::{
    AppRules, Config, Engine, InputMode, Key, KeyEvent, KeyResult, Mods, Session, SessionOptions,
    Side,
};

thread_local! {
    static ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn string(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

fn guard<T: Default>(f: impl FnOnce() -> Result<T, String>) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => value,
        result => {
            let message = match result {
                Ok(Err(e)) => e,
                _ => "Rust 引擎發生非預期錯誤".into(),
            };
            ERROR.with(|e| *e.borrow_mut() = string(&message));
            T::default()
        }
    }
}

unsafe fn text<'a>(p: *const c_char) -> Result<&'a str, String> {
    if p.is_null() {
        return Err("收到空字串指標".into());
    }
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .map_err(|e| e.to_string())
}

pub struct ZctRuntime {
    engine: Arc<Engine>,
    sync: zct_preferences::SyncWorker,
    data: DataRoot,
    dictionary: std::path::PathBuf,
    config_path: CString,
    settings_buffer: CString,
    game: Arc<AtomicBool>,
}

fn open_engine(dictionary: &std::path::Path, data: &DataRoot) -> Result<Arc<Engine>, String> {
    let engine = Engine::open(dictionary, &data.learning()).map_err(|e| e.to_string())?;
    engine.reload_user_words(&data.root().join("dict/english.txt"));
    Ok(Arc::new(engine))
}

pub struct ZctSession {
    inner: Session,
    snapshot: Snapshot,
    game: Arc<AtomicBool>,
}

#[derive(Default)]
struct Snapshot {
    commit: CString,
    preedit: CString,
    notice: CString,
    items: Vec<CString>,
    pointers: Vec<*const c_char>,
    pinned: Vec<bool>,
    columns: Vec<Vec<CString>>,
    column_pointers: Vec<Vec<*const c_char>>,
    column_views: Vec<ZctColumn>,
}

#[repr(C)]
pub struct ZctColumn {
    pub items: *const *const c_char,
    pub count: usize,
}

/// UTF-8 strings; all offsets are UTF-16 code units, as required by NSRange.
#[repr(C)]
#[derive(Default)]
pub struct ZctResult {
    pub ok: bool,
    pub eaten: bool,
    pub commit: *const c_char,
    pub preedit: *const c_char,
    pub cursor: usize,
    pub target_start: usize,
    pub target_length: usize,
    pub candidates: *const *const c_char,
    pub candidate_count: usize,
    pub pinned: *const bool,
    pub columns: *const ZctColumn,
    pub column_count: usize,
    pub first_column: usize,
    pub highlight: usize,
    pub page: usize,
    pub total_pages: usize,
    pub notice: *const c_char,
    pub bell: bool,
    pub reconvert: bool,
    /// 0 = Chinese, 1 = English.
    pub mode: u32,
    /// The candidate items are a display-only typing popup (bopomofo while keys are shown as
    /// letters): no numbers, no selection; stays until the syllable is finished.
    pub display_only: bool,
}

fn utf16_offset(s: &str, chars: usize) -> usize {
    s.chars().take(chars).map(char::len_utf16).sum()
}

impl ZctSession {
    fn result(&mut self, r: KeyResult) -> ZctResult {
        let (start, length) = r.preedit.target.unwrap_or((0, 0));
        let mut out = ZctResult {
            ok: true,
            eaten: r.eaten,
            cursor: utf16_offset(&r.preedit.text, r.preedit.cursor),
            target_start: utf16_offset(&r.preedit.text, start),
            target_length: utf16_offset(&r.preedit.text, start + length)
                - utf16_offset(&r.preedit.text, start),
            bell: r.bell,
            reconvert: r.reconvert,
            mode: u32::from(self.inner.mode() == InputMode::English),
            ..ZctResult::default()
        };
        self.snapshot = Snapshot {
            commit: string(r.commit.as_deref().unwrap_or_default()),
            preedit: string(&r.preedit.text),
            notice: string(r.notice.as_deref().unwrap_or_default()),
            ..Snapshot::default()
        };
        if let Some(c) = r.candidates {
            self.snapshot.items = c.items.iter().map(|s| string(s)).collect();
            self.snapshot.pinned = c.pinned;
            self.snapshot
                .pinned
                .resize(self.snapshot.items.len(), false);
            self.snapshot.columns = c
                .columns
                .iter()
                .map(|column| column.iter().map(|s| string(s)).collect())
                .collect();
            self.snapshot.column_pointers = self
                .snapshot
                .columns
                .iter()
                .map(|column| column.iter().map(|s| s.as_ptr()).collect())
                .collect();
            self.snapshot.column_views = self
                .snapshot
                .column_pointers
                .iter()
                .map(|column| ZctColumn {
                    items: column.as_ptr(),
                    count: column.len(),
                })
                .collect();
            out.first_column = c.first_column;
            out.display_only = c.highlight == zct_core::CandidatePage::DISPLAY_ONLY;
            // DISPLAY_ONLY is usize::MAX, which would trap when Swift converts it to Int
            out.highlight = if out.display_only { 0 } else { c.highlight };
            out.page = c.page;
            out.total_pages = c.total_pages;
        }
        self.snapshot.pointers = self.snapshot.items.iter().map(|s| s.as_ptr()).collect();
        out.commit = self.snapshot.commit.as_ptr();
        out.preedit = self.snapshot.preedit.as_ptr();
        out.notice = self.snapshot.notice.as_ptr();
        out.candidates = self.snapshot.pointers.as_ptr();
        out.candidate_count = self.snapshot.pointers.len();
        out.pinned = self.snapshot.pinned.as_ptr();
        out.columns = self.snapshot.column_views.as_ptr();
        out.column_count = self.snapshot.column_views.len();
        out
    }
}

/// Last error on this thread. Valid until the next failed call on this thread.
#[unsafe(no_mangle)]
pub extern "C" fn zct_last_error() -> *const c_char {
    ERROR.with(|e| e.borrow().as_ptr())
}

/// # Safety
/// `dict` must point to a NUL-terminated UTF-8 dictionary path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_new(dict: *const c_char) -> *mut ZctRuntime {
    guard(|| {
        let dict = unsafe { text(dict)? };
        if std::env::var_os(zct_core::paths::ENV_HOME).is_none() {
            let _ = zct_core::paths::migrate_legacy();
            let _ = zct_core::paths::seed_dev_from_official();
        }
        let data = DataRoot::discover().ok_or("找不到使用者資料目錄")?;
        std::fs::create_dir_all(data.root()).map_err(|e| e.to_string())?;
        if !data.config().exists() {
            Config::default()
                .save(&data.config())
                .map_err(|e| e.to_string())?;
        }
        let engine = open_engine(std::path::Path::new(dict), &data)?;
        let config_path = string(&data.config().to_string_lossy());
        Ok(Box::into_raw(Box::new(ZctRuntime {
            engine,
            sync: zct_preferences::SyncWorker::default(),
            data,
            dictionary: dict.into(),
            config_path,
            settings_buffer: CString::default(),
            game: Arc::new(AtomicBool::new(false)),
        })))
    })
}

/// # Safety
/// `runtime` is a live runtime handle, or NULL. Call before destroying it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_save(runtime: *mut ZctRuntime) -> bool {
    guard(|| {
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        runtime
            .engine
            .save()
            .and_then(|_| runtime.engine.backup(&runtime.data.backup(), 7))
            .map_err(|e| e.to_string())?;
        runtime.sync.tick(&runtime.data, &runtime.engine);
        Ok(true)
    })
}

/// # Safety
/// `runtime` is live. The result is borrowed until data relocation or the runtime is freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_config_path(runtime: *mut ZctRuntime) -> *const c_char {
    guard(|| {
        let runtime = unsafe { runtime.as_ref() }.ok_or("空 runtime")?;
        Ok(runtime.config_path.as_ptr())
    })
}

/// Load config or App rules as JSON. Borrowed until the next settings call.
/// # Safety
/// `runtime` is live and exclusively accessed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_settings(
    runtime: *mut ZctRuntime,
    apps: bool,
) -> *const c_char {
    guard(|| {
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        let value = if apps {
            let rules = zct_preferences::SettingsStore::new(runtime.data.clone()).apps()?;
            serde_json::to_string(&rules)
        } else {
            let config = zct_preferences::SettingsStore::new(runtime.data.clone()).config()?;
            serde_json::to_string(&config)
        }
        .map_err(|e| e.to_string())?;
        runtime.settings_buffer = string(&value);
        Ok(runtime.settings_buffer.as_ptr())
    })
}

/// Validate and atomically save config or App rules using the core's parser.
/// # Safety
/// `runtime` is live; `value` is NUL-terminated UTF-8 JSON, borrowed for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_save_settings(
    runtime: *mut ZctRuntime,
    apps: bool,
    value: *const c_char,
) -> bool {
    guard(|| {
        let runtime = unsafe { runtime.as_ref() }.ok_or("空 runtime")?;
        let value = unsafe { text(value)? };
        let store = zct_preferences::SettingsStore::new(runtime.data.clone());
        if apps {
            let rules: AppRules = serde_json::from_str(value).map_err(|e| e.to_string())?;
            store.save_apps(&rules)?;
        } else {
            let config: Config = serde_json::from_str(value).map_err(|e| e.to_string())?;
            store.save_config(&config)?;
        }
        Ok(true)
    })
}

/// Shared settings request/reply API. Return strings are borrowed until the next settings call.
/// # Safety
/// `runtime` is live and exclusive; `request` is NUL-terminated UTF-8 JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_preferences(
    runtime: *mut ZctRuntime,
    request: *const c_char,
) -> *const c_char {
    guard(|| {
        use zct_preferences::{Action, EngineBackend, Manager, Platform, Request};
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        let request: Request =
            serde_json::from_str(unsafe { text(request)? }).map_err(|e| e.to_string())?;
        match &request {
            Request::Manage {
                action: Action::SyncNow,
            } => {
                let reply = runtime.sync.request(&runtime.data, &runtime.engine)?;
                runtime.settings_buffer = string(
                    &serde_json::to_string(&zct_preferences::Response::Management { reply })
                        .map_err(|e| e.to_string())?,
                );
                return Ok(runtime.settings_buffer.as_ptr());
            }
            Request::Manage {
                action: Action::SyncStatus,
            } => {
                let reply = runtime.sync.status(&runtime.data);
                runtime.settings_buffer = string(
                    &serde_json::to_string(&zct_preferences::Response::Management { reply })
                        .map_err(|e| e.to_string())?,
                );
                return Ok(runtime.settings_buffer.as_ptr());
            }
            Request::Manage {
                action: Action::Restore { .. },
            } if runtime.sync.running() => return Err("請等待 git 同步完成後再還原".into()),
            Request::Save { .. } if runtime.sync.running() => {
                return Err("請等待 git 同步完成後再儲存設定".into());
            }
            _ => {}
        }
        let restore = matches!(
            &request,
            Request::Manage {
                action: Action::Restore { .. }
            }
        );
        let store = zct_preferences::SettingsStore::new(runtime.data.clone());
        let backup = runtime.data.backup();
        let result = Manager::new(
            store,
            EngineBackend::new(&runtime.engine, &backup),
            Platform::Macos,
        )
        .execute(request);
        // restore freezes the old engine; never leave the UI editing that instance.
        if restore {
            runtime.engine = open_engine(&runtime.dictionary, &runtime.data)?;
        }
        runtime.settings_buffer =
            string(&serde_json::to_string(&result?).map_err(|e| e.to_string())?);
        Ok(runtime.settings_buffer.as_ptr())
    })
}

/// Relocate shared data, preserving the old folder. Sessions must reactivate afterwards.
/// # Safety
/// `runtime` is live and exclusive; `destination` is NUL-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_relocate(
    runtime: *mut ZctRuntime,
    destination: *const c_char,
) -> *const c_char {
    guard(|| {
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        if std::env::var_os(zct_core::paths::ENV_HOME).is_some() {
            return Err("ZCT_HOME 已指定資料位置".into());
        }
        if runtime.sync.running() {
            return Err("請等待 git 同步完成後再搬移資料".into());
        }
        let destination = std::path::PathBuf::from(unsafe { text(destination)? });
        let pointer = zct_core::paths::pointer_dir().ok_or("找不到資料位置指標")?;
        runtime.engine.save().map_err(|e| e.to_string())?;
        runtime.engine.freeze();
        let result =
            zct_preferences::relocate_directory(&pointer, runtime.data.root(), &destination);
        if result.is_ok() {
            runtime.data = DataRoot::new(destination);
        }
        // Also recover a frozen engine when relocation failed.
        runtime.engine = open_engine(&runtime.dictionary, &runtime.data)?;
        runtime.config_path = string(&runtime.data.config().to_string_lossy());
        result?;
        let snapshot = zct_preferences::SettingsStore::new(runtime.data.clone()).load()?;
        runtime.settings_buffer =
            string(&serde_json::to_string(&snapshot).map_err(|e| e.to_string())?);
        Ok(runtime.settings_buffer.as_ptr())
    })
}

/// # Safety
/// `runtime` was returned by zct_runtime_new and must be freed exactly once (NULL is allowed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_free(runtime: *mut ZctRuntime) {
    guard(|| {
        if !runtime.is_null() {
            let runtime = unsafe { Box::from_raw(runtime) };
            runtime.engine.save().map_err(|e| e.to_string())?;
        }
        Ok(())
    });
}

/// # Safety
/// `runtime` must be live; `app` must be a NUL-terminated UTF-8 bundle identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_new(
    runtime: *mut ZctRuntime,
    app: *const c_char,
) -> *mut ZctSession {
    guard(|| {
        let runtime = unsafe { runtime.as_ref() }.ok_or("空 runtime")?;
        let app = unsafe { text(app)? };
        let mut config = Config::load(&runtime.data.config()).map_err(|e| e.to_string())?;
        // macOS 的 AI 背景調度尚未接入，不讓快捷鍵進入無法完成的等待狀態。
        config.ai.enabled = false;
        let rules =
            AppRules::load(&runtime.data.root().join("apps.toml")).map_err(|e| e.to_string())?;
        let inner = Session::new(&SessionOptions {
            engine: runtime.engine.clone(),
            config,
            rule: rules.find(app).cloned(),
        })
        .map_err(|e| e.to_string())?;
        Ok(Box::into_raw(Box::new(ZctSession {
            inner,
            snapshot: Snapshot::default(),
            game: runtime.game.clone(),
        })))
    })
}

/// # Safety
/// A live session handle, freed exactly once; NULL is allowed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_free(session: *mut ZctSession) {
    guard(|| {
        if !session.is_null() {
            drop(unsafe { Box::from_raw(session) });
        }
        Ok(())
    });
}

/// # Safety
/// `session` is live and exclusively accessed. Mode: 0 = Chinese, 1 = English.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_restore_mode(session: *mut ZctSession, mode: u32) {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        session.inner.restore_mode(if mode == 1 {
            InputMode::English
        } else {
            InputMode::Chinese
        });
        Ok(())
    });
}

/// Modifier mask: shift=1, control=2, option=4, command=8, caps lock=16, right option=32.
/// Key codes are macOS virtual key codes, independent of keyboard layout.
/// # Safety
/// `session` must be live and exclusively accessed for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_key(
    session: *mut ZctSession,
    code: u16,
    modifiers: u32,
    released: bool,
    time_ms: u64,
) -> ZctResult {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        let ev = KeyEvent {
            key: mac_key(code),
            mods: Mods {
                shift: modifiers & 1 != 0,
                ctrl: modifiers & 2 != 0,
                alt: modifiers & 4 != 0,
                meta: modifiers & 8 != 0,
                caps_lock: modifiers & 16 != 0,
                right_alt: modifiers & 32 != 0,
            },
            released,
            time_ms,
        };
        session
            .inner
            .set_global_game(session.game.load(Ordering::Relaxed));
        let mut r = session.inner.process_key(&ev);
        if r.toggle_game {
            let on = !session.game.fetch_xor(true, Ordering::Relaxed);
            session.inner.set_global_game(on);
            r.notice = Some(
                if on {
                    "遊戲模式：開"
                } else {
                    "遊戲模式：關"
                }
                .into(),
            );
        }
        if r.toggle_composition_bar {
            r.notice = Some("macOS 使用游標旁的組字提示；目前不支援個別 App 提示列切換".into());
        }
        if r.notice.is_none() {
            r.notice = r
                .mode_changed
                .map(|m| {
                    if m == InputMode::Chinese {
                        "中文"
                    } else {
                        "英文"
                    }
                    .into()
                })
                .or_else(|| {
                    r.width_changed.map(|w| {
                        if w == zct_core::Width::Full {
                            "全形"
                        } else {
                            "半形"
                        }
                        .into()
                    })
                })
                .or_else(|| {
                    r.output_changed.map(|o| {
                        if o == zct_core::Output::Traditional {
                            "繁體"
                        } else {
                            "簡體"
                        }
                        .into()
                    })
                });
        }
        Ok(session.result(r))
    })
}

/// # Safety
/// `session` is a live, exclusively accessed handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_flush(session: *mut ZctSession) -> ZctResult {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        let commit = session.inner.flush_output();
        session.inner.reset();
        Ok(session.result(KeyResult {
            commit: Some(commit),
            ..KeyResult::default()
        }))
    })
}

/// # Safety
/// `session` is live; `value` is NUL-terminated UTF-8 selected text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_reconvert(
    session: *mut ZctSession,
    value: *const c_char,
) -> ZctResult {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        let value = unsafe { text(value)? };
        let r = session.inner.reconvert(value);
        Ok(session.result(r))
    })
}

fn mac_key(code: u16) -> Key {
    use Key::*;
    match code {
        0 => Char(b'a'),
        1 => Char(b's'),
        2 => Char(b'd'),
        3 => Char(b'f'),
        4 => Char(b'h'),
        5 => Char(b'g'),
        6 => Char(b'z'),
        7 => Char(b'x'),
        8 => Char(b'c'),
        9 => Char(b'v'),
        11 => Char(b'b'),
        12 => Char(b'q'),
        13 => Char(b'w'),
        14 => Char(b'e'),
        15 => Char(b'r'),
        16 => Char(b'y'),
        17 => Char(b't'),
        18 => Char(b'1'),
        19 => Char(b'2'),
        20 => Char(b'3'),
        21 => Char(b'4'),
        22 => Char(b'6'),
        23 => Char(b'5'),
        24 => Char(b'='),
        25 => Char(b'9'),
        26 => Char(b'7'),
        27 => Char(b'-'),
        28 => Char(b'8'),
        29 => Char(b'0'),
        30 => Char(b']'),
        31 => Char(b'o'),
        32 => Char(b'u'),
        33 => Char(b'['),
        34 => Char(b'i'),
        35 => Char(b'p'),
        37 => Char(b'l'),
        38 => Char(b'j'),
        39 => Char(b'\''),
        40 => Char(b'k'),
        41 => Char(b';'),
        42 => Char(b'\\'),
        43 => Char(b','),
        44 => Char(b'/'),
        45 => Char(b'n'),
        46 => Char(b'm'),
        47 => Char(b'.'),
        50 => Char(b'`'),
        36 | 76 => Enter,
        48 => Tab,
        49 => Space,
        51 => Backspace,
        53 => Esc,
        56 => Shift(Side::Left),
        60 => Shift(Side::Right),
        59 | 62 => Ctrl,
        58 | 61 => Alt,
        54 | 55 => Meta,
        57 => CapsLock,
        65 => Numpad(b'.'),
        67 => Numpad(b'*'),
        69 => Numpad(b'+'),
        75 => Numpad(b'/'),
        78 => Numpad(b'-'),
        82 => Numpad(b'0'),
        83 => Numpad(b'1'),
        84 => Numpad(b'2'),
        85 => Numpad(b'3'),
        86 => Numpad(b'4'),
        87 => Numpad(b'5'),
        88 => Numpad(b'6'),
        89 => Numpad(b'7'),
        91 => Numpad(b'8'),
        92 => Numpad(b'9'),
        115 => Home,
        116 => PageUp,
        117 => Delete,
        119 => End,
        121 => PageDown,
        123 => Left,
        124 => Right,
        125 => Down,
        126 => Up,
        122 => F(1),
        120 => F(2),
        99 => F(3),
        118 => F(4),
        96 => F(5),
        97 => F(6),
        98 => F(7),
        100 => F(8),
        101 => F(9),
        109 => F(10),
        103 => F(11),
        111 => F(12),
        105 => F(13),
        107 => F(14),
        113 => F(15),
        106 => F(16),
        64 => F(17),
        79 => F(18),
        80 => F(19),
        90 => F(20),
        _ => Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(config: Config) -> (tempfile::TempDir, Box<ZctRuntime>) {
        let temp = tempfile::tempdir().unwrap();
        let data = DataRoot::new(temp.path());
        config.save(&data.config()).unwrap();
        let dict = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/dict");
        let engine = Arc::new(Engine::open(&dict, &data.learning()).unwrap());
        let config_path = string(&data.config().to_string_lossy());
        (
            temp,
            Box::new(ZctRuntime {
                engine,
                sync: zct_preferences::SyncWorker::default(),
                data,
                dictionary: dict,
                config_path,
                settings_buffer: CString::default(),
                game: Arc::new(AtomicBool::new(false)),
            }),
        )
    }

    unsafe fn press(session: *mut ZctSession, code: u16, mods: u32, time: u64) -> ZctResult {
        // Capture the down result only. The ABI's strings expire on the next call.
        unsafe { zct_session_key(session, code, mods, false, time) }
    }

    #[test]
    fn abi_composes_candidates_commits_and_flushes_simplified() {
        let mut config = Config::default();
        config.input.output = zct_core::Output::Simplified;
        let (_dir, mut runtime) = runtime(config);
        let app = string("com.apple.TextEdit");
        unsafe {
            let s = zct_session_new(&mut *runtime, app.as_ptr());
            assert!(!s.is_null());
            for (i, code) in [13, 32, 29, 49, 3, 32, 21].into_iter().enumerate() {
                assert!(press(s, code, 0, i as u64 * 30).eaten);
            }
            let candidates = press(s, 125, 0, 300);
            assert!(candidates.ok && candidates.candidate_count > 0);
            let first = CStr::from_ptr(*candidates.candidates).to_str().unwrap();
            assert!(!first.is_empty());
            let _ = press(s, 53, 0, 330);
            let result = zct_session_flush(s);
            assert_eq!(CStr::from_ptr(result.commit).to_str().unwrap(), "天气");
            assert_eq!(CStr::from_ptr(result.preedit).to_str().unwrap(), "");
            assert!(!press(s, 0, 8, 360).eaten, "Command+A must pass through");
            assert!(zct_runtime_save(&mut *runtime));
            zct_session_free(s);
        }
    }

    #[test]
    fn abi_app_rules_reload_and_game_state_are_shared() {
        let (dir, mut runtime) = runtime(Config::default());
        let app = string("org.example.game");
        unsafe {
            let first = zct_session_new(&mut *runtime, app.as_ptr());
            std::fs::write(
                dir.path().join("apps.toml"),
                "[[app]]\nmacos = 'org.example.game'\ndefault_mode = 'english'\n",
            )
            .unwrap();
            let second = zct_session_new(&mut *runtime, app.as_ptr());
            assert!(
                !press(second, 13, 0, 100).eaten,
                "next activation reloads app rules"
            );
            let toggled = press(first, 111, 3, 200);
            assert!(toggled.eaten);
            // Both sessions observe the same global game switch.
            assert!(!press(first, 13, 0, 300).eaten);
            let _ = press(second, 111, 3, 400);
            assert!(press(first, 13, 0, 500).eaten);
            zct_session_free(first);
            zct_session_free(second);
        }
    }

    #[test]
    fn null_handle_returns_an_error_instead_of_unwinding() {
        unsafe {
            assert!(!zct_session_flush(std::ptr::null_mut()).ok);
            assert!(
                CStr::from_ptr(zct_last_error())
                    .to_str()
                    .unwrap()
                    .contains("session")
            );
            zct_session_free(std::ptr::null_mut());
        }
    }
    #[test]
    fn utf16_ranges_include_surrogate_pairs() {
        assert_eq!(utf16_offset("中😀文", 2), 3);
        assert_eq!(utf16_offset("中😀文", 3), 4);
    }

    #[test]
    fn settings_roundtrip_validates_before_replacing_files() {
        let (_dir, mut runtime) = runtime(Config::default());
        unsafe {
            let p = zct_runtime_settings(&mut *runtime, false);
            let mut settings: serde_json::Value =
                serde_json::from_str(CStr::from_ptr(p).to_str().unwrap()).unwrap();
            settings["theme"]["font_size"] = 22.into();
            settings["theme"]["layout"] = "horizontal".into();
            let input = string(&settings.to_string());
            assert!(zct_runtime_save_settings(
                &mut *runtime,
                false,
                input.as_ptr()
            ));
            assert_eq!(
                Config::load(&runtime.data.config())
                    .unwrap()
                    .theme
                    .font_size,
                22.0
            );
            settings["theme"]["font_size"] = 200.into();
            let input = string(&settings.to_string());
            assert!(!zct_runtime_save_settings(
                &mut *runtime,
                false,
                input.as_ptr()
            ));
            assert_eq!(
                Config::load(&runtime.data.config())
                    .unwrap()
                    .theme
                    .font_size,
                22.0
            );
            let rules = string(
                r#"{"app":[{"name":"Code","windows":"Code.exe","macos":"com.microsoft.VSCode","default_mode":"english"}]}"#,
            );
            assert!(zct_runtime_save_settings(
                &mut *runtime,
                true,
                rules.as_ptr()
            ));
            let p = zct_runtime_settings(&mut *runtime, true);
            let value: serde_json::Value =
                serde_json::from_str(CStr::from_ptr(p).to_str().unwrap()).unwrap();
            assert_eq!(value["app"][0]["windows"], "Code.exe");
        }
    }
    #[test]
    fn physical_keys_preserve_modifiers_and_numpad() {
        assert_eq!(mac_key(13), Key::Char(b'w'));
        assert_eq!(mac_key(60), Key::Shift(Side::Right));
        assert_eq!(mac_key(83), Key::Numpad(b'1'));
        assert_eq!(mac_key(111), Key::F(12));
        assert_eq!(mac_key(255), Key::Other);
    }
}
