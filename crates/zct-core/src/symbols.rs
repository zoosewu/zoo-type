//! 中文模式下符號鍵的輸出
//!
//! 大千排列中 `, . / ; -` 和數字、字母都是注音鍵。符號分兩種，打法各自可以設定：
//!
//! - **主要符號**（鍵帽下排：，。；＇／－＝１ｗ…）：`` ` `` 再按那個鍵，或按住右 Alt + 那個鍵
//! - **次要符號**（鍵帽上排：：＂＜＞？＿＋｛…）：Shift + 鍵（預設），
//!   或 `` ` `` / 右 Alt + Shift + 鍵（這時 Shift + 鍵改出中文標點：，。；、「」）
//!
//! `` ` `` 和右 Alt 是同一層的兩種按法：`` ` `` 先按一下，右 Alt 按住。
//! 半形永遠是全形同一個鍵的半形版，只差寬度。

use serde::{Deserialize, Serialize};

use crate::config::Width;
use crate::key::shifted;

/// 主要符號（鍵帽下排）的打法
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimarySymbols {
    /// `` ` `` 再按那個鍵
    #[default]
    Backtick,
    /// 按住右 Alt + 那個鍵
    RightAlt,
    /// 兩種都可以
    Both,
}

impl PrimarySymbols {
    pub fn backtick(self) -> bool {
        matches!(self, PrimarySymbols::Backtick | PrimarySymbols::Both)
    }

    pub fn right_alt(self) -> bool {
        matches!(self, PrimarySymbols::RightAlt | PrimarySymbols::Both)
    }
}

/// 次要符號（鍵帽上排）的打法
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecondarySymbols {
    /// Shift + 鍵
    #[default]
    Shift,
    /// 和主要符號同一層再加 Shift（`` ` `` / 右 Alt + Shift + 鍵）；Shift + 鍵改出中文標點
    Layer,
}

impl SecondarySymbols {
    /// Shift + 符號鍵（不經過 `` ` `` / 右 Alt）的輸出方式
    pub fn shift_style(self) -> SymbolStyle {
        match self {
            SecondarySymbols::Shift => SymbolStyle::Keyboard,
            SecondarySymbols::Layer => SymbolStyle::Chinese,
        }
    }
}

/// Shift + 符號鍵的輸出方式（舊版設定 `shift = "keyboard" | "chinese"`，讀入時轉換）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolStyle {
    /// 按鍵上印的符號
    #[default]
    Keyboard,
    /// 中文標點：Shift + , → ，、Shift + . → 。、Shift + ' → ；、[ → 「
    Chinese,
}

/// ASCII 符號的全形版（U+FF01–FF5E）
fn fullwidth(c: u8) -> char {
    debug_assert!(c.is_ascii_graphic());
    char::from_u32(c as u32 + 0xFEE0).unwrap_or(c as char)
}

/// chewing 傳統的中文標點（全形）；沒有特別對照的用全形版
fn chinese_full(c: u8) -> char {
    match c {
        b'[' => '「',
        b']' => '」',
        b'{' => '『',
        b'}' => '』',
        b'\'' => '、',
        b'<' => '，',
        b'>' => '。',
        b'"' => '；',
        b'_' => '—',
        b'^' => '︿',
        _ => fullwidth(c),
    }
}

/// 中文標點的半形版：，。； 換成 , . ;，其餘就是按鍵本身
fn chinese_half(c: u8) -> char {
    match c {
        b'<' => ',',
        b'>' => '.',
        b'"' => ';',
        _ => c as char,
    }
}

/// 中文模式下這個鍵要輸出的符號；`None` 代表不是符號鍵（注音、字母、`），交給 chewing
///
/// `c` 是沒按 Shift 時的按鍵字元；`layer`：經過 `` ` `` 或右 Alt（照鍵帽輸出）。
pub fn symbol(c: u8, shift: bool, layer: bool, width: Width, style: SymbolStyle) -> Option<char> {
    if layer {
        // ` / 右 Alt + 任何鍵：照鍵帽輸出（，。沿用中文的句讀）
        if !c.is_ascii_graphic() {
            return None;
        }
        let key = if shift { shifted(c) } else { c };
        return Some(match width {
            Width::Half => key as char,
            Width::Full if key == b'.' => '。',
            Width::Full => fullwidth(key),
        });
    }
    let key = if shift {
        if c.is_ascii_alphabetic() || !c.is_ascii_graphic() {
            return None;
        }
        shifted(c)
    } else {
        // 沒按 Shift：只有不是注音、也不是符號選單的鍵
        if !matches!(c, b'[' | b']' | b'\\' | b'\'' | b'=') {
            return None;
        }
        c
    };
    Some(match (style, width) {
        (SymbolStyle::Keyboard, Width::Full) => fullwidth(key),
        (SymbolStyle::Keyboard, Width::Half) => key as char,
        (SymbolStyle::Chinese, Width::Full) => chinese_full(key),
        (SymbolStyle::Chinese, Width::Half) => chinese_half(key),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use SymbolStyle::*;
    use Width::*;

    fn s(c: u8, shift: bool, width: Width, style: SymbolStyle) -> Option<char> {
        symbol(c, shift, false, width, style)
    }

    #[test]
    fn keyboard_style_follows_keycaps() {
        assert_eq!(s(b',', true, Full, Keyboard), Some('＜'));
        assert_eq!(s(b'.', true, Full, Keyboard), Some('＞'));
        assert_eq!(s(b'/', true, Full, Keyboard), Some('？'));
        assert_eq!(s(b';', true, Full, Keyboard), Some('：'));
        assert_eq!(s(b'\'', true, Full, Keyboard), Some('＂'));
        assert_eq!(s(b'1', true, Full, Keyboard), Some('！'));
        assert_eq!(s(b'[', false, Full, Keyboard), Some('［'));
        assert_eq!(s(b'\'', false, Full, Keyboard), Some('＇'));
        assert_eq!(s(b'`', true, Full, Keyboard), Some('～'));
    }

    #[test]
    fn half_is_same_key_in_half_width() {
        for style in [Keyboard, Chinese] {
            assert_eq!(s(b',', true, Half, style), Some(if style == Keyboard { '<' } else { ',' }));
        }
        assert_eq!(s(b'\'', true, Half, Keyboard), Some('"'));
        assert_eq!(s(b'\'', true, Half, Chinese), Some(';'));
        assert_eq!(s(b'.', true, Half, Chinese), Some('.'));
        assert_eq!(s(b';', true, Half, Chinese), Some(':'));
        assert_eq!(s(b'[', false, Half, Chinese), Some('['));
    }

    #[test]
    fn chinese_style_is_chewing_table() {
        assert_eq!(s(b',', true, Full, Chinese), Some('，'));
        assert_eq!(s(b'.', true, Full, Chinese), Some('。'));
        assert_eq!(s(b'\'', true, Full, Chinese), Some('；'));
        assert_eq!(s(b';', true, Full, Chinese), Some('：'));
        assert_eq!(s(b'[', false, Full, Chinese), Some('「'));
        assert_eq!(s(b'\'', false, Full, Chinese), Some('、'));
    }

    #[test]
    fn bopomofo_letters_and_menu_go_to_chewing() {
        for c in *b",./;-1a`" {
            assert_eq!(s(c, false, Full, Keyboard), None, "{}", c as char);
        }
        assert_eq!(s(b'a', true, Full, Keyboard), None, "Shift + 字母是英文");
    }

    #[test]
    fn right_alt_follows_keycaps_for_every_key() {
        let ra = |c, shift, w| symbol(c, shift, true, w, Chinese);
        assert_eq!(ra(b',', false, Full), Some('，'));
        assert_eq!(ra(b'.', false, Full), Some('。'));
        assert_eq!(ra(b';', false, Full), Some('；'));
        assert_eq!(ra(b'/', false, Full), Some('／'));
        assert_eq!(ra(b'-', false, Full), Some('－'));
        assert_eq!(ra(b'=', false, Full), Some('＝'));
        assert_eq!(ra(b'1', false, Full), Some('１'));
        assert_eq!(ra(b'w', false, Full), Some('ｗ'));
        assert_eq!(ra(b'w', true, Full), Some('Ｗ'));
        assert_eq!(ra(b'[', true, Full), Some('｛'));
        assert_eq!(ra(b'-', true, Full), Some('＿'));
        assert_eq!(ra(b'=', true, Full), Some('＋'));
        assert_eq!(ra(b',', false, Half), Some(','));
        assert_eq!(ra(b'1', true, Half), Some('!'));
    }
}
