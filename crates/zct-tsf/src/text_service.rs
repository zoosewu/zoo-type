//! TSF 文字服務本體
//!
//! 按鍵流程（每次一到兩次 IPC 往返）：
//! - `OnTestKeyDown` → `TestKeyDown`：會不會吃掉（不吃的鍵服務程序仍會觀察，供 Shift 判定）
//! - `OnKeyDown`     → `Key`：處理並更新文件
//! - `OnTestKeyUp`   → `Key`：Shift 單按在放開時才成立；放開事件一律交給 App，需要更新文件時用非同步 edit session
//!
//! 借用規則：每個 RefCell 只短暫借用，絕不在借用期間呼叫 TSF（TSF 可能重入）。

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::{GUITHREADINFO, GetGUIThreadInfo, GetWindowRect};
use windows::core::{BOOL, GUID, IUnknown, Interface, Ref, Result, implement};
use windows_core::{ComObject, IUnknownImpl};
use zct_core::hotkey::Binding;
use zct_core::{KeyEvent, KeyResult};
use zct_ipc::{Command, Rect, Request, Response};

use crate::client::Ipc;
use crate::detect::{self, FullscreenCache};
use crate::display::{self, Atoms};
use crate::edit::EditSession;
use crate::langbar::{self, Action, LangBarButton};
use crate::{guard, keys};

#[derive(Default)]
struct Tsf {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    thread_sink_cookie: Option<u32>,
    atoms: Atoms,
    /// 工作列右下角的輸入法圖示
    langbar: Option<ITfLangBarItemButton>,
}

#[implement(
    ITfTextInputProcessorEx,
    ITfKeyEventSink,
    ITfThreadMgrEventSink,
    ITfCompositionSink,
    ITfDisplayAttributeProvider
)]
pub struct TextService {
    tsf: RefCell<Tsf>,
    composition: RefCell<Option<ITfComposition>>,
    /// 最後寫進組字區的文字：用來發現 App 自己結束了組字卻沒通知
    composition_text: RefCell<String>,
    /// `OnTestKeyUp` 已處理過的放開事件（wparam, lparam）：App 接著又呼叫 `OnKeyUp` 時不重複處理
    tested_up: Cell<Option<(usize, isize)>>,
    /// 已向 TSF 登記右 Alt 的保留按鍵
    right_alt: Cell<bool>,
    /// 上一次查游標位置時 App 有回報（不是沿用舊位置）
    last_fresh: Cell<bool>,
    ipc: RefCell<Ipc>,
    fullscreen: FullscreenCache,
    /// 最後一次從 App 取得的游標位置：App 暫時回報不出來時沿用（VS Code 終端機打注音途中常取不到），
    /// 避免視窗跳到備用位置再跳回來；焦點換到別的輸入框時清掉
    last_rect: Cell<Option<Rect>>,
    /// 遊戲模式鎖定英文時，還要交給服務程序的快捷鍵；其他按鍵直接放行（`refresh_langbar` 更新）
    passthrough: RefCell<Option<Vec<Binding>>>,
}

impl TextService {
    pub fn new() -> TextService {
        TextService {
            tsf: RefCell::default(),
            composition: RefCell::default(),
            composition_text: RefCell::default(),
            tested_up: Cell::new(None),
            right_alt: Cell::new(false),
            last_fresh: Cell::new(false),
            ipc: RefCell::default(),
            fullscreen: FullscreenCache::default(),
            last_rect: Cell::new(None),
            passthrough: RefCell::new(None),
        }
    }

    /// 遊戲模式鎖定英文：快捷鍵以外的按鍵（含所有放開事件）直接交給 App，不詢問服務程序
    fn game_passes(&self, ev: &KeyEvent) -> bool {
        self.passthrough
            .borrow()
            .as_ref()
            .is_some_and(|keys| !keys.iter().any(|b| b.matches(ev)))
    }

    fn call(&self, req: &Request) -> Option<Response> {
        self.ipc.try_borrow_mut().ok()?.call(req)
    }

    fn client_id(&self) -> u32 {
        self.tsf.borrow().client_id
    }

    /// 不在輸入狀態：按鍵全部交給 App，也不通知服務程序（Shift 不會切換中英）
    ///
    /// 組字中一律視為輸入狀態，避免組字卡在半途
    fn not_input(&self) -> bool {
        if self.composition.borrow().is_some() {
            return false;
        }
        let Some(tm) = self.tsf.borrow().thread_mgr.clone() else {
            return true;
        };
        let heuristics = match self.ipc.try_borrow_mut() {
            Ok(mut ipc) => ipc.heuristics(),
            Err(_) => return true,
        };
        detect::not_input(
            &detect::gather(&tm, heuristics, &self.fullscreen),
            heuristics,
        )
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn set_selection(ctx: &ITfContext, ec: u32, range: &ITfRange) -> Result<()> {
    let mut sel = TF_SELECTION {
        range: ManuallyDrop::new(Some(range.clone())),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_NONE,
            fInterimChar: false.into(),
        },
    };
    let r = unsafe { ctx.SetSelection(ec, std::slice::from_ref(&sel)) };
    unsafe { ManuallyDrop::drop(&mut sel.range) };
    r
}

/// 一次最多讀幾個 UTF-16 單位：夠判斷是否超過重新選字的上限
const SELECTION_READ: usize = 64;

/// App 中選取的文字；讀不到時是空字串
///
/// 完整支援 TSF 的 App（Windows 11 記事本、Chromium、Office…）從 TSF 讀。傳統 Win32 編輯框在 TSF 裡是
/// 暫時性 context，只看得到組字、看不到原有的文字，這時直接問焦點所在的 Edit / RichEdit 控制項
/// （輸入法 DLL 就在 App 的程序裡）
fn selected_text(client_id: u32, ctx: &ITfContext) -> String {
    let out = std::rc::Rc::new(RefCell::new(String::new()));
    let sink = out.clone();
    let ctx2 = ctx.clone();
    let session = EditSession::create(move |ec| {
        unsafe {
            let mut sel = [TF_SELECTION::default()];
            let mut fetched = 0u32;
            ctx2.GetSelection(ec, TF_DEFAULT_SELECTION, &mut sel, &mut fetched)?;
            let range = ManuallyDrop::into_inner(std::mem::take(&mut sel[0].range));
            if let Some(range) = range.filter(|_| fetched == 1) {
                let mut buf = [0u16; SELECTION_READ];
                let mut n = 0u32;
                range.GetText(ec, 0, &mut buf, &mut n)?;
                *sink.borrow_mut() = String::from_utf16_lossy(&buf[..n as usize]);
            }
        }
        Ok(())
    });
    unsafe {
        let _ = ctx.RequestEditSession(client_id, &session, TF_ES_SYNC | TF_ES_READ);
    }
    let text = out.take();
    if text.is_empty() {
        control_selected_text().unwrap_or_default()
    } else {
        text
    }
}

/// 焦點所在的標準 Edit / RichEdit 控制項中選取的文字
fn control_selected_text() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, SendMessageW, WM_GETTEXT, WM_GETTEXTLENGTH,
    };
    const EM_GETSEL: u32 = 0x00B0;
    // RichEdit 的 EM_GETSELTEXT（WM_USER + 62）：直接取選取的文字，不必處理換行算法的差異
    const EM_GETSELTEXT: u32 = 0x0400 + 62;
    unsafe {
        let hwnd = GetFocus();
        if hwnd.is_invalid() {
            return None;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]).to_ascii_lowercase();
        let mut start = 0u32;
        let mut end = 0u32;
        SendMessageW(
            hwnd,
            EM_GETSEL,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as isize)),
        );
        let len = end.checked_sub(start)? as usize;
        if len == 0 || len > SELECTION_READ {
            return None;
        }
        if class.starts_with("richedit") {
            let mut buf = vec![0u16; len + 1];
            let n = SendMessageW(
                hwnd,
                EM_GETSELTEXT,
                None,
                Some(LPARAM(buf.as_mut_ptr() as isize)),
            )
            .0 as usize;
            Some(String::from_utf16_lossy(&buf[..n.min(len)]))
        } else if class == "edit" {
            let total = SendMessageW(hwnd, WM_GETTEXTLENGTH, None, None).0 as usize;
            // 很大的文件不讀（只是為了取選取的幾個字）
            if total > 1 << 20 || end as usize > total {
                return None;
            }
            let mut buf = vec![0u16; total + 1];
            let n = SendMessageW(
                hwnd,
                WM_GETTEXT,
                Some(WPARAM(buf.len())),
                Some(LPARAM(buf.as_mut_ptr() as isize)),
            )
            .0 as usize;
            buf.get(start as usize..(end as usize).min(n))
                .map(String::from_utf16_lossy)
        } else {
            None
        }
    }
}
/// 目前游標（選取範圍）的螢幕矩形；App 沒排版好時回傳 None
fn caret_rect(ctx: &ITfContext, ec: u32) -> Option<Rect> {
    unsafe {
        let mut sel = [TF_SELECTION::default()];
        let mut fetched = 0u32;
        ctx.GetSelection(ec, TF_DEFAULT_SELECTION, &mut sel, &mut fetched)
            .ok()?;
        let range = ManuallyDrop::into_inner(std::mem::take(&mut sel[0].range));
        let range = range.filter(|_| fetched == 1)?;
        let view = ctx.GetActiveView().ok()?;
        let mut rc = RECT::default();
        let mut clipped = BOOL::default();
        view.GetTextExt(ec, &range, &mut rc, &mut clipped).ok()?;
        (rc != RECT::default()).then_some(Rect {
            left: rc.left,
            top: rc.top,
            right: rc.right,
            bottom: rc.bottom,
        })
    }
}

/// P4 fallback：App 不回報文字位置時，改用系統游標、再不行用焦點視窗左下角
fn fallback_rect() -> Option<Rect> {
    unsafe {
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(GetCurrentThreadId(), &mut info).is_ok()
            && !info.hwndCaret.is_invalid()
            && info.rcCaret != RECT::default()
        {
            let mut tl = POINT {
                x: info.rcCaret.left,
                y: info.rcCaret.top,
            };
            let mut br = POINT {
                x: info.rcCaret.right,
                y: info.rcCaret.bottom,
            };
            let _ = ClientToScreen(info.hwndCaret, &mut tl);
            let _ = ClientToScreen(info.hwndCaret, &mut br);
            return Some(Rect {
                left: tl.x,
                top: tl.y,
                right: br.x,
                bottom: br.y,
            });
        }
        let focus: HWND = GetFocus();
        let mut rc = RECT::default();
        if !focus.is_invalid() && GetWindowRect(focus, &mut rc).is_ok() {
            return Some(Rect {
                left: rc.left,
                top: rc.bottom - 24,
                right: rc.left + 1,
                bottom: rc.bottom,
            });
        }
        None
    }
}

impl TextService_Impl {
    fn this(&self) -> ComObject<TextService> {
        self.to_object()
    }

    /// 把一次按鍵結果套用到文件，並把游標位置回報給服務程序
    fn apply(&self, ctx: &ITfContext, r: KeyResult) {
        self.apply_with(ctx, r, TF_ES_SYNC);
    }

    /// `sync`：`TF_ES_SYNC`（按鍵處理中）或 `TF_ES_ASYNCDONTCARE`（不允許同步編輯的地方，例如 `OnTestKeyUp`）
    fn apply_with(&self, ctx: &ITfContext, r: KeyResult, sync: TF_CONTEXT_EDIT_CONTEXT_FLAGS) {
        let has_composition = self.composition.borrow().is_some();
        let touches_doc = r.commit.is_some() || !r.preedit.text.is_empty() || has_composition;
        if r.mode_changed.is_some() || r.output_changed.is_some() || r.toggle_game {
            self.refresh_langbar();
        }
        let needs_caret = touches_doc
            || r.candidates.is_some()
            || r.mode_changed.is_some()
            || r.width_changed.is_some()
            || r.output_changed.is_some()
            || r.toggle_game;

        let rect = Cell::new(None);
        if needs_caret {
            let this = self.this();
            let ctx2 = ctx.clone();
            let rect_out = std::rc::Rc::new(Cell::new(None));
            let rect_in = rect_out.clone();
            let session = EditSession::create(move |ec| {
                if touches_doc {
                    update_document(&this, &ctx2, ec, &r)?;
                }
                rect_in.set(caret_rect(&ctx2, ec));
                Ok(())
            });
            unsafe {
                let _ = ctx.RequestEditSession(self.client_id(), &session, sync | TF_ES_READWRITE);
            }
            let got = rect_out.get();
            self.last_fresh.set(got.is_some());
            if got.is_some() {
                self.last_rect.set(got);
            }
            rect.set(got.or(self.last_rect.get()).or_else(fallback_rect));
        }
        let fresh = needs_caret && self.last_fresh.get();
        self.call(&Request::Caret {
            rect: rect.get(),
            fresh,
        });
    }

    /// App 自己結束了組字卻沒通知（Windows 11 記事本按 Ctrl + , 這類 App 自己的快捷鍵時）：
    /// 字已經由 App 送出，組字區的文字不再是我們寫的那段。這時結束那個組字、清掉服務程序的組字，
    /// 不然候選窗會留著，之後再按 Enter 又送出一次
    fn drop_lost_composition(&self, ctx: &ITfContext) {
        let Some(comp) = self.composition.borrow().clone() else {
            return;
        };
        let want = wide(&self.composition_text.borrow());
        let lost = std::rc::Rc::new(Cell::new(false));
        let out = lost.clone();
        let session = EditSession::create(move |ec| {
            let same = unsafe {
                comp.GetRange().and_then(|range| {
                    let mut buf = vec![0u16; want.len() + 1];
                    let mut n = 0u32;
                    range.GetText(ec, 0, &mut buf, &mut n)?;
                    Ok(buf[..n as usize] == want[..])
                })
            };
            if !same.unwrap_or(false) {
                out.set(true);
                let _ = unsafe { comp.EndComposition(ec) };
            }
            Ok(())
        });
        unsafe {
            let _ =
                ctx.RequestEditSession(self.client_id(), &session, TF_ES_SYNC | TF_ES_READWRITE);
        }
        if lost.get() {
            self.reset(Request::Reset);
        }
    }

    /// 重新選字：讀取選取的文字交給服務程序；回來的組字從選取範圍開始，組字文字取代選取的內容
    fn reconvert(&self, ctx: &ITfContext) {
        let text = selected_text(self.client_id(), ctx);
        let req = Request::Reconvert { text };
        let resp = self
            .ipc
            .try_borrow_mut()
            .ok()
            .and_then(|mut ipc| ipc.call_with(&req, std::time::Duration::from_secs(1)));
        if let Some(Response::Key(r)) = resp {
            self.apply(ctx, r);
        }
    }

    fn reset(&self, req: Request) {
        self.composition.borrow_mut().take();
        self.tested_up.set(None);
        self.last_rect.set(None);
        self.call(&req);
        self.refresh_langbar();
    }

    /// 向服務程序查詢中英與遊戲模式，更新輸入法圖示與遊戲模式的直接放行
    ///
    /// 焦點換到別的文件、切換遊戲模式時都會呼叫，在別的 App 切換的全域遊戲模式也會在這時同步。
    /// 查不到（服務程序沒在跑）時取消直接放行，按鍵才會走到重新啟動服務程序的路徑
    fn refresh_langbar(&self) {
        let Some(Response::Status {
            chinese,
            simplified,
            game,
            right_alt,
            update,
            passthrough,
        }) = self.call(&Request::Status)
        else {
            self.passthrough.replace(None);
            return;
        };
        self.passthrough.replace(passthrough);
        let button = self.tsf.borrow().langbar.clone();
        if let Some(button) = button {
            langbar::set_state(&button, chinese, simplified, game, update);
        }
        self.sync_right_alt(right_alt);
    }

    /// 依設定登記或取消右 Alt 的保留按鍵（設定關閉時完全不攔右 Alt 組合鍵）
    fn sync_right_alt(&self, enabled: bool) {
        if self.right_alt.get() == enabled {
            return;
        }
        let tsf = self.tsf.borrow();
        let Some(km) = tsf
            .thread_mgr
            .as_ref()
            .and_then(|tm| tm.cast::<ITfKeystrokeMgr>().ok())
        else {
            return;
        };
        preserve_right_alt(&km, tsf.client_id, enabled);
        self.right_alt.set(enabled);
    }

    /// 輸入法圖示的點擊與選單
    fn on_action(&self, action: Action) {
        let command = match action {
            Action::ToggleMode | Action::ToggleOutput => {
                let request = if action == Action::ToggleMode {
                    Request::ToggleMode
                } else {
                    Request::ToggleOutput
                };
                if let Some(Response::Key(r)) = self.call(&request) {
                    // 組字中切換：送出組字內容需要目前文件的 context
                    let ctx = self
                        .tsf
                        .borrow()
                        .thread_mgr
                        .clone()
                        .and_then(|tm| unsafe { tm.GetFocus().ok()?.GetTop().ok() });
                    if let Some(ctx) = ctx {
                        self.apply(&ctx, r);
                    }
                }
                self.refresh_langbar();
                return;
            }
            Action::OpenSettings => {
                // 選單在 App 裡（前景）：允許設定程式把自己的視窗叫到前面
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                        windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
                    );
                }
                Command::OpenSettings
            }
            Action::ToggleGame => Command::ToggleGame,
            Action::Restart => Command::Restart,
            Action::InstallUpdate => Command::InstallUpdate,
            Action::Refresh => {
                self.refresh_langbar();
                return;
            }
        };
        self.call(&Request::Command(command));
        self.refresh_langbar();
    }
}

fn update_document(
    this: &ComObject<TextService>,
    ctx: &ITfContext,
    ec: u32,
    r: &KeyResult,
) -> Result<()> {
    unsafe {
        if let Some(text) = &r.commit {
            let current = this.composition.borrow_mut().take();
            match current {
                Some(comp) => {
                    let range = comp.GetRange()?;
                    range.SetText(ec, 0, &wide(text))?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    set_selection(ctx, ec, &range)?;
                    comp.EndComposition(ec)?;
                }
                None => {
                    let ins: ITfInsertAtSelection = ctx.cast()?;
                    let range = ins.InsertTextAtSelection(
                        ec,
                        INSERT_TEXT_AT_SELECTION_FLAGS(0),
                        &wide(text),
                    )?;
                    range.Collapse(ec, TF_ANCHOR_END)?;
                    set_selection(ctx, ec, &range)?;
                }
            }
        }

        let preedit = &r.preedit;
        if preedit.text.is_empty() {
            let current = this.composition.borrow_mut().take();
            if let Some(comp) = current {
                let range = comp.GetRange()?;
                range.SetText(ec, 0, &[])?;
                comp.EndComposition(ec)?;
            }
            return Ok(());
        }

        let existing = this.composition.borrow().clone();
        let comp = match existing {
            Some(c) => c,
            None => {
                let ins: ITfInsertAtSelection = ctx.cast()?;
                let range = ins.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;
                let cc: ITfContextComposition = ctx.cast()?;
                let sink: ITfCompositionSink = this.to_interface();
                let c = cc.StartComposition(ec, &range, &sink)?;
                *this.composition.borrow_mut() = Some(c.clone());
                c
            }
        };
        let range = comp.GetRange()?;
        range.SetText(ec, TF_ST_CORRECTION, &wide(&preedit.text))?;
        *this.composition_text.borrow_mut() = preedit.text.clone();

        // 底線：整段組字 + 選字目標（字元位置換成 UTF-16）
        let display = preedit.display();
        let target = display
            .target_utf16()
            .map(|(start, len)| (start as i32, len as i32));
        let atoms = this.tsf.borrow().atoms;
        display::apply(ctx, ec, &range, atoms, target);

        // 組字區內的游標：從開頭往後移 cursor 個 UTF-16 單位
        let caret = range.Clone()?;
        caret.Collapse(ec, TF_ANCHOR_START)?;
        let mut moved = 0i32;
        let n = display.cursor_utf16() as i32;
        caret.ShiftEnd(ec, n, &mut moved, std::ptr::null())?;
        caret.Collapse(ec, TF_ANCHOR_END)?;
        set_selection(ctx, ec, &caret)?;
    }
    Ok(())
}

/// 登記（或取消）右 Alt（+ Shift）+ 可以打字的鍵為保留按鍵
fn preserve_right_alt(km: &ITfKeystrokeMgr, tid: u32, preserve: bool) {
    let desc: Vec<u16> = "右 Alt 符號".encode_utf16().collect();
    for (guid, vk, shift, _) in keys::right_alt_keys() {
        let key = TF_PRESERVEDKEY {
            uVKey: vk.0 as u32,
            uModifiers: TF_MOD_RALT | if shift { TF_MOD_SHIFT } else { 0 },
        };
        unsafe {
            let _ = if preserve {
                km.PreserveKey(tid, &guid, &key, &desc)
            } else {
                km.UnpreserveKey(&guid, &key)
            };
        }
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        guard(|| {
            let tsf = std::mem::take(&mut *self.tsf.borrow_mut());
            if let Some(tm) = &tsf.thread_mgr {
                unsafe {
                    if let Some(button) = &tsf.langbar {
                        if let Ok(mgr) = tm.cast::<ITfLangBarItemMgr>() {
                            let _ = mgr.RemoveItem(button);
                        }
                        langbar::detach(button);
                    }
                    if let Ok(km) = tm.cast::<ITfKeystrokeMgr>() {
                        if self.right_alt.replace(false) {
                            preserve_right_alt(&km, tsf.client_id, false);
                        }
                        let _ = km.UnadviseKeyEventSink(tsf.client_id);
                    }
                    if let (Some(cookie), Ok(src)) =
                        (tsf.thread_sink_cookie, tm.cast::<ITfSource>())
                    {
                        let _ = src.UnadviseSink(cookie);
                    }
                }
            }
            self.composition.borrow_mut().take();
            self.tested_up.set(None);
            self.passthrough.replace(None);
            if let Ok(mut ipc) = self.ipc.try_borrow_mut() {
                ipc.disconnect();
            }
            Ok(())
        })
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(&self, ptim: Ref<ITfThreadMgr>, tid: u32, _flags: u32) -> Result<()> {
        guard(|| {
            let tm = ptim.ok()?.clone();
            unsafe {
                let km: ITfKeystrokeMgr = tm.cast()?;
                let key_sink: ITfKeyEventSink = self.to_interface();
                km.AdviseKeyEventSink(tid, &key_sink, true)?;
                // 右 Alt 的保留按鍵在 refresh_langbar 依設定登記

                let src: ITfSource = tm.cast()?;
                let thread_sink: ITfThreadMgrEventSink = self.to_interface();
                let cookie = src
                    .AdviseSink(
                        &ITfThreadMgrEventSink::IID,
                        &thread_sink.cast::<IUnknown>()?,
                    )
                    .ok();

                // 輸入法圖示：處理函式持有 TextService，Deactivate 時解除
                let this = self.this();
                let button = LangBarButton::create(Box::new(move |a| this.on_action(a)));
                let langbar = tm
                    .cast::<ITfLangBarItemMgr>()
                    .ok()
                    .and_then(|mgr| mgr.AddItem(&button).ok())
                    .map(|_| button);

                *self.tsf.borrow_mut() = Tsf {
                    thread_mgr: Some(tm),
                    client_id: tid,
                    thread_sink_cookie: cookie,
                    atoms: Atoms::register().unwrap_or_default(),
                    langbar,
                };
            }
            self.refresh_langbar();
            Ok(())
        })
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, _foreground: BOOL) -> Result<()> {
        Ok(())
    }

    fn OnTestKeyDown(&self, _pic: Ref<ITfContext>, w: WPARAM, l: LPARAM) -> Result<BOOL> {
        guard(|| {
            // 新的按鍵：上一個放開事件的記錄作廢（同一個鍵的放開事件 lparam 每次都一樣）
            self.tested_up.set(None);
            let ev = keys::event(w, l, false);
            if self.game_passes(&ev) || self.not_input() {
                return Ok(false.into());
            }
            let eat = matches!(
                self.call(&Request::TestKeyDown(ev)),
                Some(Response::Eat(true))
            );
            Ok(eat.into())
        })
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, w: WPARAM, l: LPARAM) -> Result<BOOL> {
        guard(|| {
            // 新的按鍵：上一個放開事件的記錄作廢（同一個鍵的放開事件 lparam 每次都一樣）
            self.tested_up.set(None);
            let ev = keys::event(w, l, false);
            if self.game_passes(&ev) || self.not_input() {
                return Ok(false.into());
            }
            if let Some(ctx) = pic.as_ref() {
                self.drop_lost_composition(ctx);
            }
            let Some(Response::Key(r)) = self.call(&Request::Key(ev)) else {
                return Ok(false.into());
            };
            let (eaten, reconvert) = (r.eaten, r.reconvert);
            if let Some(ctx) = pic.as_ref() {
                self.apply(ctx, r);
                if reconvert {
                    self.reconvert(ctx);
                }
            }
            Ok(eaten.into())
        })
    }

    /// 放開事件一律交給 App（回傳 false）：吃掉的話 App 以為按鍵還按著，
    /// 例如單按 Shift 切中英後，設定程式的 Tab 變成 Shift + Tab。需要更新文件時改用非同步的 edit session
    fn OnTestKeyUp(&self, pic: Ref<ITfContext>, w: WPARAM, l: LPARAM) -> Result<BOOL> {
        guard(|| {
            let ev = keys::event(w, l, true);
            if self.game_passes(&ev) || self.not_input() {
                return Ok(false.into());
            }
            self.tested_up.set(Some((w.0, l.0)));
            if let Some(Response::Key(r)) = self.call(&Request::Key(ev))
                && (r.commit.is_some() || r.mode_changed.is_some())
                && let Some(ctx) = pic.as_ref()
            {
                self.apply_with(ctx, r, TF_ES_ASYNCDONTCARE);
            }
            Ok(false.into())
        })
    }

    /// 有些 App（例如 Win11 記事本）直接呼叫 ITfKeystrokeMgr::KeyUp，不會先經過 OnTestKeyUp；
    /// 這時要在這裡處理放開事件，否則單按 Shift 永遠不會切換中英
    fn OnKeyUp(&self, pic: Ref<ITfContext>, w: WPARAM, l: LPARAM) -> Result<BOOL> {
        guard(|| {
            let ev = keys::event(w, l, true);
            if self.tested_up.take() == Some((w.0, l.0))
                || self.game_passes(&ev)
                || self.not_input()
            {
                return Ok(false.into());
            }
            if let Some(Response::Key(r)) = self.call(&Request::Key(ev))
                && (r.commit.is_some() || r.mode_changed.is_some())
                && let Some(ctx) = pic.as_ref()
            {
                self.apply(ctx, r);
            }
            Ok(false.into())
        })
    }

    fn OnPreservedKey(&self, pic: Ref<ITfContext>, guid: *const GUID) -> Result<BOOL> {
        guard(|| {
            if guid.is_null() || self.passthrough.borrow().is_some() || self.not_input() {
                return Ok(false.into());
            }
            if let Some(ctx) = pic.as_ref() {
                self.drop_lost_composition(ctx);
            }
            let guid = unsafe { *guid };
            let Some((_, _, shift, c)) = keys::right_alt_keys().find(|(g, ..)| *g == guid) else {
                return Ok(false.into());
            };
            let Some(Response::Key(r)) = self.call(&Request::Key(keys::right_alt_event(c, shift)))
            else {
                return Ok(false.into());
            };
            let eaten = r.eaten;
            if let Some(ctx) = pic.as_ref() {
                self.apply(ctx, r);
            }
            Ok(eaten.into())
        })
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    /// 焦點換到別的文件：丟棄組字狀態、隱藏候選窗，依設定重設中英狀態
    fn OnSetFocus(&self, focus: Ref<ITfDocumentMgr>, _prev: Ref<ITfDocumentMgr>) -> Result<()> {
        guard(|| {
            let req = if focus.is_some() {
                Request::Focus
            } else {
                Request::Reset
            };
            self.reset(req);
            Ok(())
        })
    }

    fn OnPushContext(&self, _pic: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _pic: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }
}

impl ITfDisplayAttributeProvider_Impl for TextService_Impl {
    fn EnumDisplayAttributeInfo(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        Ok(display::AttributeEnum::create())
    }

    fn GetDisplayAttributeInfo(&self, guid: *const GUID) -> Result<ITfDisplayAttributeInfo> {
        guard(|| {
            if guid.is_null() {
                return Err(windows::Win32::Foundation::E_INVALIDARG.into());
            }
            display::info(unsafe { &*guid })
                .ok_or_else(|| windows::Win32::Foundation::E_INVALIDARG.into())
        })
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    /// App 主動結束組字（例如滑鼠點到別處）：文字已留在文件中，只需同步狀態
    fn OnCompositionTerminated(&self, _ec: u32, _comp: Ref<ITfComposition>) -> Result<()> {
        guard(|| {
            self.reset(Request::Reset);
            Ok(())
        })
    }
}
