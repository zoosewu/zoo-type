//! 候選窗與「中 / A」提示窗（Direct2D / DirectWrite）
//!
//! - 視窗常駐、預先建立，打字時只更新內容與位置
//! - WS_EX_NOACTIVATE：永不搶走 App 的焦點
//! - 所有視窗操作都在 UI 執行緒；連線執行緒透過 channel + PostMessage 送指令

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_UNKNOWN, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget,
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
    KillTimer, LoadCursorW, IDC_ARROW, MA_NOACTIVATE, MSG, PostMessageW, RegisterClassExW,
    SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer, SetWindowPos, ShowWindow,
    TranslateMessage, WM_APP, WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_PAINT, WM_TIMER, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HSTRING, Result, w};
use windows_numerics::Vector2;
use zct_core::{CandidatePage, Preedit};
use zct_core::config::{Color, Layout, ThemeConfig};
use zct_ipc::Rect;

const WM_WAKE: u32 = WM_APP + 1;
const HINT_TIMER: usize = 1;
const HINT_MS: u32 = 800;

/// 數字字級相對於候選字字級
const NUM_RATIO: f32 = 0.75;
const PAD_X: f32 = 6.0;
const PAD_Y: f32 = 4.0;
/// 選擇框左右內距
const ROW_PAD_X: f32 = 6.0;
/// 每列上下內距
const ROW_PAD_Y: f32 = 2.0;
/// 數字與候選字之間的距離
const GAP_NUM: f32 = 8.0;
/// 選擇框圓角
const ROW_RADIUS: f32 = 4.0;
/// 候選窗與游標之間的距離
const OFFSET_DIP: f32 = 4.0;

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
    Hide {
        owner: u64,
    },
    /// 只隱藏候選窗（每次按鍵後沒有候選時；組字提示列由 Composition 控制）
    HideCandidates {
        owner: u64,
    },
    /// 狀態提示：中 / A / 全形 / 半形
    Hint {
        text: String,
        anchor: Option<Rect>,
    },
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
    std::thread::Builder::new().name("ui".into()).spawn(move || {
        match Ui::new(rx, theme) {
            Ok(ui) => {
                let wake = ui.cand.hwnd.0 as isize;
                UI.with(|cell| *cell.borrow_mut() = Some(ui));
                let _ = ready_tx.send(Ok(wake));
                message_loop();
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
            }
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

/// 排版結果：文字、選擇框、視窗大小（DIP）
type Laid = (Vec<Run>, Option<D2D_RECT_F>, (f32, f32));

/// 已排版、待繪製的一段文字
struct Run {
    layout: IDWriteTextLayout,
    x: f32,
    y: f32,
    dim: bool,
}

struct Popup {
    hwnd: HWND,
    target: Option<ID2D1HwndRenderTarget>,
    runs: Vec<Run>,
    /// 選擇框（DIP）
    highlight: Option<D2D_RECT_F>,
    /// 游標（DIP，組字提示列用）
    caret: Option<D2D_RECT_F>,
    /// 實體像素
    size: (u32, u32),
    scale: f32,
}

struct Ui {
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    theme: ThemeConfig,
    colors: Colors,
    text_fmt: IDWriteTextFormat,
    num_fmt: IDWriteTextFormat,
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
        Ok((fmt(t.font_size)?, fmt(t.font_size * NUM_RATIO)?))
    }

    fn set_theme(&mut self, theme: ThemeConfig) -> Result<()> {
        let (text_fmt, num_fmt) = Self::formats(&self.dw, &theme)?;
        self.text_fmt = text_fmt;
        self.num_fmt = num_fmt;
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
                    runs: Vec::new(),
                    highlight: None,
                    caret: None,
                    size: (1, 1),
                    scale: 1.0,
                })
            };
            Ok(Ui {
                d2d,
                dw,
                colors: Colors::from(&theme),
                theme,
                text_fmt,
                num_fmt,
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

    fn layout(&self, text: &str, fmt: &IDWriteTextFormat) -> Result<(IDWriteTextLayout, f32, f32)> {
        unsafe {
            let wide: Vec<u16> = text.encode_utf16().collect();
            let layout = self.dw.CreateTextLayout(&wide, fmt, 10_000.0, 1_000.0)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            Ok((layout, m.widthIncludingTrailingWhitespace, m.height))
        }
    }

    fn show_candidates(&mut self, page: &CandidatePage, anchor: Option<Rect>) -> Result<()> {
        let Some(anchor) = self.anchor(anchor) else {
            return Ok(());
        };
        let (runs, highlight, dip) = match self.theme.layout {
            _ if !page.columns.is_empty() => self.layout_columns(page)?,
            Layout::Vertical => self.layout_vertical(page)?,
            Layout::Horizontal => self.layout_horizontal(page)?,
        };
        let (d2d, colors, cand) = (&self.d2d, self.colors, &mut self.cand);
        cand.runs = runs;
        cand.highlight = highlight;
        place(cand, anchor, dip)?;
        render(d2d, &colors, cand)
    }

    /// 橫式：「1 天 2 添 …」排成一列，選擇框框住目前項目，最後是頁碼
    fn layout_horizontal(&self, page: &CandidatePage) -> Result<Laid> {
        let mut items = Vec::new();
        let mut row_h: f32 = 0.0;
        for (i, item) in page.items.iter().enumerate() {
            let mark = if page.pinned.get(i).copied().unwrap_or(false) { "★" } else { "" };
            let num = self.layout(&format!("{}{mark}", (i + 1) % 10), &self.num_fmt)?;
            let text = self.layout(item, &self.text_fmt)?;
            row_h = row_h.max(text.2).max(num.2);
            items.push((num, text));
        }
        let row_h = row_h + ROW_PAD_Y * 2.0;
        let mut runs = Vec::new();
        let mut highlight = None;
        let mut x = PAD_X;
        for (i, ((num, nw, nh), (text, tw, th))) in items.into_iter().enumerate() {
            let w = ROW_PAD_X + nw + GAP_NUM / 2.0 + tw + ROW_PAD_X;
            if i == page.highlight {
                highlight = Some(D2D_RECT_F {
                    left: x,
                    top: PAD_Y,
                    right: x + w,
                    bottom: PAD_Y + row_h,
                });
            }
            runs.push(Run {
                layout: num,
                x: x + ROW_PAD_X,
                y: PAD_Y + (row_h - nh) / 2.0,
                dim: true,
            });
            runs.push(Run {
                layout: text,
                x: x + ROW_PAD_X + nw + GAP_NUM / 2.0,
                y: PAD_Y + (row_h - th) / 2.0,
                dim: false,
            });
            x += w;
        }
        if page.total_pages > 1 {
            let (layout, w, h) =
                self.layout(&format!("{}/{}", page.page + 1, page.total_pages), &self.num_fmt)?;
            runs.push(Run {
                layout,
                x: x + ROW_PAD_X,
                y: PAD_Y + (row_h - h) / 2.0,
                dim: true,
            });
            x += w + ROW_PAD_X * 2.0;
        }
        Ok((runs, highlight, (x + PAD_X, row_h + PAD_Y * 2.0)))
    }

    /// 展開多欄：每一頁一欄並排，目前這一欄有數字與選擇框，其他欄淡色；底部顯示頁碼
    fn layout_columns(&self, page: &CandidatePage) -> Result<Laid> {
        let active = page.page.saturating_sub(page.first_column);
        let (_, num_w, _) = self.layout("0★", &self.num_fmt)?;
        let mut cols = Vec::new();
        let mut row_h: f32 = 0.0;
        for (c, items) in page.columns.iter().enumerate() {
            // 目前這欄用 page.items（含 ★ 與英文選項）
            let items = if c == active { &page.items } else { items };
            let mut rows = Vec::new();
            let mut text_w: f32 = 0.0;
            for (i, item) in items.iter().enumerate() {
                let num = if c == active {
                    let mark = if page.pinned.get(i).copied().unwrap_or(false) { "★" } else { "" };
                    Some(self.layout(&format!("{}{mark}", (i + 1) % 10), &self.num_fmt)?)
                } else {
                    None
                };
                let text = self.layout(item, &self.text_fmt)?;
                row_h = row_h.max(text.2);
                text_w = text_w.max(text.1);
                rows.push((num, text));
            }
            cols.push((rows, text_w));
        }
        let row_h = row_h + ROW_PAD_Y * 2.0;

        let mut runs = Vec::new();
        let mut highlight = None;
        let mut x = PAD_X;
        let mut bottom = PAD_Y;
        for (c, (rows, text_w)) in cols.into_iter().enumerate() {
            let col_w = ROW_PAD_X + num_w + GAP_NUM + text_w + ROW_PAD_X;
            let mut y = PAD_Y;
            for (i, (num, (text, _, th))) in rows.into_iter().enumerate() {
                if c == active && i == page.highlight {
                    highlight = Some(D2D_RECT_F {
                        left: x,
                        top: y,
                        right: x + col_w,
                        bottom: y + row_h,
                    });
                }
                if let Some((num, nw, nh)) = num {
                    runs.push(Run {
                        layout: num,
                        x: x + ROW_PAD_X + num_w - nw,
                        y: y + (row_h - nh) / 2.0,
                        dim: true,
                    });
                }
                runs.push(Run {
                    layout: text,
                    x: x + ROW_PAD_X + num_w + GAP_NUM,
                    y: y + (row_h - th) / 2.0,
                    dim: c != active,
                });
                y += row_h;
            }
            bottom = bottom.max(y);
            x += col_w;
        }
        let width = x - PAD_X;
        let (layout, w, h) =
            self.layout(&format!("{}/{}", page.page + 1, page.total_pages), &self.num_fmt)?;
        runs.push(Run {
            layout,
            x: PAD_X + width - ROW_PAD_X - w,
            y: bottom + ROW_PAD_Y,
            dim: true,
        });
        let height = bottom + h + ROW_PAD_Y * 2.0;
        Ok((runs, highlight, (PAD_X * 2.0 + width, height + PAD_Y)))
    }

    /// 直式：每列「數字 候選字」，選擇框標示目前項目，多頁時底部顯示頁碼
    fn layout_vertical(&self, page: &CandidatePage) -> Result<Laid> {
        // 數字欄寬固定，候選字才會對齊
        let (_, num_w, _) = self.layout("0", &self.num_fmt)?;
        let mut rows = Vec::new();
        let mut row_h: f32 = 0.0;
        let mut text_w: f32 = 0.0;
        for (i, item) in page.items.iter().enumerate() {
            let mark = if page.pinned.get(i).copied().unwrap_or(false) { "★" } else { "" };
            let num = self.layout(&format!("{}{mark}", (i + 1) % 10), &self.num_fmt)?;
            let text = self.layout(item, &self.text_fmt)?;
            row_h = row_h.max(text.2).max(num.2);
            text_w = text_w.max(text.1);
            rows.push((num, text));
        }
        let row_h = row_h + ROW_PAD_Y * 2.0;
        let row_w = ROW_PAD_X + num_w + GAP_NUM + text_w + ROW_PAD_X;

        let mut runs = Vec::new();
        let mut y = PAD_Y;
        let mut highlight = None;
        for (i, ((num, nw, nh), (text, _, th))) in rows.into_iter().enumerate() {
            if i == page.highlight {
                highlight = Some(D2D_RECT_F {
                    left: PAD_X,
                    top: y,
                    right: PAD_X + row_w,
                    bottom: y + row_h,
                });
            }
            let x0 = PAD_X + ROW_PAD_X;
            runs.push(Run {
                layout: num,
                // 數字靠右對齊數字欄
                x: x0 + num_w - nw,
                y: y + (row_h - nh) / 2.0,
                dim: true,
            });
            runs.push(Run {
                layout: text,
                x: x0 + num_w + GAP_NUM,
                y: y + (row_h - th) / 2.0,
                dim: false,
            });
            y += row_h;
        }
        let mut width = row_w;
        if page.total_pages > 1 {
            let (layout, w, h) =
                self.layout(&format!("{}/{}", page.page + 1, page.total_pages), &self.num_fmt)?;
            width = width.max(w + ROW_PAD_X * 2.0);
            runs.push(Run {
                layout,
                x: PAD_X + width - ROW_PAD_X - w,
                y: y + ROW_PAD_Y,
                dim: true,
            });
            y += h + ROW_PAD_Y * 2.0;
        }
        if let Some(hl) = &mut highlight {
            hl.right = PAD_X + width;
        }
        Ok((runs, highlight, (PAD_X * 2.0 + width, y + PAD_Y)))
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
        let chars: Vec<char> = p.text.chars().collect();
        let (start, len) = p.target.unwrap_or((chars.len(), 0));
        let end = (start + len).min(chars.len());
        let pieces = [
            chars[..start.min(chars.len())].iter().collect::<String>(),
            chars[start.min(chars.len())..end].iter().collect::<String>(),
            chars[end..].iter().collect::<String>(),
        ];
        let mut runs = Vec::new();
        let mut x = PAD_X + ROW_PAD_X;
        let mut h: f32 = 0.0;
        let mut highlight = None;
        for (i, piece) in pieces.iter().enumerate() {
            if piece.is_empty() {
                continue;
            }
            let (layout, w, lh) = self.layout(piece, &self.text_fmt)?;
            h = h.max(lh);
            if i == 1 {
                highlight = Some(D2D_RECT_F {
                    left: x - 1.0,
                    top: PAD_Y,
                    right: x + w + 1.0,
                    bottom: PAD_Y + lh + ROW_PAD_Y * 2.0,
                });
            }
            runs.push(Run {
                layout,
                x,
                y: PAD_Y + ROW_PAD_Y,
                dim: false,
            });
            x += w;
        }
        let (count, cw, ch) = self.layout(&format!("{} 字", p.len), &self.num_fmt)?;
        let row_h = h + ROW_PAD_Y * 2.0;
        runs.push(Run {
            layout: count,
            x: x + GAP_NUM * 2.0,
            y: PAD_Y + (row_h - ch) / 2.0,
            dim: true,
        });
        let width = x + GAP_NUM * 2.0 + cw + ROW_PAD_X + PAD_X;
        if let Some(hl) = &mut highlight {
            hl.bottom = PAD_Y + row_h;
        }
        // 游標：游標前文字的寬度
        let x0 = PAD_X + ROW_PAD_X;
        let before: String = chars[..p.cursor.min(chars.len())].iter().collect();
        let caret_x = if before.is_empty() { x0 } else { x0 + self.layout(&before, &self.text_fmt)?.1 };
        let caret = D2D_RECT_F {
            left: caret_x - 0.5,
            top: PAD_Y + ROW_PAD_Y,
            right: caret_x + 1.0,
            bottom: PAD_Y + row_h - ROW_PAD_Y,
        };
        let (d2d, colors, comp) = (&self.d2d, self.colors, &mut self.comp);
        comp.runs = runs;
        comp.highlight = highlight;
        comp.caret = Some(caret);
        place_at(comp, anchor, (width, row_h + PAD_Y * 2.0), true)?;
        render(d2d, &colors, comp)
    }

    fn show_hint(&mut self, text: &str, anchor: Option<Rect>) -> Result<()> {
        let Some(anchor) = self.anchor(anchor) else {
            return Ok(());
        };
        let (layout, w, h) = self.layout(text, &self.text_fmt)?;
        let (d2d, colors, hint) = (&self.d2d, self.colors, &mut self.hint);
        hint.highlight = None;
        hint.runs = vec![Run {
            layout,
            x: PAD_X,
            y: PAD_Y,
            dim: false,
        }];
        place(hint, anchor, (w + PAD_X * 2.0, h + PAD_Y * 2.0))?;
        render(d2d, &colors, hint)?;
        unsafe {
            SetTimer(Some(hint.hwnd), HINT_TIMER, HINT_MS, None);
        }
        Ok(())
    }
}

/// 依游標位置放置視窗：預設在游標下方，超出螢幕下緣就改放上方
fn place(p: &mut Popup, anchor: Rect, dip: (f32, f32)) -> Result<()> {
    place_at(p, anchor, dip, false)
}

/// `above`：優先放在游標上方（組字提示列，避免與下方的候選窗重疊）
fn place_at(p: &mut Popup, anchor: Rect, dip: (f32, f32), above: bool) -> Result<()> {
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
        let gap = (OFFSET_DIP * scale) as i32;
        let mut y = if above { rc.top - gap - h } else { rc.bottom + gap };
        if above && y < work.top {
            y = rc.bottom + gap;
        } else if !above && y + h > work.bottom {
            y = rc.top - gap - h;
        }
        let x = rc.left.clamp(work.left, (work.right - w).max(work.left));
        let y = y.clamp(work.top, (work.bottom - h).max(work.top));

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
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )?;
        Ok(())
    }
}

fn render(d2d: &ID2D1Factory, c: &Colors, p: &mut Popup) -> Result<()> {
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
        if let Some(rect) = p.highlight {
            let hl = t.CreateSolidColorBrush(&c.hl, None)?;
            t.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect,
                    radiusX: ROW_RADIUS,
                    radiusY: ROW_RADIUS,
                },
                &hl,
            );
        }
        if let Some(rect) = p.caret {
            t.FillRectangle(&rect, &fg);
        }
        for run in &p.runs {
            let brush = if run.dim { &dim } else { &fg };
            t.DrawTextLayout(
                Vector2 { X: run.x, Y: run.y },
                &run.layout,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            );
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
                let d2d = ui.d2d.clone();
                let colors = ui.colors;
                let p = if ui.cand.hwnd == hwnd {
                    &mut ui.cand
                } else {
                    &mut ui.hint
                };
                let _ = render(&d2d, &colors, p);
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
