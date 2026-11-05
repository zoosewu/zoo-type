//! ZCT 輸入法核心：所有「判斷」都在這裡，平台層（TSF / IMK）只做轉譯。

pub mod config;
pub mod key;
pub mod paths;
pub mod session;
pub mod shift;

pub use config::{Config, InputMode};
pub use key::{Key, KeyEvent, Mods, Side};
pub use session::{CandidatePage, KeyResult, Preedit, Session, SessionOptions};
