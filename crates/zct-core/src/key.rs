//! 平台無關的按鍵事件。平台層（TSF / IMK）負責把系統按鍵轉成這裡的型別。

use chewing::input::KeyboardEvent;
use serde::{Deserialize, Serialize};
use chewing::input::keymap::{QWERTY_MAP, map_ascii};
use chewing::input::{keycode, keysym};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    /// 實體按鍵在美式鍵盤上「未按 Shift」時的字元，例如 `a`、`,`、`1`
    Char(u8),
    Space,
    Enter,
    Tab,
    Esc,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Shift(Side),
    Ctrl,
    Alt,
    Meta,
    CapsLock,
    /// F1–F24
    F(u8),
    /// 數字鍵盤：`0`–`9`、`.`、`+`、`-`、`*`、`/`
    Numpad(u8),
    /// 其他輸入法不處理的鍵，一律放行
    Other,
}

impl Key {
    pub fn is_modifier(self) -> bool {
        matches!(
            self,
            Key::Shift(_) | Key::Ctrl | Key::Alt | Key::Meta | Key::CapsLock
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Windows 鍵 / macOS Command
    pub meta: bool,
    pub caps_lock: bool,
    /// 按著的是右 Alt（`alt` 也是 true）；「右 Alt + 注音鍵」打那個鍵本身的符號
    pub right_alt: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
        caps_lock: false,
        right_alt: false,
    };

    pub fn shift() -> Mods {
        Mods {
            shift: true,
            ..Mods::NONE
        }
    }

    pub fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::NONE
        }
    }

    /// 是否有「交給 App 處理」的修飾鍵（P3：Ctrl / Alt / Meta 組合鍵一律放行）
    pub fn has_command_modifier(self) -> bool {
        self.ctrl || self.alt || self.meta
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Mods,
    pub released: bool,
    /// 單調遞增的毫秒時間戳（用於 Shift 單按判定）
    pub time_ms: u64,
}

impl KeyEvent {
    pub fn down(key: Key, mods: Mods, time_ms: u64) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            released: false,
            time_ms,
        }
    }

    pub fn up(key: Key, mods: Mods, time_ms: u64) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            released: true,
            time_ms,
        }
    }
}

/// 美式鍵盤按住 Shift 後的字元
pub(crate) fn shifted(c: u8) -> u8 {
    match c {
        b'a'..=b'z' => c.to_ascii_uppercase(),
        b'1' => b'!',
        b'2' => b'@',
        b'3' => b'#',
        b'4' => b'$',
        b'5' => b'%',
        b'6' => b'^',
        b'7' => b'&',
        b'8' => b'*',
        b'9' => b'(',
        b'0' => b')',
        b'-' => b'_',
        b'=' => b'+',
        b'[' => b'{',
        b']' => b'}',
        b'\\' => b'|',
        b';' => b':',
        b'\'' => b'"',
        b',' => b'<',
        b'.' => b'>',
        b'/' => b'?',
        b'`' => b'~',
        other => other,
    }
}

/// 非字元鍵保留 Shift 狀態：chewing 用 Shift + ← / → 選取加詞範圍
fn special(code: keycode::Keycode, sym: keysym::Keysym, shift: bool) -> KeyboardEvent {
    KeyboardEvent::builder()
        .code(code)
        .ksym(sym)
        .shift_if(shift)
        .build()
}

/// 一個 ASCII 字元的 chewing 按鍵事件（把英文字母、空白塞進組字區用）
pub(crate) fn ascii_event(b: u8) -> KeyboardEvent {
    map_ascii(&QWERTY_MAP, b)
}

/// 轉成 chewing 的按鍵事件；修飾鍵本身與 `Other` 回傳 `None`
pub(crate) fn to_chewing(key: Key, mods: Mods) -> Option<KeyboardEvent> {
    let ev = match key {
        Key::Char(c) => {
            let ascii = if mods.shift { shifted(c) } else { c };
            let ev = map_ascii(&QWERTY_MAP, ascii);
            if ev.is_invalid() {
                return None;
            }
            ev
        }
        Key::Space => map_ascii(&QWERTY_MAP, b' '),
        Key::Enter => special(keycode::KEY_ENTER, keysym::SYM_RETURN, mods.shift),
        Key::Tab => special(keycode::KEY_TAB, keysym::SYM_TAB, mods.shift),
        Key::Esc => special(keycode::KEY_ESC, keysym::SYM_ESC, mods.shift),
        Key::Backspace => special(keycode::KEY_BACKSPACE, keysym::SYM_BACKSPACE, mods.shift),
        Key::Delete => special(keycode::KEY_DELETE, keysym::SYM_DELETE, mods.shift),
        Key::Left => special(keycode::KEY_LEFT, keysym::SYM_LEFT, mods.shift),
        Key::Right => special(keycode::KEY_RIGHT, keysym::SYM_RIGHT, mods.shift),
        Key::Up => special(keycode::KEY_UP, keysym::SYM_UP, mods.shift),
        Key::Down => special(keycode::KEY_DOWN, keysym::SYM_DOWN, mods.shift),
        Key::Home => special(keycode::KEY_HOME, keysym::SYM_HOME, mods.shift),
        Key::End => special(keycode::KEY_END, keysym::SYM_END, mods.shift),
        Key::PageUp => special(keycode::KEY_PAGEUP, keysym::SYM_PAGEUP, mods.shift),
        Key::PageDown => special(keycode::KEY_PAGEDOWN, keysym::SYM_PAGEDOWN, mods.shift),
        Key::Shift(_)
        | Key::Ctrl
        | Key::Alt
        | Key::Meta
        | Key::CapsLock
        | Key::F(_)
        | Key::Numpad(_)
        | Key::Other => {
            return None;
        }
    };
    Some(ev)
}
