//! TSF DLL ↔ 服務程序之間的訊息（架構文件第 4 節）

use serde::{Deserialize, Serialize};
use zct_core::hotkey::Binding;
use zct_core::{DayStats, KeyEvent, KeyResult, PhraseAction, PhraseEntry};

#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
pub mod secret;

/// 協定版本；DLL 與服務程序版本不一致時拒絕連線，避免解碼錯亂
/// 12：程式改名為 zoo-type-*（舊版服務程序不能讓位給不同檔名的新版，改走協定不同的唯讀模式）
pub const PROTOCOL_VERSION: u32 = 17;

/// 單一訊息上限（詞庫清單等管理用回應可能較大）
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// 緩衝區初始大小與管道緩衝大小；按鍵訊息遠小於此，較大的回應會自動擴充緩衝區
pub const INITIAL_BUFFER: usize = 64 * 1024;

/// 輸入法圖示右鍵選單的指令
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    OpenSettings,
    ToggleGame,
    Restart,
    /// 設定程式要變更資料位置：立即存檔，之後不再寫入（搬移完成後再送 Restart）
    PrepareMove,
    /// 安裝已下載的新版（啟動以 SYSTEM 執行的更新工作）
    InstallUpdate,
}

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
    /// 游標矩形；`None` 表示取不到。`fresh = false`：App 這次沒回報位置
    /// （例如 VS Code 剛輸出完還在排版），`rect` 是 DLL 沿用的舊位置或系統游標
    Caret { rect: Option<Rect>, fresh: bool },
    /// 失焦、組字被 App 中止：丟棄組字狀態並隱藏候選窗
    Reset,
    /// 輸入焦點換到另一個文件：同 Reset，並依 `on_focus` 設定重設中英狀態
    Focus,

    // ---- 管理用（設定程式）：服務程序是學習資料唯一的擁有者 ----
    /// 列出使用者詞庫 → `Phrases`
    ListPhrases,
    /// 加入自訂詞 → `Ok` / `Error`
    AddPhrase { word: String, bopomofo: String },
    /// 封鎖 / 解除封鎖 / 取消固頂 → `Ok` / `Error`
    Phrase {
        word: String,
        bopomofo: String,
        action: PhraseAction,
    },
    /// 每日統計 → `Stats`
    Stats,
    /// 備份清單（新的在前）→ `Backups`
    Backups,
    /// 從備份還原；成功後服務程序會結束，下次按鍵時重新啟動並載入 → `Ok` / `Error`
    Restore { day: String },
    /// 立即在背景同步；結果寫在 `local/sync-status.txt` → `Ok`
    SyncNow,
    /// 輸入法圖示顯示用：目前中英、繁簡與遊戲模式 → `Status`
    Status,
    /// 點輸入法圖示切換中英 → `Key`（組字中會先送出）
    ToggleMode,
    /// 輸入法圖示右鍵選單 → `Ok`
    Command(Command),
}

// 每次呼叫只傳一個回應，大小差異不影響效能；不裝箱讓序列化格式保持單純
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Ok,
    /// 回應 `Hello`：告訴 DLL 這個 App 要做哪些啟發式偵測
    Welcome {
        /// 舊式程式（TSF transitory context）+ 沒有系統游標 + 視窗蓋滿螢幕 → 視為不在輸入狀態
        detect_legacy: bool,
        /// 有程式獨佔全螢幕 → 視為不在輸入狀態
        detect_fullscreen: bool,
    },
    Eat(bool),
    Key(KeyResult),
    Phrases(Vec<PhraseEntry>),
    Stats(Vec<DayStats>),
    Backups(Vec<String>),
    Status {
        chinese: bool,
        simplified: bool,
        game: bool,
        /// 「右 Alt 照鍵帽輸出」開啟：DLL 要向 TSF 登記保留按鍵
        right_alt: bool,
        /// 已下載、可以安裝的新版版號（輸入法圖示選單顯示「更新到 vX」）
        update: Option<String>,
        /// 遊戲模式鎖定英文：除了這些快捷鍵，DLL 直接放行按鍵、不再詢問服務程序
        passthrough: Option<Vec<Binding>>,
    },
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
