//! TSF 文字服務本體
//!
//! 按鍵流程（每次一到兩次 IPC 往返）：
//! - `OnTestKeyDown` → `TestKeyDown`：會不會吃掉（不吃的鍵服務程序仍會觀察，供 Shift 判定）
//! - `OnKeyDown`     → `Key`：處理並更新文件
//! - `OnTestKeyUp`   → `Key`：Shift 單按在放開時才成立；需要更新文件時吃掉，交給 `OnKeyUp`
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
use zct_core::KeyResult;
use zct_ipc::{Rect, Request, Response};

use crate::client::Ipc;
use crate::edit::EditSession;
use crate::{guard, keys};

#[derive(Default)]
struct Tsf {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    thread_sink_cookie: Option<u32>,
}

#[implement(ITfTextInputProcessorEx, ITfKeyEventSink, ITfThreadMgrEventSink, ITfCompositionSink)]
pub struct TextService {
    tsf: RefCell<Tsf>,
    composition: RefCell<Option<ITfComposition>>,
    /// `OnTestKeyUp` 已處理、等 `OnKeyUp` 套用到文件的結果
    pending_up: RefCell<Option<KeyResult>>,
    ipc: RefCell<Ipc>,
}

impl TextService {
    pub fn new() -> TextService {
        TextService {
            tsf: RefCell::default(),
            composition: RefCell::default(),
            pending_up: RefCell::default(),
            ipc: RefCell::default(),
        }
    }

    fn call(&self, req: &Request) -> Option<Response> {
        self.ipc.try_borrow_mut().ok()?.call(req)
    }

    fn client_id(&self) -> u32 {
        self.tsf.borrow().client_id
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
        let has_composition = self.composition.borrow().is_some();
        let touches_doc = r.commit.is_some() || !r.preedit.text.is_empty() || has_composition;
        let needs_caret = touches_doc
            || r.candidates.is_some()
            || r.mode_changed.is_some()
            || r.width_changed.is_some();

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
                let _ = ctx.RequestEditSession(
                    self.client_id(),
                    &session,
                    TF_ES_SYNC | TF_ES_READWRITE,
                );
            }
            rect.set(rect_out.get().or_else(fallback_rect));
        }
        self.call(&Request::Caret(rect.get()));
    }

    fn reset(&self) {
        self.composition.borrow_mut().take();
        self.pending_up.borrow_mut().take();
        self.call(&Request::Reset);
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
                    let range =
                        ins.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &wide(text))?;
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

        // 組字區內的游標：從開頭往後移 cursor 個 UTF-16 單位
        let prefix: String = preedit.text.chars().take(preedit.cursor).collect();
        let caret = range.Clone()?;
        caret.Collapse(ec, TF_ANCHOR_START)?;
        let mut moved = 0i32;
        let n = prefix.encode_utf16().count() as i32;
        caret.ShiftEnd(ec, n, &mut moved, std::ptr::null())?;
        caret.Collapse(ec, TF_ANCHOR_END)?;
        set_selection(ctx, ec, &caret)?;
    }
    Ok(())
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
                    if let Ok(km) = tm.cast::<ITfKeystrokeMgr>() {
                        let _ = km.UnadviseKeyEventSink(tsf.client_id);
                    }
                    if let (Some(cookie), Ok(src)) = (tsf.thread_sink_cookie, tm.cast::<ITfSource>())
                    {
                        let _ = src.UnadviseSink(cookie);
                    }
                }
            }
            self.composition.borrow_mut().take();
            self.pending_up.borrow_mut().take();
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

                let src: ITfSource = tm.cast()?;
                let thread_sink: ITfThreadMgrEventSink = self.to_interface();
                let cookie = src
                    .AdviseSink(&ITfThreadMgrEventSink::IID, &thread_sink.cast::<IUnknown>()?)
                    .ok();

                *self.tsf.borrow_mut() = Tsf {
                    thread_mgr: Some(tm),
                    client_id: tid,
                    thread_sink_cookie: cookie,
                };
            }
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
            let ev = keys::event(w, l, false);
            let eat = matches!(self.call(&Request::TestKeyDown(ev)), Some(Response::Eat(true)));
            Ok(eat.into())
        })
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, w: WPARAM, l: LPARAM) -> Result<BOOL> {
        guard(|| {
            let ev = keys::event(w, l, false);
            let Some(Response::Key(r)) = self.call(&Request::Key(ev)) else {
                return Ok(false.into());
            };
            let eaten = r.eaten;
            if let Some(ctx) = pic.as_ref() {
                self.apply(ctx, r);
            }
            Ok(eaten.into())
        })
    }

    fn OnTestKeyUp(&self, _pic: Ref<ITfContext>, w: WPARAM, l: LPARAM) -> Result<BOOL> {
        guard(|| {
            let ev = keys::event(w, l, true);
            let Some(Response::Key(r)) = self.call(&Request::Key(ev)) else {
                return Ok(false.into());
            };
            let needs_doc = r.commit.is_some() || r.mode_changed.is_some();
            if needs_doc {
                *self.pending_up.borrow_mut() = Some(r);
            }
            Ok(needs_doc.into())
        })
    }

    fn OnKeyUp(&self, pic: Ref<ITfContext>, _w: WPARAM, _l: LPARAM) -> Result<BOOL> {
        guard(|| {
            let pending = self.pending_up.borrow_mut().take();
            match (pending, pic.as_ref()) {
                (Some(r), Some(ctx)) => {
                    self.apply(ctx, r);
                    Ok(true.into())
                }
                _ => Ok(false.into()),
            }
        })
    }

    fn OnPreservedKey(&self, _pic: Ref<ITfContext>, _guid: *const GUID) -> Result<BOOL> {
        Ok(false.into())
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    /// 焦點換到別的文件：丟棄組字狀態、隱藏候選窗
    fn OnSetFocus(&self, _focus: Ref<ITfDocumentMgr>, _prev: Ref<ITfDocumentMgr>) -> Result<()> {
        guard(|| {
            self.reset();
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

impl ITfCompositionSink_Impl for TextService_Impl {
    /// App 主動結束組字（例如滑鼠點到別處）：文字已留在文件中，只需同步狀態
    fn OnCompositionTerminated(&self, _ec: u32, _comp: Ref<ITfComposition>) -> Result<()> {
        guard(|| {
            self.reset();
            Ok(())
        })
    }
}
