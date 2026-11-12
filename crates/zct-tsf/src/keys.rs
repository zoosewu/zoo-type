//! Windows 虛擬鍵碼 → zct-core 的按鍵事件

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use zct_core::{Key, KeyEvent, Mods, Side};

/// 右 Shift 的掃描碼
const SCAN_RSHIFT: u32 = 0x36;

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

pub fn mods() -> Mods {
    Mods {
        shift: down(VK_SHIFT),
        ctrl: down(VK_CONTROL),
        alt: down(VK_MENU),
        meta: down(VK_LWIN) || down(VK_RWIN),
        caps_lock: unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0,
        right_alt: down(VK_RMENU),
    }
}

pub fn key(wparam: WPARAM, lparam: LPARAM) -> Key {
    let vk = VIRTUAL_KEY(wparam.0 as u16);
    let scan = ((lparam.0 >> 16) & 0xFF) as u32;
    match vk {
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => {
            if vk == VK_RSHIFT || scan == SCAN_RSHIFT {
                Key::Shift(Side::Right)
            } else {
                Key::Shift(Side::Left)
            }
        }
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => Key::Ctrl,
        VK_MENU | VK_LMENU | VK_RMENU => Key::Alt,
        VK_LWIN | VK_RWIN => Key::Meta,
        VK_CAPITAL => Key::CapsLock,
        VK_SPACE => Key::Space,
        VK_RETURN => Key::Enter,
        VK_TAB => Key::Tab,
        VK_ESCAPE => Key::Esc,
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        VK_OEM_COMMA => Key::Char(b','),
        VK_OEM_PERIOD => Key::Char(b'.'),
        VK_OEM_2 => Key::Char(b'/'),
        VK_OEM_1 => Key::Char(b';'),
        VK_OEM_7 => Key::Char(b'\''),
        VK_OEM_4 => Key::Char(b'['),
        VK_OEM_6 => Key::Char(b']'),
        VK_OEM_5 => Key::Char(b'\\'),
        VK_OEM_MINUS => Key::Char(b'-'),
        VK_OEM_PLUS => Key::Char(b'='),
        VK_OEM_3 => Key::Char(b'`'),
        VK_DECIMAL => Key::Numpad(b'.'),
        VK_ADD => Key::Numpad(b'+'),
        VK_SUBTRACT => Key::Numpad(b'-'),
        VK_MULTIPLY => Key::Numpad(b'*'),
        VK_DIVIDE => Key::Numpad(b'/'),
        VIRTUAL_KEY(v @ 0x60..=0x69) => Key::Numpad(b'0' + (v - 0x60) as u8),
        VIRTUAL_KEY(v @ 0x70..=0x87) => Key::F((v - 0x70 + 1) as u8),
        VIRTUAL_KEY(v @ 0x30..=0x39) => Key::Char(v as u8),
        VIRTUAL_KEY(v @ 0x41..=0x5A) => Key::Char((v as u8).to_ascii_lowercase()),
        _ => Key::Other,
    }
}

pub fn event(wparam: WPARAM, lparam: LPARAM, released: bool) -> KeyEvent {
    KeyEvent {
        key: key(wparam, lparam),
        mods: mods(),
        released,
        time_ms: unsafe { GetTickCount64() },
    }
}
