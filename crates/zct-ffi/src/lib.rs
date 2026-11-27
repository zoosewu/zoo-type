//! macOS 的 C ABI。所有呼叫在 IMK 主執行緒；handle 不可跨執行緒並行使用。
//! 結果字串由 session 擁有，僅保證在該 session 下一次呼叫前有效。

#[cfg(test)]
mod ai;

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use zct_core::{AppRules, Config, InputMode, Key, KeyEvent, KeyResult, Mods, Side};
use zct_host::{Host, HostSession};

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
    /// 資料目錄、引擎、存檔與同步、設定熱載入、遊戲模式、中英狀態記憶（與 Windows 服務程序共用）
    host: Arc<Host>,
    next_session: u64,
    config_path: CString,
    settings_buffer: CString,
    /// 自動更新：查、下載、驗證、排程（共用）；只有正式版
    updater: Option<Arc<zct_host::update::Updater>>,
    update_available: CString,
    update_message: CString,
}

impl ZctRuntime {
    fn new(host: Host) -> ZctRuntime {
        let config_path = string(&host.data().config().to_string_lossy());
        ZctRuntime {
            host: Arc::new(host),
            next_session: 1,
            config_path,
            settings_buffer: CString::default(),
            updater: None,
            update_available: CString::default(),
            update_message: CString::default(),
        }
    }

    fn store(&self) -> zct_preferences::SettingsStore {
        zct_preferences::SettingsStore::new(self.host.data())
    }
}

pub struct ZctSession {
    inner: HostSession,
    snapshot: Snapshot,
    /// 三個彈出視窗怎麼顯示（規則與 Windows 共用）
    popups: zct_core::popup::Popups<ZctRect>,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct ZctRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Default)]
struct Snapshot {
    commit: CString,
    preedit: CString,
    notice: CString,
    items: Vec<CString>,
    pointers: Vec<*const c_char>,
    labels: Vec<CString>,
    label_pointers: Vec<*const c_char>,
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
    pub has_highlight: bool,
    pub labels: *const *const c_char,
    pub ai_pending: bool,
    pub toggle_composition_bar: bool,
    pub game: bool,
    pub simplified: bool,
    /// 這個 App 目前要不要顯示組字提示列（App 規則 > 終端機 > 全域；快捷鍵切換後已寫回 apps.toml）
    pub composition_bar: bool,
}

impl ZctSession {
    fn result(&mut self, r: KeyResult) -> ZctResult {
        let display = r.preedit.display();
        let (start, length) = display.target_utf16().unwrap_or_default();
        let mut out = ZctResult {
            ok: true,
            ai_pending: self.inner.ai_pending(),
            toggle_composition_bar: r.toggle_composition_bar,
            composition_bar: self.inner.composition_bar(),
            game: self.inner.game(),
            simplified: self.inner.simplified(),
            eaten: r.eaten,
            cursor: display.cursor_utf16(),
            target_start: start,
            target_length: length,
            bell: r.bell,
            reconvert: r.reconvert,
            mode: u32::from(self.inner.session().mode() == InputMode::English),
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
            self.snapshot.labels = (0..c.items.len())
                .map(|i| string(&c.row_label(i)))
                .collect();
            self.snapshot.label_pointers =
                self.snapshot.labels.iter().map(|s| s.as_ptr()).collect();
            self.snapshot.pinned.clone_from(&c.pinned);
            self.snapshot
                .pinned
                .resize(self.snapshot.items.len(), false);
            self.snapshot.columns = c
                .visible_columns()
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
            out.display_only = c.is_display_only();
            // DISPLAY_ONLY is usize::MAX, which would trap when Swift converts it to Int
            out.highlight = c.selected_row().unwrap_or_default();
            out.has_highlight = c.selected_row().is_some();
            out.page = c.page;
            out.total_pages = c.total_pages;
        }
        self.snapshot.pointers = self.snapshot.items.iter().map(|s| s.as_ptr()).collect();
        out.commit = self.snapshot.commit.as_ptr();
        out.preedit = self.snapshot.preedit.as_ptr();
        out.notice = self.snapshot.notice.as_ptr();
        out.candidates = self.snapshot.pointers.as_ptr();
        out.candidate_count = self.snapshot.pointers.len();
        out.labels = self.snapshot.label_pointers.as_ptr();
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
        let (data, _) = zct_core::paths::prepare_data()?;
        let target = zct_host::update::Target::macos(&data);
        let host = Host::open(zct_host::Platform::Macos, data, dict.into())?;
        let mut runtime = ZctRuntime::new(host);
        if zct_host::update::manages_updates() {
            let updater = zct_host::update::Updater::new(target);
            updater
                .spawn_schedule(runtime.host.clone())
                .map_err(|e| e.to_string())?;
            runtime.updater = Some(updater);
        }
        Ok(Box::into_raw(Box::new(runtime)))
    })
}

/// # Safety
/// `runtime` is a live runtime handle, or NULL. Call before destroying it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_save(runtime: *mut ZctRuntime) -> bool {
    guard(|| {
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        // 存檔、每日備份、到了間隔就自動同步（唯讀時都不做）
        runtime.host.tick();
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
            let rules = runtime.store().apps()?;
            serde_json::to_string(&rules)
        } else {
            let config = runtime.store().config()?;
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
        let store = runtime.store();
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
        let host = runtime.host.clone();
        let reply = match &request {
            Request::Manage {
                action: Action::SyncNow,
            } => Some(host.sync_now()?),
            Request::Manage {
                action: Action::SyncStatus,
            } => Some(host.sync_status()),
            Request::Manage {
                action: Action::Restore { .. },
            } if host.sync_running() => return Err("請等待 git 同步完成後再還原".into()),
            Request::Save { .. } if host.sync_running() => {
                return Err("請等待 git 同步完成後再儲存設定".into());
            }
            _ => None,
        };
        if let Some(reply) = reply {
            runtime.settings_buffer = string(
                &serde_json::to_string(&zct_preferences::Response::Management { reply })
                    .map_err(|e| e.to_string())?,
            );
            return Ok(runtime.settings_buffer.as_ptr());
        }
        let restore = matches!(
            &request,
            Request::Manage {
                action: Action::Restore { .. }
            }
        );
        let backup = host.data().backup();
        let engine = host.engine();
        let result = Manager::new(
            runtime.store(),
            EngineBackend::new(&engine, &backup),
            Platform::Macos,
        )
        .execute(request);
        // restore freezes the old engine; never leave the UI editing that instance.
        if restore {
            host.reload_engine()?;
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
        let destination = std::path::PathBuf::from(unsafe { text(destination)? });
        let result = runtime.host.relocate(&destination);
        runtime.config_path = string(&runtime.host.data().config().to_string_lossy());
        result?;
        let snapshot = runtime.store().load()?;
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
            runtime.host.save()?;
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
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        let app = unsafe { text(app)? };
        let id = runtime.next_session;
        runtime.next_session += 1;
        // 同一個 App 再次啟用時沿用中英狀態（記在共用主機層）
        let inner = HostSession::open(&runtime.host, id, app, app)?;
        Ok(Box::into_raw(Box::new(ZctSession {
            inner,
            snapshot: Snapshot::default(),
            popups: Default::default(),
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
        let r = session.inner.key(&ev);
        Ok(session.result(r))
    })
}

/// # Safety
/// `session` is a live, exclusively accessed handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_flush(session: *mut ZctSession) -> ZctResult {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        session.popups.reset();
        let r = session.inner.flush();
        Ok(session.result(r))
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

/// # Safety
/// Session is live and exclusively owned by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_ai_enabled(session: *const ZctSession) -> bool {
    unsafe { session.as_ref() }
        .is_some_and(|session| session.inner.host().settings().current().0.ai.enabled)
}
/// Set a session credential from the platform keychain; never persisted in config.
/// # Safety
/// Session is exclusive and key is NUL-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_ai_key(session: *mut ZctSession, key: *const c_char) -> bool {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        // 金鑰由 Keychain 提供，所有 session 共用；不寫進設定檔
        session
            .inner
            .host()
            .set_ai_key(Some(unsafe { text(key)? }.to_owned()));
        Ok(true)
    })
}
/// Poll an AI mailbox on the IMK thread. False means no completed response.
/// # Safety
/// Session is exclusive and result is a valid output pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_poll(
    session: *mut ZctSession,
    result: *mut ZctResult,
) -> bool {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        let result = unsafe { result.as_mut() }.ok_or("空結果")?;
        match session.inner.poll_ai() {
            Some(r) => {
                *result = session.result(r);
                Ok(true)
            }
            None => Ok(false),
        }
    })
}
/// Native menu/mouse actions: 0 mode, 1 output, 2 global game, 3 next page, 4 previous page.
/// # Safety
/// Session is live and exclusive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_command(session: *mut ZctSession, command: u32) -> ZctResult {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        let r = match command {
            0 => session.inner.toggle_mode(),
            1 => session.inner.toggle_output(),
            2 => session.inner.toggle_game(),
            3 => session.inner.page(true),
            4 => session.inner.page(false),
            5 => session.inner.current(),
            _ => return Err("未知選單命令".into()),
        };
        Ok(session.result(r))
    })
}
/// # Safety
/// Session is live and exclusive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_choose(
    session: *mut ZctSession,
    page: usize,
    row: usize,
) -> ZctResult {
    guard(|| {
        let session = unsafe { session.as_mut() }.ok_or("空 session")?;
        let r = session.inner.choose(page, row);
        if !r.eaten {
            return Err("候選已改變或不可選取".into());
        }
        Ok(session.result(r))
    })
}
/// 設定或引擎變了、而且沒有組字：換成新設定（沿用中英等狀態）。回傳有沒有換，換了就重新讀外觀與畫面
/// # Safety
/// Session is live and exclusive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_refresh(session: *mut ZctSession) -> bool {
    guard(|| {
        Ok(unsafe { session.as_mut() }
            .ok_or("空 session")?
            .inner
            .refresh())
    })
}
/// 不在輸入狀態（按鍵交給 App）：平台收集訊號，規則與 Windows 共用（`zct_core::detect`）。
/// `secure`：安全輸入；`text_context`：有選取範圍、正在組字或是終端機；`covers_screen`：前景視窗蓋滿螢幕
/// # Safety
/// Session is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_not_input(
    session: *const ZctSession,
    secure: bool,
    text_context: bool,
    has_caret: bool,
    covers_screen: bool,
) -> bool {
    guard(|| {
        let session = unsafe { session.as_ref() }.ok_or("空 session")?;
        let signals = zct_core::detect::Signals {
            blocked: secure,
            text_context,
            has_caret,
            covers_screen,
            exclusive_fullscreen: false,
        };
        Ok(zct_core::detect::not_input(
            &signals,
            session.inner.session().heuristics(),
        ))
    })
}

/// # Safety
/// Session is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_is_idle(session: *const ZctSession) -> bool {
    guard(|| {
        Ok(unsafe { session.as_ref() }
            .ok_or("空 session")?
            .inner
            .is_idle())
    })
}
/// Freeze disk writes before replacing the installed app. Caller stops input first.
/// # Safety
/// Runtime is live and exclusive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_freeze(runtime: *mut ZctRuntime) -> bool {
    guard(|| {
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        runtime.host.freeze()?;
        Ok(true)
    })
}
/// Recover the live engine after an update transaction was rolled back.
/// # Safety
/// Runtime is live and exclusive; host reactivates existing sessions afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_runtime_recover(runtime: *mut ZctRuntime) -> bool {
    guard(|| {
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        runtime.host.recover()?;
        Ok(true)
    })
}
/// 這個 App 是不是純終端機（預設顯示組字提示列；清單與 Windows 共用）
/// # Safety
/// `app` is a NUL-terminated UTF-8 bundle identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_is_terminal(app: *const c_char) -> bool {
    guard(|| Ok(zct_host::is_terminal(unsafe { text(app)? })))
}
/// 自動更新的狀態；字串借用到下一次更新相關呼叫
#[repr(C)]
pub struct ZctUpdateStatus {
    /// 正式版才管更新（開發版、ZCT_HOME 隔離測試不管）
    pub supported: bool,
    pub checking: bool,
    /// 正在檢查、下載或安裝
    pub busy: bool,
    /// 已下載驗證、等使用者按更新的版本；沒有是空字串
    pub available: *const c_char,
    pub message: *const c_char,
}

/// # Safety
/// `runtime` is live; call on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_update_status(runtime: *mut ZctRuntime) -> ZctUpdateStatus {
    let empty = c"".as_ptr();
    let mut out = ZctUpdateStatus {
        supported: false,
        checking: false,
        busy: false,
        available: empty,
        message: empty,
    };
    let Some(runtime) = (unsafe { runtime.as_mut() }) else {
        return out;
    };
    let Some(updater) = &runtime.updater else {
        return out;
    };
    let status = updater.status();
    runtime.update_available = string(status.available.as_deref().unwrap_or_default());
    runtime.update_message = string(&status.message);
    out.supported = true;
    out.checking = status.checking;
    out.busy = status.state().busy();
    out.available = runtime.update_available.as_ptr();
    out.message = runtime.update_message.as_ptr();
    out
}

/// 平台開始（`true`）或放棄（`false`）安裝新版：選單顯示「正在安裝 vX…」
/// # Safety
/// `runtime` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_update_installing(runtime: *mut ZctRuntime, on: bool) {
    if let Some(updater) = unsafe { runtime.as_ref() }.and_then(|r| r.updater.as_ref()) {
        updater.set_installing(on);
    }
}

/// 在背景立刻檢查更新；結果看 `zct_update_status`
/// # Safety
/// `runtime` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_update_check(runtime: *mut ZctRuntime) -> bool {
    guard(|| {
        let runtime = unsafe { runtime.as_ref() }.ok_or("空 runtime")?;
        let updater = runtime.updater.as_ref().ok_or("只有正式版會檢查更新")?;
        updater.check_in_background();
        Ok(true)
    })
}

/// 使用者按「更新」：下載（已下載就略過）並驗證，回傳要安裝的檔案路徑（用 `zct_string_free` 釋放）。
/// 會等下載完，請在背景執行緒呼叫；下載進度看 `zct_update_status`。失敗是 NULL 並設定錯誤
/// # Safety
/// `runtime` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_update_prepare(runtime: *const ZctRuntime) -> *mut c_char {
    let path = guard(|| {
        let runtime = unsafe { runtime.as_ref() }.ok_or("空 runtime")?;
        let updater = runtime.updater.as_ref().ok_or("只有正式版會更新")?;
        let (_, file) = updater.prepare()?;
        Ok(Some(string(&file.to_string_lossy())))
    });
    path.map_or(std::ptr::null_mut(), CString::into_raw)
}

/// 平台安裝失敗：顯示原因，可以再按一次
/// # Safety
/// `runtime` is live; `reason` is NULL or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_update_failed(runtime: *const ZctRuntime, reason: *const c_char) {
    if let Some(updater) = unsafe { runtime.as_ref() }.and_then(|r| r.updater.as_ref()) {
        let reason = unsafe { reason.as_ref() }
            .map(|r| unsafe { CStr::from_ptr(r) }.to_string_lossy().into_owned())
            .unwrap_or_default();
        updater.failed(&reason);
    }
}

/// 平台量文字：`font` 0 = 候選字、1 = 數字；寫回寬與高（點）
pub type ZctMeasure = Option<
    unsafe extern "C" fn(
        context: *mut std::ffi::c_void,
        text: *const c_char,
        font: u32,
        width: *mut f32,
        height: *mut f32,
    ),
>;

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct PopupPage {
    items: Vec<String>,
    page: usize,
    total_pages: usize,
    /// 沒有選擇框時大於等於 items 的長度
    highlight: usize,
    pinned: Vec<bool>,
    columns: Vec<Vec<String>>,
    first_column: usize,
    display_only: bool,
}

#[derive(serde::Deserialize)]
struct PopupComposition {
    text: String,
    /// UTF-16（NSRange 的單位）
    cursor: usize,
    target: Option<(usize, usize)>,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct PopupRequest {
    page: Option<PopupPage>,
    composition: Option<PopupComposition>,
    notice: String,
    horizontal: bool,
}

/// UTF-16 位置換成第幾個字
fn char_index(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.chars().enumerate() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    text.chars().count()
}

/// 面板內容的排版（與 Windows 共用 `zct_core::popup`）。`request` 是 JSON：
/// `{"page": {...候選頁, "display_only"}, "composition": {"text", "cursor", "target": [起點, 長度]}, "notice", "horizontal"}`
/// （位置是 UTF-16）。回傳 JSON `{"width", "height", "composition", "candidates", "notice"}`，
/// 各部分由上往下疊好；用 `zct_string_free` 釋放
/// # Safety
/// `request` is NUL-terminated UTF-8; `measure` is called synchronously on this thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_popup_layout(
    request: *const c_char,
    measure: ZctMeasure,
    context: *mut std::ffi::c_void,
) -> *mut c_char {
    guard(|| {
        use zct_core::popup;
        let request: PopupRequest =
            serde_json::from_str(unsafe { text(request)? }).map_err(|e| e.to_string())?;
        let measure = measure.ok_or("沒有量文字的函式")?;
        let mut m = |s: &str, font: popup::Font| {
            let (mut w, mut h) = (0.0f32, 0.0f32);
            let s = string(s);
            let font = u32::from(font == popup::Font::Number);
            unsafe { measure(context, s.as_ptr(), font, &mut w, &mut h) };
            (w, h)
        };
        let composition = request.composition.map(|c| {
            let chars = c.text.chars().count();
            let preedit = zct_core::Preedit {
                cursor: char_index(&c.text, c.cursor),
                target: c.target.filter(|t| t.1 > 0).map(|(start, len)| {
                    let s = char_index(&c.text, start);
                    (s, char_index(&c.text, start + len) - s)
                }),
                len: chars,
                text: c.text,
            };
            popup::composition(&preedit, &mut m)
        });
        let candidates = request.page.map(|p| {
            let page = zct_core::CandidatePage {
                highlight: if p.display_only {
                    zct_core::CandidatePage::DISPLAY_ONLY
                } else {
                    p.highlight
                },
                items: p.items,
                page: p.page,
                total_pages: p.total_pages,
                pinned: p.pinned,
                columns: p.columns,
                first_column: p.first_column,
            };
            popup::candidates(&page, request.horizontal, &mut m)
        });
        let notice = popup::hint(&request.notice, &mut m);
        let ([composition, candidates, notice], (width, height)) = popup::arrange([
            composition.unwrap_or_default(),
            candidates.unwrap_or_default(),
            notice,
        ]);
        let json = serde_json::json!({
            "width": width,
            "height": height,
            "composition": composition,
            "candidates": candidates,
            "notice": notice,
        });
        Ok(string(&json.to_string()).into_raw())
    })
}

#[repr(C)]
#[derive(Default)]
pub struct ZctPoint {
    pub x: f64,
    pub y: f64,
}

/// 面板的位置（與 Windows 共用規則：預設在游標下方，放不下改上方，不超出螢幕）。
/// 座標往上為正：`anchor`、`work` 與回傳值都是左下角
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn zct_popup_place(
    anchor_x: f64,
    anchor_y: f64,
    anchor_w: f64,
    anchor_h: f64,
    width: f64,
    height: f64,
    work_x: f64,
    work_y: f64,
    work_w: f64,
    work_h: f64,
    above: bool,
) -> ZctPoint {
    use zct_core::popup::{OFFSET, Rect, place_y_up};
    let rect = |x: f64, y: f64, w: f64, h: f64| Rect {
        x: x as f32,
        y: y as f32,
        w: w as f32,
        h: h as f32,
    };
    let (x, y) = place_y_up(
        rect(anchor_x, anchor_y, anchor_w, anchor_h),
        (width as f32, height as f32),
        rect(work_x, work_y, work_w, work_h),
        OFFSET,
        above,
    );
    ZctPoint {
        x: x.into(),
        y: y.into(),
    }
}

/// 一個彈出視窗這次怎麼處理：`action` 0 收起、1 顯示在 `anchor`、2 不動；`above` 優先放上方
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct ZctPopupWindow {
    pub action: u8,
    pub anchor: ZctRect,
    pub above: bool,
}

/// 這次按鍵後三個彈出視窗怎麼顯示（與 Windows 共用 `zct_core::popup::Popups`）
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct ZctPopupPlan {
    pub bar: ZctPopupWindow,
    pub candidates: ZctPopupWindow,
    pub hint: ZctPopupWindow,
}

/// `caret`：App 的游標；`fresh`：`caret` 是 App 回報的（量不到改用沿用的位置時是 false）
/// # Safety
/// `session` is live and exclusive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_session_popup_plan(
    session: *mut ZctSession,
    composing: bool,
    composition_bar: bool,
    candidates: bool,
    notice: bool,
    caret: ZctRect,
    fresh: bool,
) -> ZctPopupPlan {
    use zct_core::popup::{PopupAction, PopupContent, PopupKind, Popups};
    let content = PopupContent {
        composing,
        composition_bar,
        candidates,
        notice,
    };
    let mut scratch = Popups::default();
    let popups = match unsafe { session.as_mut() } {
        Some(session) => &mut session.popups,
        None => &mut scratch,
    };
    let plan = popups.plan(content, Some(caret), fresh);
    let window = |action: PopupAction<ZctRect>, kind: PopupKind| {
        let (action, anchor) = match action {
            PopupAction::Hide => (0, caret),
            PopupAction::Show(anchor) => (1, anchor.unwrap_or(caret)),
            PopupAction::Keep => (2, caret),
        };
        ZctPopupWindow {
            action,
            anchor,
            above: kind.above(),
        }
    };
    ZctPopupPlan {
        bar: window(plan.bar, PopupKind::Bar),
        candidates: window(plan.candidates, PopupKind::Candidates),
        hint: window(plan.hint, PopupKind::Hint),
    }
}

/// 輸入法選單（與 Windows 共用 `zct_core::menu`）：JSON 陣列，
/// 每項是 `{"kind": "action", "action": "toggle_mode", "label", "checked", "enabled"}` 或 `{"kind": "separator"}`。
/// 更新項目照 `runtime` 的更新狀態。用 `zct_string_free` 釋放
/// # Safety
/// `runtime` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_menu_items(
    runtime: *const ZctRuntime,
    chinese: bool,
    simplified: bool,
    game: bool,
) -> *mut c_char {
    guard(|| {
        let update = unsafe { runtime.as_ref() }
            .and_then(|r| r.updater.as_ref())
            .map_or(zct_core::menu::UpdateState::Off, |u| u.status().state());
        let state = zct_core::menu::MenuState {
            chinese,
            simplified,
            game,
            update,
        };
        let json =
            serde_json::to_string(&zct_core::menu::items(&state)).map_err(|e| e.to_string())?;
        Ok(string(&json).into_raw())
    })
}

/// 提示泡泡顯示多久（秒）
#[unsafe(no_mangle)]
pub extern "C" fn zct_hint_seconds() -> f64 {
    zct_core::popup::HINT_MS as f64 / 1000.0
}

/// 數字字級相對於候選字字級
#[unsafe(no_mangle)]
pub extern "C" fn zct_number_ratio() -> f64 {
    zct_core::popup::NUM_RATIO.into()
}

/// 選擇框圓角
#[unsafe(no_mangle)]
pub extern "C" fn zct_highlight_radius() -> f64 {
    zct_core::popup::ROW_RADIUS.into()
}

/// Stateless AI connection test, safe on a background thread. Caller frees result.
/// # Safety
/// JSON and key are NUL-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_ai_test(config: *const c_char, key: *const c_char) -> *mut c_char {
    guard(|| {
        let config: zct_core::config::AiConfig =
            serde_json::from_str(unsafe { text(config)? }).map_err(|e| e.to_string())?;
        let key = unsafe { text(key)? }.to_owned();
        let endpoint = zct_ai::Endpoint {
            base_url: config.endpoint,
            model: config.model,
            api_key: Some(key).filter(|k| !k.is_empty()),
            timeout: std::time::Duration::from_secs(config.timeout_seconds.max(1)),
        };
        let reply = zct_ai::ping(&endpoint).map_err(|e| e.to_string())?;
        Ok(string(&reply).into_raw())
    })
}
/// # Safety
/// Value was returned by zct_ai_test and freed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
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
    use zct_core::paths::DataRoot;

    fn runtime(config: Config) -> (tempfile::TempDir, Box<ZctRuntime>) {
        let temp = tempfile::tempdir().unwrap();
        let data = DataRoot::new(temp.path());
        config.save(&data.config()).unwrap();
        let dict = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/dict");
        let host = Host::open(zct_host::Platform::Macos, data, dict).unwrap();
        (temp, Box::new(ZctRuntime::new(host)))
    }

    unsafe fn press(session: *mut ZctSession, code: u16, mods: u32, time: u64) -> ZctResult {
        // Capture the down result only. The ABI's strings expire on the next call.
        unsafe { zct_session_key(session, code, mods, false, time) }
    }

    #[test]
    fn abi_popup_plan_matches_shared_rules() {
        let (_dir, mut runtime) = runtime(Config::default());
        let app = string("com.apple.TextEdit");
        let rect = |x: f64| ZctRect {
            x,
            y: 0.0,
            width: 1.0,
            height: 16.0,
        };
        unsafe {
            let s = zct_session_new(&mut *runtime, app.as_ptr());
            let plan = zct_session_popup_plan(s, true, true, true, false, rect(10.0), true);
            assert_eq!(
                (plan.bar.action, plan.candidates.action, plan.hint.action),
                (1, 1, 2)
            );
            assert!(plan.bar.above && !plan.candidates.above && !plan.hint.above);
            let plan = zct_session_popup_plan(s, true, true, true, false, rect(20.0), true);
            assert_eq!(plan.bar.anchor.x, 10.0, "組字中提示列不動");
            assert_eq!(plan.candidates.anchor.x, 20.0, "候選窗跟著游標");
            let plan = zct_session_popup_plan(s, false, true, false, true, rect(30.0), true);
            assert_eq!(
                (plan.bar.action, plan.candidates.action, plan.hint.action),
                (0, 0, 1)
            );
            assert_eq!(plan.hint.anchor.x, 30.0);
            zct_session_free(s);
        }
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
    fn detection_follows_shared_rules_and_app_rules() {
        let (dir, mut runtime) = runtime(Config::default());
        std::fs::write(
            dir.path().join("apps.toml"),
            "[[app]]
macos = 'org.example.chat'
detect = false
",
        )
        .unwrap();
        unsafe {
            let game = string("org.example.game");
            let chat = string("org.example.chat");
            let g = zct_session_new(&mut *runtime, game.as_ptr());
            let c = zct_session_new(&mut *runtime, chat.as_ptr());
            assert!(
                zct_session_not_input(g, true, true, true, false),
                "安全輸入"
            );
            assert!(
                !zct_session_not_input(g, false, true, true, false),
                "一般輸入框"
            );
            assert!(
                zct_session_not_input(g, false, false, false, true),
                "全螢幕遊戲"
            );
            assert!(
                !zct_session_not_input(c, false, false, false, true),
                "App 規則關掉偵測"
            );
            assert!(
                zct_session_not_input(c, true, true, true, false),
                "安全輸入一律放行"
            );
            zct_session_free(g);
            zct_session_free(c);
        }
    }

    unsafe extern "C" fn fixed_measure(
        _: *mut std::ffi::c_void,
        text: *const c_char,
        font: u32,
        width: *mut f32,
        height: *mut f32,
    ) {
        let n = unsafe { CStr::from_ptr(text) }
            .to_str()
            .unwrap()
            .chars()
            .count() as f32;
        unsafe {
            *width = n * if font == 0 { 10.0 } else { 7.0 };
            *height = if font == 0 { 20.0 } else { 15.0 };
        }
    }

    #[test]
    fn popup_layout_matches_windows_and_stacks_parts() {
        // 「天😀氣」：😀 佔兩個 UTF-16，游標 3 = 第 2 個字之後
        let request = string(
            &serde_json::json!({
                "composition": {"text": "天😀氣", "cursor": 3, "target": [3, 1]},
                "page": {"items": ["氣", "汽"], "total_pages": 1, "highlight": 1},
                "notice": "中",
            })
            .to_string(),
        );
        let json = unsafe {
            let raw = zct_popup_layout(request.as_ptr(), Some(fixed_measure), std::ptr::null_mut());
            assert!(!raw.is_null());
            let out = CStr::from_ptr(raw).to_str().unwrap().to_owned();
            zct_string_free(raw);
            out
        };
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let comp = &v["composition"];
        // 游標在兩個字後面：12 + 20 - 0.75
        assert_eq!(comp["caret"]["x"].as_f64().unwrap(), 31.25);
        let cands = &v["candidates"];
        let comp_h = comp["height"].as_f64().unwrap();
        assert_eq!(
            cands["hits"][0]["rect"]["y"].as_f64().unwrap(),
            comp_h + 4.0,
            "候選窗疊在組字列下面"
        );
        assert_eq!(cands["highlights"][0], cands["hits"][1]["rect"]);
        assert_eq!(v["notice"]["runs"][0]["text"], "中");
        let total =
            comp_h + cands["height"].as_f64().unwrap() + v["notice"]["height"].as_f64().unwrap();
        assert_eq!(v["height"].as_f64().unwrap(), total);
        // 只顯示的注音小視窗：沒有點擊範圍
        let request = string(r#"{"page": {"items": ["ㄋㄧ"], "display_only": true}}"#);
        let json = unsafe {
            let raw = zct_popup_layout(request.as_ptr(), Some(fixed_measure), std::ptr::null_mut());
            let out = CStr::from_ptr(raw).to_str().unwrap().to_owned();
            zct_string_free(raw);
            out
        };
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["candidates"]["hits"].as_array().unwrap().is_empty());
    }

    /// 使用者在 macOS 回報的四個問題，照 IMK 送來的事件順序（flagsChanged 也會送進來）重現
    #[test]
    fn macos_reported_shift_width_and_backspace_follow_shared_rules() {
        let (_dir, mut runtime) = runtime(Config::default());
        let app = string("com.apple.TextEdit");
        unsafe {
            let s = zct_session_new(&mut *runtime, app.as_ptr());
            let mut t = 1000;
            let mut ev = |code: u16, mods: u32, released: bool| {
                t += 40;
                zct_session_key(s, code, mods, released, t)
            };
            let mode = |s| zct_session_command(s, 5).mode;
            // 1. 按住 Shift 打 $（Shift + 4）：不切換中英
            assert_eq!(mode(s), 0);
            ev(56, 1, false); // flagsChanged：左 Shift 按下
            ev(21, 1, false); // 4 按下（帶 Shift）
            ev(21, 1, true);
            ev(56, 0, true); // 放開 Shift
            assert_eq!(mode(s), 0, "Shift + 4 不能切換中英");
            assert_eq!(text(zct_session_flush(s).commit).unwrap(), "＄");
            // 單按 Shift 才切換
            ev(56, 1, false);
            ev(56, 0, true);
            assert_eq!(mode(s), 1);
            ev(56, 1, false);
            ev(56, 0, true);
            assert_eq!(mode(s), 0);
            // 2. Shift + 空白切換全形半形
            ev(56, 1, false);
            let r = ev(49, 1, false);
            assert!(r.eaten, "Shift + 空白要吃掉");
            assert_eq!(text(r.notice).unwrap(), "半形");
            ev(49, 1, true);
            ev(56, 0, true);
            assert_eq!(mode(s), 0, "Shift + 空白不切換中英");
            // 3. ㄐㄧㄣ（r u p）按 Backspace：整個刪
            for code in [15, 32, 35] {
                ev(code, 0, false);
                ev(code, 0, true);
            }
            assert_eq!(text(zct_session_command(s, 5).preedit).unwrap(), "ㄐㄧㄣ");
            let r = ev(51, 0, false);
            assert_eq!(text(r.preedit).unwrap(), "", "還沒完成的注音整個刪");
            ev(51, 0, true);
            // 4. 打完的英文單字（hello + 空白）按 Backspace：一次一個字元
            for code in [4, 14, 37, 37, 31, 49] {
                ev(code, 0, false);
                ev(code, 0, true);
            }
            assert_eq!(text(zct_session_command(s, 5).preedit).unwrap(), "hello ");
            for want in ["hello", "hell"] {
                let r = ev(51, 0, false);
                assert_eq!(text(r.preedit).unwrap(), want);
                ev(51, 0, true);
            }
            zct_session_free(s);
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
        let keys = AcceptanceKeys::new(Config::default());
        let session = unsafe { &mut *keys.session };
        let result = session.result(KeyResult {
            preedit: zct_core::Preedit {
                text: "中😀文".into(),
                cursor: 2,
                target: Some((1, 1)),
                len: 3,
            },
            ..Default::default()
        });
        assert_eq!(
            (result.cursor, result.target_start, result.target_length),
            (3, 1, 2)
        );
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
                Config::load(&runtime.host.data().config())
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
                Config::load(&runtime.host.data().config())
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
    /// Exercise real macOS hardware codes through the C ABI, including key-up.
    struct AcceptanceKeys {
        _directory: tempfile::TempDir,
        _runtime: Box<ZctRuntime>,
        session: *mut ZctSession,
        time: u64,
    }
    struct AcceptanceResult {
        preedit: String,
        commit: String,
        candidates: Vec<String>,
        display_only: bool,
    }
    impl AcceptanceKeys {
        fn new(mut config: Config) -> Self {
            config.candidates.language_model = false;
            let (directory, mut runtime) = runtime(config);
            let app = string("com.apple.TextEdit");
            let session = unsafe { zct_session_new(&mut *runtime, app.as_ptr()) };
            assert!(!session.is_null());
            Self {
                _directory: directory,
                _runtime: runtime,
                session,
                time: 1000,
            }
        }
        fn press(&mut self, code: u16) -> AcceptanceResult {
            self.time += 30;
            unsafe {
                let result = zct_session_key(self.session, code, 0, false, self.time);
                assert!(result.ok && result.eaten);
                let owned = AcceptanceResult {
                    preedit: text(result.preedit).unwrap().to_owned(),
                    commit: text(result.commit).unwrap().to_owned(),
                    candidates: (0..result.candidate_count)
                        .map(|i| text(*result.candidates.add(i)).unwrap().to_owned())
                        .collect(),
                    display_only: result.display_only,
                };
                self.time += 30;
                assert!(zct_session_key(self.session, code, 0, true, self.time).ok);
                owned
            }
        }
        fn type_codes(&mut self, codes: &[u16]) -> AcceptanceResult {
            codes.iter().map(|code| self.press(*code)).last().unwrap()
        }
    }
    impl Drop for AcceptanceKeys {
        fn drop(&mut self) {
            unsafe { zct_session_free(self.session) };
        }
    }

    #[test]
    fn native_display_contract_tracks_the_same_composition_and_selected_row() {
        let mut keys = AcceptanceKeys::new(Config::default());
        // 天氣, move left, open the list, move down a row and expand the next column.
        keys.type_codes(&[13, 32, 29, 49, 3, 32, 21]);
        for code in [123, 125, 125, 124] {
            keys.time += 30;
            unsafe {
                let native = zct_session_key(keys.session, code, 0, false, keys.time);
                assert!(native.ok && native.eaten);
                let session = (*keys.session).inner.session();
                let preedit = session.preedit();
                let display = preedit.display();
                assert_eq!(text(native.preedit).unwrap(), "天氣");
                assert_eq!(native.cursor, display.cursor_utf16());
                assert!(!display.target.is_empty());
                assert_eq!(
                    (native.target_start, native.target_length),
                    display.target_utf16().unwrap()
                );
                if code != 123 {
                    let page = session.candidates().unwrap();
                    assert!(native.has_highlight && !native.display_only);
                    assert_eq!(Some(native.highlight), page.selected_row());
                    assert_eq!(native.page, page.page);
                    assert_eq!(native.candidate_count, page.items.len());
                    for (row, item) in page.items.iter().enumerate() {
                        assert_eq!(text(*native.candidates.add(row)).unwrap(), item);
                        assert_eq!(text(*native.labels.add(row)).unwrap(), page.row_label(row));
                    }
                    if code == 124 {
                        assert!(native.column_count > 1);
                        for (i, column) in page.visible_columns().enumerate() {
                            let view = &*native.columns.add(i);
                            assert_eq!(view.count, column.len());
                            for (row, item) in column.iter().enumerate() {
                                assert_eq!(text(*view.items.add(row)).unwrap(), item);
                            }
                        }
                    }
                }
                keys.time += 30;
                assert!(zct_session_key(keys.session, code, 0, true, keys.time).ok);
            }
        }
    }

    #[test]
    fn macos_acceptance_english_detection_and_character_backspace() {
        let mut chinese = AcceptanceKeys::new(Config::default());
        // hello + Space
        assert_eq!(
            chinese.type_codes(&[4, 14, 37, 37, 31, 49]).preedit,
            "hello "
        );
        assert_eq!(chinese.press(36).commit, "hello ");
        let mut config = Config::default();
        config.mixed.english = zct_core::config::EnglishWords::EnglishFirst;
        let mut english = AcceptanceKeys::new(config.clone());
        assert_eq!(english.type_codes(&[37, 31, 9, 14, 49]).preedit, "love ");
        assert_eq!(english.type_codes(&[1, 32, 20]).preedit, "love 你");
        // 標點完成單字，之後回到注音：lo , su3
        let mut punctuation = AcceptanceKeys::new(config.clone());
        assert_eq!(
            punctuation.type_codes(&[37, 31, 43, 1, 32, 20]).preedit,
            "lo,你"
        );
        let mut deletion = AcceptanceKeys::new(config);
        // 還沒按空白的單字：Backspace 整個刪
        deletion.type_codes(&[37, 31, 9, 14]);
        assert_eq!(deletion.press(51).preedit, "");
        assert_eq!(deletion.type_codes(&[1, 32, 20]).preedit, "你");
    }

    #[test]
    fn macos_acceptance_typing_popup_crosses_abi_and_disappears_on_tone() {
        for typing in [
            zct_core::config::TypingDisplay::Keys,
            zct_core::config::TypingDisplay::KeysAndChars,
        ] {
            let mut config = Config::default();
            config.mixed.typing = typing;
            let mut keys = AcceptanceKeys::new(config);
            // Ensure dictionary statistics have finished loading before asserting common characters.
            unsafe {
                (*keys.session)
                    .inner
                    .session()
                    .engine()
                    .char_usage('一', "ㄧ");
            }
            let result = keys.type_codes(&[1, 32]);
            assert_eq!(result.preedit, "su");
            assert!(result.display_only);
            assert_eq!(result.candidates[0], "ㄋㄧ");
            if typing == zct_core::config::TypingDisplay::Keys {
                assert_eq!(result.candidates, ["ㄋㄧ"]);
            } else {
                for common in ["你", "妳", "尼"] {
                    assert!(
                        result.candidates.iter().any(|item| item == common),
                        "{common}: {:?}",
                        result.candidates
                    );
                }
            }
            let finished = keys.press(20);
            assert_eq!(finished.preedit, "你");
            assert!(!finished.display_only && finished.candidates.is_empty());
        }
    }

    #[test]
    fn macos_acceptance_legacy_settings_load_and_save_as_new_fields() {
        for enabled in [false, true] {
            let (_directory, mut runtime) = runtime(Config::default());
            std::fs::write(runtime.host.data().config(), format!("[mixed]\nauto_english = {enabled}\n[detect]\nlegacy_fullscreen = {enabled}\nexclusive_fullscreen = true\n[theme]\nfont_size = 19.5\n")).unwrap();
            unsafe {
                let load = string(r#"{"op":"load"}"#);
                let result = zct_runtime_preferences(&mut *runtime, load.as_ptr());
                assert!(!result.is_null());
                let value: serde_json::Value = serde_json::from_str(text(result).unwrap()).unwrap();
                let snapshot = &value["snapshot"];
                assert_eq!(
                    snapshot["document"]["config"]["mixed"]["english"],
                    if enabled { "chinese_first" } else { "off" }
                );
                assert_eq!(
                    snapshot["document"]["config"]["detect"]["fullscreen"],
                    enabled
                );
                let save = string(&serde_json::json!({"op":"save","document":snapshot["document"],"revision":snapshot["revision"]}).to_string());
                assert!(!zct_runtime_preferences(&mut *runtime, save.as_ptr()).is_null());
            }
            let saved = std::fs::read_to_string(runtime.host.data().config()).unwrap();
            for legacy in ["auto_english", "legacy_fullscreen", "exclusive_fullscreen"] {
                assert!(!saved.contains(legacy));
            }
            assert_eq!(
                Config::load(&runtime.host.data().config())
                    .unwrap()
                    .theme
                    .font_size,
                19.5
            );
        }
    }
    #[test]
    fn macos_ai_runs_in_background_and_rejects_stale_results() {
        for cancel in [false, true] {
            let (endpoint, server) = crate::ai::server("天氣");
            let mut config = Config::default();
            config.ai.enabled = true;
            config.ai.endpoint = endpoint;
            config.ai.model = "fixture".into();
            let mut keys = AcceptanceKeys::new(config);
            unsafe {
                let original = string("天汽");
                assert!(zct_session_reconvert(keys.session, original.as_ptr()).ok);
                let secret = string("test-only");
                assert!(zct_session_ai_key(keys.session, secret.as_ptr()));
                let request = zct_session_key(keys.session, 39, 3, false, 1000);
                assert!(request.ai_pending);
                assert!(zct_session_command(keys.session, 5).ai_pending);
                if cancel {
                    keys.press(53);
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                let mut result = ZctResult::default();
                if !cancel {
                    while !zct_session_poll(keys.session, &mut result) {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    assert_eq!(text(*result.candidates).unwrap(), "天氣");
                    assert!(!result.ai_pending);
                    assert_eq!(keys.press(36).commit, "天氣");
                }
                let request = server.join().unwrap();
                assert!(request.contains("Bearer test-only"));
                if cancel {
                    assert!(!zct_session_poll(keys.session, &mut result));
                }
            }
        }
    }
    #[test]
    fn macos_mouse_selection_menu_and_session_reload_share_core_behavior() {
        let mut keys = AcceptanceKeys::new(Config::default());
        let bar = unsafe { zct_session_key(keys.session, 103, 3, false, 1) };
        assert!(bar.ok && bar.toggle_composition_bar);
        keys.type_codes(&[13, 32, 29, 49, 3, 32, 21]);
        let menu = keys.press(125);
        let selected = menu.candidates[1].clone();
        unsafe {
            let result = zct_session_choose(keys.session, 0, 1);
            assert!(result.ok && result.eaten);
            assert!(text(result.preedit).unwrap().contains(&selected));
            assert!(!zct_session_choose(keys.session, usize::MAX, 0).ok);
            let _ = keys.press(36);
            let changed = zct_session_command(keys.session, 1);
            assert!(changed.ok && changed.simplified);
            assert!(zct_session_is_idle(keys.session));
            assert!(zct_runtime_freeze(&mut *keys._runtime));
            assert!(keys._runtime.host.read_only());
            assert!(zct_runtime_recover(&mut *keys._runtime));
            assert!(!keys._runtime.host.read_only());
            // 換了引擎：沒有組字的 session 換新，沿用繁簡
            assert!(zct_session_refresh(keys.session));
            assert!(zct_session_command(keys.session, 5).simplified);
        }
    }
}
