use std::cell::RefCell;

use windows::Win32::UI::TextServices::{ITfEditSession, ITfEditSession_Impl};
use windows::core::{Result, implement};

use crate::guard;

type Body = Box<dyn FnOnce(u32) -> Result<()>>;

/// 以閉包實作的 edit session：TSF 只允許在 edit session 內讀寫文件
#[implement(ITfEditSession)]
pub struct EditSession(RefCell<Option<Body>>);

impl EditSession {
    pub fn create(f: impl FnOnce(u32) -> Result<()> + 'static) -> ITfEditSession {
        EditSession(RefCell::new(Some(Box::new(f)))).into()
    }
}

impl ITfEditSession_Impl for EditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        guard(|| match self.0.borrow_mut().take() {
            Some(f) => f(ec),
            None => Ok(()),
        })
    }
}
