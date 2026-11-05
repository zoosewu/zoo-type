//! 偵測「不在輸入狀態」：此時按鍵全部交給 App，也不通知服務程序（Shift 不會切換中英）
//!
//! 三層，任一層成立即視為不在輸入狀態：
//! 1. 系統訊號（一律啟用，與微軟 SampleIME 的 `_IsKeyboardDisabled` 相同）：
//!    沒有焦點文件、沒有 context、`GUID_COMPARTMENT_KEYBOARD_DISABLED` 或 `GUID_COMPARTMENT_EMPTYCONTEXT`
//!    （SampleIME 後一個判斷會覆蓋前一個，是範例的 bug；這裡兩者取 OR）
//! 2. 舊式程式（transitory context，即不支援 TSF、由系統相容層代管）+ 沒有系統游標 + 視窗蓋滿整個螢幕：
//!    無邊框全螢幕的 DirectX 遊戲。必須同時蓋滿螢幕，否則 IntelliJ 等 Java / Qt 程式（同樣是舊式介面、
//!    沒有系統游標）會被誤判而無法打中文；瀏覽器 F11 全螢幕則因支援 TSF 不受影響
//! 3. 有程式獨佔全螢幕

use std::cell::Cell;
use std::time::{Duration, Instant};

use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::VT_I4;
use windows::Win32::UI::Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::Win32::UI::TextServices::{
    GUID_COMPARTMENT_EMPTYCONTEXT, GUID_COMPARTMENT_KEYBOARD_DISABLED, ITfCompartmentMgr,
    ITfContext, ITfThreadMgr, TS_SS_TRANSITORY,
};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowRect,
};
use windows::core::{GUID, Interface};

/// 服務程序依設定與 App 規則決定要不要做的啟發式偵測
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Heuristics {
    pub legacy_fullscreen: bool,
    pub exclusive_fullscreen: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Signals {
    pub has_focus_doc: bool,
    pub has_context: bool,
    pub keyboard_disabled: bool,
    pub empty_context: bool,
    pub transitory: bool,
    pub has_caret: bool,
    pub covers_monitor: bool,
    pub exclusive_fullscreen: bool,
}

/// 依訊號判斷是否不在輸入狀態
pub fn not_input(s: &Signals, h: Heuristics) -> bool {
    if !s.has_focus_doc || !s.has_context || s.keyboard_disabled || s.empty_context {
        return true;
    }
    if h.legacy_fullscreen && s.transitory && !s.has_caret && s.covers_monitor {
        return true;
    }
    h.exclusive_fullscreen && s.exclusive_fullscreen
}

fn compartment_on(mgr: &ITfCompartmentMgr, guid: &GUID) -> bool {
    unsafe {
        let Ok(c) = mgr.GetCompartment(guid) else {
            return false;
        };
        let Ok(v) = c.GetValue() else { return false };
        let inner = &v.Anonymous.Anonymous;
        inner.vt == VT_I4 && inner.Anonymous.lVal != 0
    }
}

/// compartment 可能設在 thread manager（SampleIME 讀這裡）或 context 上，兩邊都看
fn compartments(tm: &ITfThreadMgr, ctx: &ITfContext) -> (bool, bool) {
    let mut disabled = false;
    let mut empty = false;
    let mgrs = [tm.cast::<ITfCompartmentMgr>().ok(), ctx.cast::<ITfCompartmentMgr>().ok()];
    for mgr in mgrs.iter().flatten() {
        disabled |= compartment_on(mgr, &GUID_COMPARTMENT_KEYBOARD_DISABLED);
        empty |= compartment_on(mgr, &GUID_COMPARTMENT_EMPTYCONTEXT);
    }
    (disabled, empty)
}

/// 前景視窗是否蓋滿它所在的整個螢幕（含工作列區域）
fn covers_monitor() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut rc = RECT::default();
        if hwnd.is_invalid() || GetWindowRect(hwnd, &mut rc).is_err() {
            return false;
        }
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(mon, &mut info).as_bool()
            && rc.left <= info.rcMonitor.left
            && rc.top <= info.rcMonitor.top
            && rc.right >= info.rcMonitor.right
            && rc.bottom >= info.rcMonitor.bottom
    }
}

fn has_caret() -> bool {
    unsafe {
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(GetCurrentThreadId(), &mut info).is_ok() && !info.hwndCaret.is_invalid()
    }
}

/// 獨佔全螢幕查詢要經過 Shell，結果快取一小段時間
#[derive(Default)]
pub struct FullscreenCache {
    checked_at: Cell<Option<Instant>>,
    value: Cell<bool>,
}

impl FullscreenCache {
    const TTL: Duration = Duration::from_millis(500);

    fn get(&self) -> bool {
        let fresh = self
            .checked_at
            .get()
            .is_some_and(|t| t.elapsed() < Self::TTL);
        if !fresh {
            let v = unsafe { SHQueryUserNotificationState() }
                .is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN);
            self.value.set(v);
            self.checked_at.set(Some(Instant::now()));
        }
        self.value.get()
    }
}

/// 收集目前執行緒的訊號；只查詢啟用的啟發式需要的部分
pub fn gather(tm: &ITfThreadMgr, h: Heuristics, fullscreen: &FullscreenCache) -> Signals {
    let mut s = Signals::default();
    unsafe {
        let Ok(doc) = tm.GetFocus() else { return s };
        s.has_focus_doc = true;
        let Ok(ctx) = doc.GetTop() else { return s };
        s.has_context = true;
        (s.keyboard_disabled, s.empty_context) = compartments(tm, &ctx);
        if h.legacy_fullscreen {
            s.transitory = ctx
                .GetStatus()
                .is_ok_and(|st| st.dwStaticFlags & TS_SS_TRANSITORY != 0);
            if s.transitory {
                s.has_caret = has_caret();
                s.covers_monitor = !s.has_caret && covers_monitor();
            }
        }
    }
    if h.exclusive_fullscreen {
        s.exclusive_fullscreen = fullscreen.get();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Heuristics = Heuristics {
        legacy_fullscreen: true,
        exclusive_fullscreen: true,
    };
    const NONE: Heuristics = Heuristics {
        legacy_fullscreen: false,
        exclusive_fullscreen: false,
    };

    fn normal() -> Signals {
        Signals {
            has_focus_doc: true,
            has_context: true,
            ..Default::default()
        }
    }

    #[test]
    fn normal_text_field_is_input() {
        assert!(!not_input(&normal(), ALL));
    }

    #[test]
    fn system_signals_always_apply() {
        for s in [
            Signals::default(),
            Signals { has_context: false, ..normal() },
            Signals { keyboard_disabled: true, ..normal() },
            Signals { empty_context: true, ..normal() },
        ] {
            assert!(not_input(&s, NONE), "{s:?}");
        }
    }

    #[test]
    fn fullscreen_legacy_app_without_caret_is_not_input() {
        let game = Signals { transitory: true, covers_monitor: true, ..normal() };
        assert!(not_input(&game, ALL));
        assert!(!not_input(&game, NONE), "可以用設定關掉");
        let legacy_edit = Signals { transitory: true, has_caret: true, covers_monitor: true, ..normal() };
        assert!(!not_input(&legacy_edit, ALL), "舊式程式的輸入框有游標，照常輸入");
    }

    #[test]
    fn windowed_legacy_app_without_caret_is_still_input() {
        // IntelliJ 等 Java / Qt 程式：舊式介面、沒有系統游標，但不是全螢幕
        let ide = Signals { transitory: true, ..normal() };
        assert!(!not_input(&ide, ALL));
    }

    #[test]
    fn exclusive_fullscreen_is_not_input() {
        let s = Signals { exclusive_fullscreen: true, ..normal() };
        assert!(not_input(&s, ALL));
        assert!(!not_input(&s, NONE));
    }
}
