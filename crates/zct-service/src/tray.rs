//! 系統匣圖示：設定、遊戲模式、重新啟動
//!
//! 必須在 UI 執行緒建立（需要訊息迴圈）。

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, CreateSolidBrush, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_BOLD, FillRect, GetDC, HGDIOBJ, ReleaseDC,
    RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, GetCursorPos, GetSystemMetrics, HICON, HWND_MESSAGE, ICONINFO, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassExW, RegisterWindowMessageW,
    SM_CXSMICON, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSEXW,
};
use windows::core::{PCWSTR, Result, w};
use zct_core::Engine;

const WM_TRAY: u32 = WM_APP + 2;
const ID_SETTINGS: usize = 1;
const ID_GAME: usize = 2;
const ID_RESTART: usize = 3;

/// 選單動作需要的共用狀態
pub struct TrayCtx {
    pub game: Arc<AtomicBool>,
    pub engine: Arc<Engine>,
}

struct Tray {
    hwnd: HWND,
    icon: HICON,
    /// Explorer 重新啟動時廣播的訊息，收到要重新加入圖示
    taskbar_created: u32,
    ctx: TrayCtx,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

fn wide_fixed<const N: usize>(s: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        out[i] = c;
    }
    out
}

fn data(t: &Tray) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: t.hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: t.icon,
        szTip: wide_fixed("ZCT 注音"),
        ..Default::default()
    }
}

/// 畫一個藍底白字「注」的圓角圖示；透明角落用 alpha 處理
fn make_icon() -> Result<HICON> {
    const KEY: u32 = 0x00FF00FF; // 洋紅色當透明色
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, HGDIOBJ(color.0));
        let rc = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        let key = CreateSolidBrush(COLORREF(KEY));
        FillRect(dc, &rc, key);
        let blue = CreateSolidBrush(COLORREF(0x00B06C2B)); // #2B6CB0（GDI 是 BGR）
        let old_brush = SelectObject(dc, HGDIOBJ(blue.0));
        let r = size / 4;
        let _ = RoundRect(dc, 0, 0, size, size, r, r);
        SelectObject(dc, old_brush);
        let font = CreateFontW(
            -(size * 13 / 16),
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            CLEARTYPE_QUALITY,
            0,
            w!("Microsoft JhengHei UI"),
        );
        let old_font = SelectObject(dc, HGDIOBJ(font.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FFFFFF));
        let mut text: Vec<u16> = "注".encode_utf16().collect();
        let mut trc = rc;
        DrawTextW(dc, &mut text, &mut trc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old_font);
        SelectObject(dc, old);

        // GDI 不寫 alpha：洋紅色 → 全透明，其他 → 不透明
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        for p in px.iter_mut() {
            *p = if *p & 0x00FFFFFF == KEY {
                0
            } else {
                *p | 0xFF00_0000
            };
        }

        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        });
        for obj in [HGDIOBJ(mask.0), HGDIOBJ(color.0), HGDIOBJ(key.0), HGDIOBJ(blue.0), HGDIOBJ(font.0)] {
            let _ = DeleteObject(obj);
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        icon
    }
}

pub fn create(ctx: TrayCtx) -> Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(tray_proc),
            hInstance: hinst.into(),
            lpszClassName: w!("ZctTray"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let hwnd = CreateWindowExW(
            Default::default(),
            w!("ZctTray"),
            w!(""),
            Default::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )?;
        let tray = Tray {
            hwnd,
            icon: make_icon()?,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            ctx,
        };
        let _ = Shell_NotifyIconW(NIM_ADD, &data(&tray));
        TRAY.with(|t| *t.borrow_mut() = Some(tray));
    }
    Ok(())
}

/// 程序結束前移除圖示，避免系統匣留下殘影
pub fn remove() {
    TRAY.with(|t| {
        if let Some(tray) = t.borrow().as_ref() {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data(tray));
            }
        }
    });
}

fn open_settings() {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("zct-settings.exe")));
    match exe {
        Some(exe) if exe.exists() => {
            if let Err(e) = std::process::Command::new(&exe).spawn() {
                log::warn!("無法開啟設定程式：{e}");
            }
        }
        _ => log::warn!("找不到設定程式 zct-settings.exe"),
    }
}

fn show_menu(t: &Tray) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let game = t.ctx.game.load(Ordering::Relaxed);
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("設定…"));
        let flags = if game { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, flags, ID_GAME, w!("遊戲模式"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_RESTART, w!("重新啟動 ZCT"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // 沒有先設前景，點選單外面時選單不會關閉（已知的 Win32 行為）
        let _ = SetForegroundWindow(t.hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            None,
            t.hwnd,
            None,
        );
        let _ = PostMessageW(Some(t.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match cmd.0 as usize {
            ID_SETTINGS => open_settings(),
            ID_GAME => {
                let on = !t.ctx.game.fetch_xor(true, Ordering::Relaxed);
                log::info!("系統匣：遊戲模式{}", if on { "開" } else { "關" });
            }
            ID_RESTART => {
                log::info!("系統匣：重新啟動");
                let _ = t.ctx.engine.save();
                let _ = Shell_NotifyIconW(NIM_DELETE, &data(t));
                // DLL 會在下次按鍵時重新啟動服務程序
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

extern "system" fn tray_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = std::panic::catch_unwind(|| {
        TRAY.with(|t| {
            let guard = t.try_borrow().ok()?;
            let tray = guard.as_ref()?;
            if msg == WM_TRAY {
                let event = (lparam.0 & 0xFFFF) as u32;
                if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                    show_menu(tray);
                }
                return Some(LRESULT(0));
            }
            if msg == tray.taskbar_created {
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_ADD, &data(tray));
                }
                return Some(LRESULT(0));
            }
            None
        })
    })
    .unwrap_or(None);
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
}
