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

/// 右 Alt 照鍵帽輸出：TSF 一般的按鍵通道不會把 Alt 組合鍵交給輸入法，
/// 所以把「右 Alt（+ Shift）+ 可以打字的鍵」登記成保留按鍵，由 `OnPreservedKey` 收到。
const RIGHT_ALT_VKS: [VIRTUAL_KEY; 11] = [
    VK_OEM_COMMA,
    VK_OEM_PERIOD,
    VK_OEM_2,
    VK_OEM_1,
    VK_OEM_7,
    VK_OEM_4,
    VK_OEM_6,
    VK_OEM_5,
    VK_OEM_MINUS,
    VK_OEM_PLUS,
    VK_OEM_3,
];

/// 保留按鍵：(GUID, 虛擬鍵碼, 是否含 Shift, 按鍵字元)；GUID 由按鍵與 Shift 推得，每次都一樣
pub fn right_alt_keys() -> impl Iterator<Item = (windows::core::GUID, VIRTUAL_KEY, bool, u8)> {
    let vks = RIGHT_ALT_VKS
        .into_iter()
        .chain((0x30..=0x39).map(VIRTUAL_KEY))
        .chain((0x41..=0x5A).map(VIRTUAL_KEY));
    vks.flat_map(|vk| {
        let Key::Char(c) = key(WPARAM(vk.0 as usize), LPARAM(0)) else {
            unreachable!("清單裡都是字元鍵")
        };
        [false, true].map(move |shift| {
            let guid = windows::core::GUID::from_u128(
                0x3c0e5a71_9d42_4b6e_8f13_a2b4c6d80000 | (shift as u128) << 8 | vk.0 as u128,
            );
            (guid, vk, shift, c)
        })
    })
}

/// 保留按鍵（右 Alt 符號）對應的按下事件
pub fn right_alt_event(c: u8, shift: bool) -> KeyEvent {
    KeyEvent {
        key: Key::Char(c),
        mods: Mods {
            alt: true,
            right_alt: true,
            shift,
            ..Mods::NONE
        },
        released: false,
        time_ms: unsafe { GetTickCount64() },
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
