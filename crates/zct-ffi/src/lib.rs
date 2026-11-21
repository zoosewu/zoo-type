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
    update_file: CString,
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
            update_file: CString::default(),
        }
    }

    fn store(&self) -> zct_preferences::SettingsStore {
        zct_preferences::SettingsStore::new(self.host.data())
    }
}

pub struct ZctSession {
    inner: HostSession,
    snapshot: Snapshot,
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
    out.available = runtime.update_available.as_ptr();
    out.message = runtime.update_message.as_ptr();
    out
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

/// 要安裝的檔案（重新驗證過簽章與版號）；沒有可安裝的版本是 NULL 並設定錯誤
/// # Safety
/// `runtime` is live; the path is borrowed until the next update call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zct_update_pending(runtime: *mut ZctRuntime) -> *const c_char {
    guard(|| {
        let runtime = unsafe { runtime.as_mut() }.ok_or("空 runtime")?;
        let updater = runtime.updater.as_ref().ok_or("只有正式版會更新")?;
        let (_, file) = updater.pending_file().ok_or("沒有驗證過的更新檔")?;
        runtime.update_file = string(&file.to_string_lossy());
        Ok(runtime.update_file.as_ptr())
    })
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
        let mut punctuation = AcceptanceKeys::new(config.clone());
        assert_eq!(
            punctuation
                .type_codes(&[37, 31, 43, 47, 44, 41, 27, 20, 21, 22, 26, 49])
                .preedit,
            "lo,./;-3467 "
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
