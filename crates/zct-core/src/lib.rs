//! ZCT 輸入法核心：所有「判斷」都在這裡，平台層（TSF / IMK）只做轉譯。

pub mod apps;
pub mod config;
pub mod emoji;
pub mod engine;
pub mod hotkey;
pub mod key;
pub mod mixed;
pub mod paths;
pub mod session;
pub mod shift;
pub mod sync;

pub use apps::{AppRule, AppRules};
pub use engine::{DayStats, Engine, PhraseAction, PhraseEntry};
pub use config::{Config, GameMode, InputMode, Output, Width};
pub use key::{Key, KeyEvent, Mods, Side};
pub use session::{CandidatePage, KeyResult, Preedit, Session, SessionOptions};
