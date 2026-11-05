//! TSF DLL ↔ 服務程序之間的訊息（架構文件第 4 節）

use serde::{Deserialize, Serialize};
use zct_core::{KeyEvent, KeyResult};

#[cfg(windows)]
pub mod pipe;

/// 協定版本；DLL 與服務程序版本不一致時拒絕連線，避免解碼錯亂
pub const PROTOCOL_VERSION: u32 = 2;

/// 單一訊息上限（候選字頁面遠小於此）
pub const MAX_MESSAGE: usize = 64 * 1024;

/// 螢幕座標（實體像素）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// 連線後第一則訊息
    Hello {
        version: u32,
        pid: u32,
        /// 執行檔名稱（App 規則表用）
        exe: String,
    },
    /// `OnTestKeyDown`：回 `Eat`
    TestKeyDown(KeyEvent),
    /// `OnKeyDown` / `OnTestKeyUp`：回 `Key`
    Key(KeyEvent),
    /// 游標矩形；`None` 表示取不到，由服務程序沿用上一次的位置
    Caret(Option<Rect>),
    /// 失焦、組字被 App 中止：丟棄組字狀態並隱藏候選窗
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Ok,
    Eat(bool),
    Key(KeyResult),
    Error(String),
}

#[derive(Debug)]
pub enum Error {
    Encode(postcard::Error),
    Decode(postcard::Error),
    TooLarge(usize),
    Timeout,
    #[cfg(windows)]
    Os(windows::core::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Encode(e) => write!(f, "編碼失敗：{e}"),
            Error::Decode(e) => write!(f, "解碼失敗：{e}"),
            Error::TooLarge(n) => write!(f, "訊息過大：{n} bytes"),
            Error::Timeout => write!(f, "逾時"),
            #[cfg(windows)]
            Error::Os(e) => write!(f, "系統錯誤：{e}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(windows)]
impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Error::Os(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = postcard::to_stdvec(value).map_err(Error::Encode)?;
    if bytes.len() > MAX_MESSAGE {
        return Err(Error::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T> {
    postcard::from_bytes(bytes).map_err(Error::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zct_core::{Key, Mods};

    #[test]
    fn roundtrip() {
        let req = Request::Key(KeyEvent::down(Key::Char(b'a'), Mods::shift(), 42));
        let back: Request = decode(&encode(&req).unwrap()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn key_message_is_small() {
        let req = Request::Key(KeyEvent::down(Key::Char(b'a'), Mods::NONE, u64::MAX));
        assert!(encode(&req).unwrap().len() < 32);
    }
}
