//! 候選窗、組字提示列與「中 / A」提示窗（Direct2D / DirectWrite）
//!
//! - 排版與彈出位置在共用的 `zct_core::popup`（與 macOS 相同）；這裡只量文字、照結果畫
//! - 視窗常駐、預先建立，打字時只更新內容與位置
//! - WS_EX_NOACTIVATE：永不搶走 App 的焦點
//! - 所有視窗操作都在 UI 執行緒；連線執行緒透過 channel + PostMessage 送指令

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_UNKNOWN, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_METRICS, DWriteCreateFactory, IDWriteFactory,
    IDWriteTextFormat, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, ValidateRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_TOPMOST,
    IDC_ARROW, KillTimer, LoadCursorW, MA_NOACTIVATE, MSG, PostMessageW, RegisterClassExW, SW_HIDE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, WM_APP,
    WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HSTRING, Result, w};
use windows_numerics::Vector2;
use zct_core::config::{Color, Layout, ThemeConfig};
use zct_core::popup::{self, Font, Ink};
use zct_core::{CandidatePage, Preedit};
use zct_ipc::Rect;

const WM_WAKE: u32 = WM_APP + 1;
const HINT_TIMER: usize = 1;

fn color(c: Color) -> D2D1_COLOR_F {
    let (r, g, b) = c.rgb();
    D2D1_COLOR_F {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

/// 主題轉成 Direct2D 用的顏色
#[derive(Clone, Copy)]
struct Colors {
    bg: D2D1_COLOR_F,
    fg: D2D1_COLOR_F,
    dim: D2D1_COLOR_F,
    hl: D2D1_COLOR_F,
}

impl Colors {
    fn from(t: &ThemeConfig) -> Colors {
        Colors {
            bg: color(t.background),
            fg: color(t.text),
            dim: color(t.number),
            hl: color(t.highlight),
        }
    }
}

pub enum UiCmd {
    /// `owner`：連線 id，避免舊連線的 Hide 蓋掉新連線的候選窗
    Show {
        owner: u64,
        page: CandidatePage,
        anchor: Option<Rect>,
    },
    /// 隱藏候選窗與組字提示列（失焦、重設、連線結束）
    Hide { owner: u64 },
    /// 只隱藏候選窗（每次按鍵後沒有候選時；組字提示列由 Composition 控制）
    HideCandidates { owner: u64 },
    /// 狀態提示：中 / A / 全形 / 半形
    Hint { text: String, anchor: Option<Rect> },
    /// 設定檔的主題變更了
    Theme(ThemeConfig),
    /// 組字提示列；None 表示隱藏
    Composition {
        owner: u64,
        preedit: Option<Preedit>,
        anchor: Option<Rect>,
    },
    /// 結束服務程序
    Exit,
}

#[derive(Clone)]
pub struct UiHandle {
    tx: Sender<UiCmd>,
    /// HWND 不是 Send，以整數保存（只用來 PostMessage 喚醒）
    wake: isize,
}

impl UiHandle {
    pub fn send(&self, cmd: UiCmd) {
        if self.tx.send(cmd).is_ok() {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.wake as *mut _)),
                    WM_WAKE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }
}

pub fn spawn(theme: ThemeConfig) -> std::result::Result<UiHandle, Box<dyn std::error::Error>> {
    let (tx, rx) = channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("ui".into())
        .spawn(move || match Ui::new(rx, theme) {
            Ok(ui) => {
                let wake = ui.cand.hwnd.0 as isize;
                UI.with(|cell| *cell.borrow_mut() = Some(ui));
                let _ = ready_tx.send(Ok(wake));
                message_loop();
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
            }
        })?;
    let wake = ready_rx.recv()??;
    Ok(UiHandle { tx, wake })
}

fn message_loop() {
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

struct Popup {
    hwnd: HWND,
    target: Option<ID2D1HwndRenderTarget>,
    /// 共用排版的結果（DIP）
    content: popup::Popup,
    /// 實體像素
    size: (u32, u32),
    scale: f32,
}

/// DirectWrite 的文字排版：量過的文字留著，畫的時候直接用
struct Texts {
    dw: IDWriteFactory,
    text_fmt: IDWriteTextFormat,
    num_fmt: IDWriteTextFormat,
    cache: HashMap<(String, Font), (IDWriteTextLayout, f32, f32)>,
}

impl Texts {
    fn get(&mut self, text: &str, font: Font) -> Option<&(IDWriteTextLayout, f32, f32)> {
        let key = (text.to_owned(), font);
        if !self.cache.contains_key(&key) {
            // 不同的字很多：滿了就整個清掉（只是要重新排版）
            if self.cache.len() > 1024 {
                self.cache.clear();
            }
            let fmt = match font {
                Font::Text => &self.text_fmt,
                Font::Number => &self.num_fmt,
            };
            let made = unsafe {
                let wide: Vec<u16> = text.encode_utf16().collect();
                self.dw
                    .CreateTextLayout(&wide, fmt, 10_000.0, 1_000.0)
                    .and_then(|layout| {
                        let mut m = DWRITE_TEXT_METRICS::default();
                        layout.GetMetrics(&mut m)?;
                        Ok((layout, m.widthIncludingTrailingWhitespace, m.height))
                    })
            };
            match made {
                Ok(v) => {
                    self.cache.insert(key.clone(), v);
                }
                Err(e) => {
                    log::warn!("文字排版失敗：{e}");
                    return None;
                }
            }
        }
        self.cache.get(&key)
    }
}

impl popup::Measure for Texts {
    fn measure(&mut self, text: &str, font: Font) -> (f32, f32) {
        self.get(text, font)
            .map_or((0.0, 0.0), |(_, w, h)| (*w, *h))
    }
}

struct Ui {
    d2d: ID2D1Factory,
    theme: ThemeConfig,
    colors: Colors,
    texts: Texts,
    cand: Popup,
    hint: Popup,
    comp: Popup,
    rx: Receiver<UiCmd>,
    owner: Option<u64>,
    comp_owner: Option<u64>,
    last_anchor: Option<Rect>,
}

impl Ui {
    fn formats(
        dw: &IDWriteFactory,
        t: &ThemeConfig,
    ) -> Result<(IDWriteTextFormat, IDWriteTextFormat)> {
        let fmt = |size: f32| unsafe {
            dw.CreateTextFormat(
                &HSTRING::from(t.font.as_str()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("zh-TW"),
            )
        };
        Ok((fmt(t.font_size)?, fmt(t.font_size * popup::NUM_RATIO)?))
    }

    fn set_theme(&mut self, theme: ThemeConfig) -> Result<()> {
        let (text_fmt, num_fmt) = Self::formats(&self.texts.dw, &theme)?;
        self.texts.text_fmt = text_fmt;
        self.texts.num_fmt = num_fmt;
        self.texts.cache.clear();
        self.colors = Colors::from(&theme);
        self.theme = theme;
        Ok(())
    }

    fn new(rx: Receiver<UiCmd>, theme: ThemeConfig) -> Result<Ui> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let (text_fmt, num_fmt) = Self::formats(&dw, &theme)?;

            let hinst = GetModuleHandleW(None)?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DROPSHADOW,
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: w!("ZctPopup"),
                ..Default::default()
            };
            RegisterClassExW(&class);
            let make = || -> Result<Popup> {
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    w!("ZctPopup"),
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    Some(hinst.into()),
                    None,
                )?;
                let corner = DWMWCP_ROUND;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &corner as *const _ as *const _,
                    size_of_val(&corner) as u32,
                );
                Ok(Popup {
                    hwnd,
                    target: None,
                    content: popup::Popup::default(),
                    size: (1, 1),
                    scale: 1.0,
                })
            };
            Ok(Ui {
                d2d,
                colors: Colors::from(&theme),
                theme,
                texts: Texts {
                    dw,
                    text_fmt,
                    num_fmt,
                    cache: HashMap::new(),
                },
                cand: make()?,
                hint: make()?,
                comp: make()?,
                rx,
                owner: None,
                comp_owner: None,
                last_anchor: None,
            })
        }
    }

    fn drain(&mut self) {
        while let Ok(cmd) = self.rx.try_recv() {
            let r = match cmd {
                UiCmd::Show {
                    owner,
                    page,
                    anchor,
                } => {
                    self.owner = Some(owner);
                    self.show_candidates(&page, anchor)
                }
                UiCmd::Hide { owner } => {
                    self.hide_candidates(owner);
                    self.hide_composition(owner);
                    Ok(())
                }
                UiCmd::HideCandidates { owner } => {
                    self.hide_candidates(owner);
                    Ok(())
                }
                UiCmd::Composition {
                    owner,
                    preedit,
                    anchor,
                } => match preedit.filter(|p| !p.text.is_empty()) {
                    Some(p) => {
                        self.comp_owner = Some(owner);
                        self.show_composition(&p, anchor)
                    }
                    None => {
                        self.hide_composition(owner);
                        Ok(())
                    }
                },
                UiCmd::Hint { text, anchor } => self.show_hint(&text, anchor),
                UiCmd::Exit => std::process::exit(0),
                UiCmd::Theme(t) => {
                    log::info!("套用候選窗主題");
                    self.set_theme(t)
                }
            };
            if let Err(e) = r {
                log::warn!("繪製失敗：{e}");
            }
        }
    }

    /// 取不到游標位置時沿用上一次的位置（P4 的最後一道 fallback）
    fn anchor(&mut self, anchor: Option<Rect>) -> Option<Rect> {
        let valid = anchor.filter(|r| r.right > r.left || r.bottom > r.top || r.left != 0);
        if valid.is_some() {
            self.last_anchor = valid;
        }
        self.last_anchor
    }

    fn show_candidates(&mut self, page: &CandidatePage, anchor: Option<Rect>) -> Result<()> {
        let Some(anchor) = self.anchor(anchor) else {
            return Ok(());
        };
        let horizontal = self.theme.layout == Layout::Horizontal;
        let content = popup::candidates(page, horizontal, &mut self.texts);
        let (d2d, colors, texts) = (&self.d2d, self.colors, &mut self.texts);
        show(
            d2d,
            &colors,
            texts,
            &mut self.cand,
            content,
            anchor,
            popup::PopupKind::Candidates.above(),
        )
    }

    /// 只隱藏自己顯示的視窗，避免舊連線蓋掉新連線的畫面
    fn hide_candidates(&mut self, owner: u64) {
        if self.owner == Some(owner) {
            self.owner = None;
            unsafe {
                let _ = ShowWindow(self.cand.hwnd, SW_HIDE);
            }
        }
    }

    fn hide_composition(&mut self, owner: u64) {
        if self.comp_owner == Some(owner) {
            self.comp_owner = None;
            unsafe {
                let _ = ShowWindow(self.comp.hwnd, SW_HIDE);
            }
        }
    }

    /// 組字提示列：「組字內容（選字目標反白）　N 字」，放在游標上方
    fn show_composition(&mut self, p: &Preedit, anchor: Option<Rect>) -> Result<()> {
        let Some(anchor) = self.anchor(anchor) else {
            return Ok(());
        };
        let content = popup::composition(p, &mut self.texts);
        let (d2d, colors, texts) = (&self.d2d, self.colors, &mut self.texts);
        show(
            d2d,
            &colors,
            texts,
            &mut self.comp,
            content,
            anchor,
            popup::PopupKind::Bar.above(),
        )
    }

    fn show_hint(&mut self, text: &str, anchor: Option<Rect>) -> Result<()> {
        let Some(anchor) = self.anchor(anchor) else {
            return Ok(());
        };
        let content = popup::hint(text, &mut self.texts);
        let (d2d, colors, texts) = (&self.d2d, self.colors, &mut self.texts);
        show(
            d2d,
            &colors,
            texts,
            &mut self.hint,
            content,
            anchor,
            popup::PopupKind::Hint.above(),
        )?;
        unsafe {
            SetTimer(
                Some(self.hint.hwnd),
                HINT_TIMER,
                popup::HINT_MS as u32,
                None,
            );
        }
        Ok(())
    }
}

/// 換上排好的內容、放到游標旁（位置規則見 `popup::place`）並畫出來
fn show(
    d2d: &ID2D1Factory,
    colors: &Colors,
    texts: &mut Texts,
    p: &mut Popup,
    content: popup::Popup,
    anchor: Rect,
    above: bool,
) -> Result<()> {
    let size = (content.width, content.height);
    p.content = content;
    place(p, anchor, size, above)?;
    render(d2d, colors, texts, p)
}

/// `above`：優先放在游標上方（組字提示列，避免與下方的候選窗重疊）
fn place(p: &mut Popup, anchor: Rect, dip: (f32, f32), above: bool) -> Result<()> {
    unsafe {
        let rc = RECT {
            left: anchor.left,
            top: anchor.top,
            right: anchor.right.max(anchor.left + 1),
            bottom: anchor.bottom.max(anchor.top + 1),
        };
        let mon = MonitorFromRect(&rc, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f32 / 96.0;
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut info);
        let work = info.rcWork;

        let w = (dip.0 * scale).ceil() as i32;
        let h = (dip.1 * scale).ceil() as i32;
        let rect = |l: i32, t: i32, r: i32, b: i32| popup::Rect {
            x: l as f32,
            y: t as f32,
            w: (r - l) as f32,
            h: (b - t) as f32,
        };
        let (x, y) = popup::place(
            rect(rc.left, rc.top, rc.right, rc.bottom),
            (w as f32, h as f32),
            rect(work.left, work.top, work.right, work.bottom),
            (popup::OFFSET * scale).floor(),
            above,
        );

        if (p.scale - scale).abs() > f32::EPSILON {
            p.scale = scale;
            if let Some(t) = &p.target {
                t.SetDpi(96.0 * scale, 96.0 * scale);
            }
        }
        let size = (w as u32, h as u32);
        if size != p.size {
            p.size = size;
            if let Some(t) = &p.target {
                t.Resize(&D2D_SIZE_U {
                    width: size.0,
                    height: size.1,
                })?;
            }
        }
        SetWindowPos(
            p.hwnd,
            Some(HWND_TOPMOST),
            x as i32,
            y as i32,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )?;
        Ok(())
    }
}

fn d2d_rect(r: &popup::Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x,
        top: r.y,
        right: r.right(),
        bottom: r.bottom(),
    }
}

fn render(d2d: &ID2D1Factory, c: &Colors, texts: &mut Texts, p: &mut Popup) -> Result<()> {
    unsafe {
        if p.target.is_none() {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_UNKNOWN,
                    alphaMode: D2D1_ALPHA_MODE_UNKNOWN,
                },
                dpiX: 96.0 * p.scale,
                dpiY: 96.0 * p.scale,
                ..Default::default()
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: p.hwnd,
                pixelSize: D2D_SIZE_U {
                    width: p.size.0,
                    height: p.size.1,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            p.target = Some(d2d.CreateHwndRenderTarget(&props, &hwnd_props)?);
        }
        let Some(t) = &p.target else { return Ok(()) };
        let fg = t.CreateSolidColorBrush(&c.fg, None)?;
        let dim = t.CreateSolidColorBrush(&c.dim, None)?;
        t.BeginDraw();
        t.Clear(Some(&c.bg));
        if !p.content.highlights.is_empty() {
            let hl = t.CreateSolidColorBrush(&c.hl, None)?;
            for r in &p.content.highlights {
                t.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: d2d_rect(r),
                        radiusX: popup::ROW_RADIUS,
                        radiusY: popup::ROW_RADIUS,
                    },
                    &hl,
                );
            }
        }
        if let Some(r) = &p.content.caret {
            t.FillRectangle(&d2d_rect(r), &fg);
        }
        for run in &p.content.runs {
            let brush = match run.ink {
                Ink::Text => &fg,
                Ink::Dim => &dim,
            };
            if let Some((layout, ..)) = texts.get(&run.text, run.font) {
                t.DrawTextLayout(
                    Vector2 { X: run.x, Y: run.y },
                    layout,
                    brush,
                    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                );
            }
        }
        if let Err(e) = t.EndDraw(None, None) {
            // 顯示卡重置等情況：下次重建 render target
            p.target = None;
            return Err(e);
        }
        let _ = ValidateRect(Some(p.hwnd), None);
        Ok(())
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // 不讓 panic 跨過系統回呼邊界
    let handled = std::panic::catch_unwind(|| handle(hwnd, msg, wparam)).unwrap_or(None);
    match handled {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn handle(hwnd: HWND, msg: u32, wparam: WPARAM) -> Option<LRESULT> {
    match msg {
        WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
        WM_ERASEBKGND => Some(LRESULT(1)),
        WM_WAKE => {
            with_ui(|ui| ui.drain());
            Some(LRESULT(0))
        }
        WM_TIMER if wparam.0 == HINT_TIMER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), HINT_TIMER);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            Some(LRESULT(0))
        }
        WM_PAINT => {
            let painted = with_ui(|ui| {
                let (d2d, colors, texts) = (&ui.d2d, ui.colors, &mut ui.texts);
                let p = if ui.cand.hwnd == hwnd {
                    &mut ui.cand
                } else if ui.comp.hwnd == hwnd {
                    &mut ui.comp
                } else {
                    &mut ui.hint
                };
                let _ = render(d2d, &colors, texts, p);
            });
            painted.map(|_| LRESULT(0))
        }
        _ => None,
    }
}

/// 重入時（例如 SetWindowPos 同步送來的訊息）直接略過，交給預設處理
fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(f),
        Err(_) => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_and_candidate_popups_paint_the_shared_selection() -> Result<()> {
        let (_, rx) = channel();
        let mut ui = Ui::new(rx, ThemeConfig::default())?;
        let anchor = Some(Rect {
            left: 10,
            top: 10,
            right: 11,
            bottom: 28,
        });
        let mut preedit = Preedit {
            text: "天😀氣".into(),
            cursor: 0,
            target: Some((0, 1)),
            len: 3,
        };
        ui.show_composition(&preedit, anchor)?;
        let first = ui.comp.content.caret.unwrap().x;
        preedit.cursor = 1;
        preedit.target = Some((1, 1));
        ui.show_composition(&preedit, anchor)?;
        let caret = ui.comp.content.caret.unwrap().x;
        let target = ui.comp.content.highlights[0];
        assert!(caret > first && target.x <= caret && target.right() > caret);
        let mut page = CandidatePage {
            items: vec!["氣".into(), "汽".into()],
            total_pages: 1,
            ..Default::default()
        };
        ui.show_candidates(&page, anchor)?;
        let first = ui.cand.content.highlights[0].y;
        page.highlight = 1;
        ui.show_candidates(&page, anchor)?;
        assert!(ui.cand.content.highlights[0].y > first);
        Ok(())
    }
}
