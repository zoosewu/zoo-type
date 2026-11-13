//! 中文模式下符號鍵的輸出
//!
//! 大千排列中 `, . / ; -` 和數字、字母都是注音鍵，其餘符號鍵（`[ ] \ ' =`）與 Shift + 任何非字母鍵
//! 由輸入法自己決定輸出：
//!
//! - **照鍵盤**（預設）：按鍵上印什麼就出什麼；全形是同一個符號的全形版（Shift + , → ＜、Shift + ' → ＂）
//! - **中文標點**：chewing 的傳統對照（Shift + , → ，、Shift + ' → ；、[ → 「）
//!
//! 半形永遠是全形同一個鍵的半形版，只差寬度。被注音占用的 `, . / ; -` 本身的符號
//! 用 ` 符號選單，或開啟「右 Alt」：右 Alt + 任何鍵都照鍵帽輸出（Shift 照樣作用），
//! 例如右 Alt + 1 → １、右 Alt + w → ｗ、右 Alt + Shift + [ → ｛。

use serde::{Deserialize, Serialize};

use crate::config::Width;
use crate::key::shifted;

/// Shift + 符號鍵的輸出方式
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

/// 中文模式下這個鍵要輸出的符號；`None` 代表不是符號鍵（注音、字母、` 符號選單），交給 chewing
///
/// `c` 是沒按 Shift 時的按鍵字元。
pub fn symbol(c: u8, shift: bool, right_alt: bool, width: Width, style: SymbolStyle) -> Option<char> {
    if right_alt {
        // 右 Alt + 任何鍵：照鍵帽輸出（，。沿用中文的句讀）
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
