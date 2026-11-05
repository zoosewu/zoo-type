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
    /// 其他輸入法不處理的鍵（F1–F12 等），一律放行
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
}

impl Mods {
    pub const NONE: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
        caps_lock: false,
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

fn special(code: keycode::Keycode, sym: keysym::Keysym) -> KeyboardEvent {
    KeyboardEvent::builder().code(code).ksym(sym).build()
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
        Key::Enter => special(keycode::KEY_ENTER, keysym::SYM_RETURN),
        Key::Tab => special(keycode::KEY_TAB, keysym::SYM_TAB),
        Key::Esc => special(keycode::KEY_ESC, keysym::SYM_ESC),
        Key::Backspace => special(keycode::KEY_BACKSPACE, keysym::SYM_BACKSPACE),
        Key::Delete => special(keycode::KEY_DELETE, keysym::SYM_DELETE),
        Key::Left => special(keycode::KEY_LEFT, keysym::SYM_LEFT),
        Key::Right => special(keycode::KEY_RIGHT, keysym::SYM_RIGHT),
        Key::Up => special(keycode::KEY_UP, keysym::SYM_UP),
        Key::Down => special(keycode::KEY_DOWN, keysym::SYM_DOWN),
        Key::Home => special(keycode::KEY_HOME, keysym::SYM_HOME),
        Key::End => special(keycode::KEY_END, keysym::SYM_END),
        Key::PageUp => special(keycode::KEY_PAGEUP, keysym::SYM_PAGEUP),
        Key::PageDown => special(keycode::KEY_PAGEDOWN, keysym::SYM_PAGEDOWN),
        Key::Shift(_) | Key::Ctrl | Key::Alt | Key::Meta | Key::CapsLock | Key::Other => {
            return None;
        }
    };
    Some(ev)
}
