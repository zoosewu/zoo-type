//! 工作列右下角的輸入法圖示（語言列項目，GUID_LBI_INPUTMODE）
//!
//! - 顯示目前「中 / 英」
//! - 左鍵：切換中英
//! - 右鍵：設定…、遊戲模式、重新啟動 ZCT
//!
//! 取代服務程序自製的系統匣圖示（使用者回饋：自製圖示會被收進「^」找不到）。

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, CreateSolidBrush, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_BOLD, FillRect, GetDC, HGDIOBJ, ReleaseDC,
    RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyMenu, GetForegroundWindow,
    GetSystemMetrics, HICON, ICONINFO, MF_CHECKED, MF_SEPARATOR, MF_STRING, SM_CXSMICON,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
};
use windows::core::{BSTR, GUID, IUnknown, Interface, PCWSTR, Ref, Result, implement, w};

use crate::{CLSID_ZCT, guard};

/// 圖示上的動作，交給 TextService 透過服務程序處理
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ToggleMode,
    OpenSettings,
    ToggleGame,
    Restart,
}

const ID_SETTINGS: u32 = 1;
const ID_GAME: u32 = 2;
const ID_RESTART: u32 = 3;

type Handler = Box<dyn Fn(Action)>;

#[implement(ITfLangBarItemButton, ITfSource)]
pub struct LangBarButton {
    chinese: Cell<bool>,
    game: Cell<bool>,
    sink: RefCell<Option<ITfLangBarItemSink>>,
    handler: RefCell<Option<Handler>>,
}

impl LangBarButton {
    pub fn create(handler: Handler) -> ITfLangBarItemButton {
        LangBarButton {
            chinese: Cell::new(true),
            game: Cell::new(false),
            sink: RefCell::default(),
            handler: RefCell::new(Some(handler)),
        }
        .into()
    }
}

impl LangBarButton_Impl {
    fn notify(&self) {
        let sink = self.sink.borrow().clone();
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnUpdate(TF_LBI_ICON | TF_LBI_TEXT | TF_LBI_TOOLTIP);
            }
        }
    }

    fn act(&self, a: Action) {
        // 取出後再呼叫：處理過程中可能回頭呼叫 set_state（重入）
        let handler = self.handler.borrow_mut().take();
        if let Some(h) = &handler {
            h(a);
        }
        if self.handler.borrow().is_none() {
            *self.handler.borrow_mut() = handler;
        }
    }

    fn menu(&self, pt: &POINT) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS as usize, w!("設定…"));
            let game = if self.game.get() { MF_STRING | MF_CHECKED } else { MF_STRING };
            let _ = AppendMenuW(menu, game, ID_GAME as usize, w!("遊戲模式"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, ID_RESTART as usize, w!("重新啟動 ZCT"));
            let owner: HWND = {
                let f = GetFocus();
                if f.is_invalid() { GetForegroundWindow() } else { f }
            };
            let cmd = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                None,
                owner,
                None,
            );
            let _ = DestroyMenu(menu);
            match cmd.0 as u32 {
                ID_SETTINGS => self.act(Action::OpenSettings),
                ID_GAME => self.act(Action::ToggleGame),
                ID_RESTART => self.act(Action::Restart),
                _ => {}
            }
        }
    }
}

/// 更新圖示狀態（中英、遊戲模式）；有變化才通知系統重畫
pub fn set_state(button: &ITfLangBarItemButton, chinese: bool, game: bool) {
    let Ok(obj) = button.cast_object_ref::<LangBarButton>() else {
        return;
    };
    if obj.chinese.get() != chinese || obj.game.get() != game {
        obj.chinese.set(chinese);
        obj.game.set(game);
        obj.notify();
    }
}

/// 解除與 TextService 的循環參照（Deactivate 時呼叫）
pub fn detach(button: &ITfLangBarItemButton) {
    if let Ok(obj) = button.cast_object_ref::<LangBarButton>() {
        obj.handler.borrow_mut().take();
        obj.sink.borrow_mut().take();
    }
}

impl ITfLangBarItem_Impl for LangBarButton_Impl {
    fn GetInfo(&self, pinfo: *mut TF_LANGBARITEMINFO) -> Result<()> {
        guard(|| {
            if pinfo.is_null() {
                return Err(windows::Win32::Foundation::E_INVALIDARG.into());
            }
            let mut desc = [0u16; 32];
            for (i, c) in "ZCT 注音".encode_utf16().enumerate() {
                desc[i] = c;
            }
            unsafe {
                *pinfo = TF_LANGBARITEMINFO {
                    clsidService: CLSID_ZCT,
                    guidItem: GUID_LBI_INPUTMODE,
                    dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
                    ulSort: 0,
                    szDescription: desc,
                };
            }
            Ok(())
        })
    }

    fn GetStatus(&self) -> Result<u32> {
        Ok(0)
    }

    fn Show(&self, _show: windows::core::BOOL) -> Result<()> {
        Ok(())
    }

    fn GetTooltipString(&self) -> Result<BSTR> {
        Ok(BSTR::from(if self.chinese.get() {
            "ZCT：中文（左鍵切換、右鍵選單）"
        } else {
            "ZCT：英文（左鍵切換、右鍵選單）"
        }))
    }
}

impl ITfLangBarItemButton_Impl for LangBarButton_Impl {
    fn OnClick(&self, click: TfLBIClick, pt: &POINT, _area: *const RECT) -> Result<()> {
        guard(|| {
            if click == TF_LBI_CLK_LEFT {
                self.act(Action::ToggleMode);
            } else if click == TF_LBI_CLK_RIGHT {
                self.menu(pt);
            }
            Ok(())
        })
    }

    fn InitMenu(&self, _menu: Ref<ITfMenu>) -> Result<()> {
        Ok(())
    }

    fn OnMenuSelect(&self, _id: u32) -> Result<()> {
        Ok(())
    }

    fn GetIcon(&self) -> Result<HICON> {
        guard(|| {
            if self.chinese.get() {
                make_icon("中", 0x00B06C2B) // #2B6CB0
            } else {
                make_icon("英", 0x00707070)
            }
        })
    }

    fn GetText(&self) -> Result<BSTR> {
        Ok(BSTR::from(if self.chinese.get() { "中" } else { "英" }))
    }
}

impl ITfSource_Impl for LangBarButton_Impl {
    fn AdviseSink(&self, riid: *const GUID, punk: Ref<IUnknown>) -> Result<u32> {
        guard(|| unsafe {
            if riid.is_null() || *riid != ITfLangBarItemSink::IID {
                return Err(windows::Win32::System::Ole::CONNECT_E_CANNOTCONNECT.into());
            }
            let sink: ITfLangBarItemSink = punk.ok()?.cast()?;
            *self.sink.borrow_mut() = Some(sink);
            Ok(1)
        })
    }

    fn UnadviseSink(&self, _cookie: u32) -> Result<()> {
        self.sink.borrow_mut().take();
        Ok(())
    }
}

/// 圓角色塊加一個字的圖示；透明角落用 alpha 處理
fn make_icon(text: &str, bg: u32) -> Result<HICON> {
    const KEY: u32 = 0x00FF00FF;
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
        let fill = CreateSolidBrush(COLORREF(bg));
        let old_brush = SelectObject(dc, HGDIOBJ(fill.0));
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
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut trc = rc;
        DrawTextW(dc, &mut wide, &mut trc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old_font);
        SelectObject(dc, old);
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        for p in px.iter_mut() {
            *p = if *p & 0x00FFFFFF == KEY { 0 } else { *p | 0xFF00_0000 };
        }
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        });
        for obj in [
            HGDIOBJ(mask.0),
            HGDIOBJ(color.0),
            HGDIOBJ(key.0),
            HGDIOBJ(fill.0),
            HGDIOBJ(font.0),
        ] {
            let _ = DeleteObject(obj);
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        icon
    }
}
