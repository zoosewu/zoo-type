//! 收集「不在輸入狀態」的訊號（判斷規則在 `zct_core::detect`，與 macOS 共用）
//!
//! - 系統訊號（與微軟 SampleIME 的 `_IsKeyboardDisabled` 相同）：沒有焦點文件、沒有 context、
//!   `GUID_COMPARTMENT_KEYBOARD_DISABLED` 或 `GUID_COMPARTMENT_EMPTYCONTEXT`
//!   （SampleIME 後一個判斷會覆蓋前一個，是範例的 bug；這裡兩者取 OR）
//! - 舊式程式（transitory context，即不支援 TSF、由系統相容層代管）看不出是文字輸入框；
//!   瀏覽器 F11 全螢幕支援 TSF，不受影響
//! - 有程式獨佔全螢幕：`SHQueryUserNotificationState`

use std::cell::Cell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::VT_I4;
use windows::Win32::UI::Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::Win32::UI::TextServices::{
    GUID_COMPARTMENT_EMPTYCONTEXT, GUID_COMPARTMENT_KEYBOARD_DISABLED, ITfCompartmentMgr,
    ITfContext, ITfThreadMgr, TS_SS_TRANSITORY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowRect,
};
use windows::core::{GUID, Interface};

pub use zct_core::detect::{Heuristics, Signals, not_input};

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
    let mgrs = [
        tm.cast::<ITfCompartmentMgr>().ok(),
        ctx.cast::<ITfCompartmentMgr>().ok(),
    ];
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
    let mut s = Signals {
        blocked: true,
        ..Default::default()
    };
    unsafe {
        let Ok(doc) = tm.GetFocus() else { return s };
        let Ok(ctx) = doc.GetTop() else { return s };
        let (disabled, empty) = compartments(tm, &ctx);
        s.blocked = disabled || empty;
        if h.fullscreen_without_caret {
            let transitory = ctx
                .GetStatus()
                .is_ok_and(|st| st.dwStaticFlags & TS_SS_TRANSITORY != 0);
            s.text_context = !transitory;
            if transitory {
                s.has_caret = has_caret();
                s.covers_screen = !s.has_caret && covers_monitor();
            }
        }
    }
    if h.exclusive_fullscreen {
        s.exclusive_fullscreen = fullscreen.get();
    }
    s
}
