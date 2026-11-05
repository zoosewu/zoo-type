//! 單按 Shift 切換中英的判定（需求 4.1）
//!
//! 只有「單獨按下並放開 Shift、中間沒有其他按鍵、按住時間不超過門檻」才算切換。

use crate::config::ShiftToggle;
use crate::key::{Key, KeyEvent, Side};

#[derive(Debug, Clone, Copy)]
struct Pending {
    side: Side,
    since_ms: u64,
}

#[derive(Debug)]
pub struct ShiftTap {
    which: ShiftToggle,
    max_ms: u64,
    pending: Option<Pending>,
}

impl ShiftTap {
    pub fn new(which: ShiftToggle, max_ms: u64) -> ShiftTap {
        ShiftTap {
            which,
            max_ms,
            pending: None,
        }
    }

    fn side_enabled(&self, side: Side) -> bool {
        match self.which {
            ShiftToggle::Both => true,
            ShiftToggle::Left => side == Side::Left,
            ShiftToggle::Right => side == Side::Right,
            ShiftToggle::Off => false,
        }
    }

    /// 觀察每一個按鍵事件；回傳 `true` 表示這次事件完成了一次切換
    pub fn observe(&mut self, ev: &KeyEvent) -> bool {
        match (ev.key, ev.released) {
            (Key::Shift(side), false) => {
                // 按住 Shift 時鍵盤會重複送 down，只記第一次
                let fresh = !matches!(self.pending, Some(p) if p.side == side);
                let other_mods = ev.mods.has_command_modifier();
                if fresh {
                    self.pending = (self.side_enabled(side) && !other_mods).then_some(Pending {
                        side,
                        since_ms: ev.time_ms,
                    });
                }
                false
            }
            (Key::Shift(side), true) => match self.pending.take() {
                Some(p) if p.side == side => ev.time_ms.saturating_sub(p.since_ms) <= self.max_ms,
                _ => false,
            },
            // 放開其他鍵不影響判定（例如先放開 Ctrl 才按 Shift）
            (_, true) => false,
            // Shift 按住期間按了任何其他鍵 → 取消
            (_, false) => {
                self.pending = None;
                false
            }
        }
    }

    /// 失焦等情況下清除狀態
    pub fn reset(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Mods;

    fn tap() -> ShiftTap {
        ShiftTap::new(ShiftToggle::Both, 300)
    }

    fn down(key: Key, t: u64) -> KeyEvent {
        KeyEvent::down(key, Mods::shift(), t)
    }

    fn up(key: Key, t: u64) -> KeyEvent {
        KeyEvent::up(key, Mods::NONE, t)
    }

    const LSHIFT: Key = Key::Shift(Side::Left);
    const RSHIFT: Key = Key::Shift(Side::Right);

    #[test]
    fn quick_tap_toggles() {
        let mut s = tap();
        assert!(!s.observe(&down(LSHIFT, 0)));
        assert!(s.observe(&up(LSHIFT, 100)));
    }

    #[test]
    fn long_hold_does_not_toggle() {
        let mut s = tap();
        s.observe(&down(LSHIFT, 0));
        assert!(!s.observe(&up(LSHIFT, 301)));
    }

    #[test]
    fn auto_repeat_keeps_original_start_time() {
        let mut s = tap();
        s.observe(&down(LSHIFT, 0));
        s.observe(&down(LSHIFT, 250));
        assert!(!s.observe(&up(LSHIFT, 350)));
    }

    #[test]
    fn shift_plus_letter_cancels() {
        let mut s = tap();
        s.observe(&down(LSHIFT, 0));
        s.observe(&down(Key::Char(b'a'), 50));
        s.observe(&up(Key::Char(b'a'), 80));
        assert!(!s.observe(&up(LSHIFT, 100)));
    }

    #[test]
    fn ctrl_shift_does_not_toggle() {
        let mut s = tap();
        s.observe(&KeyEvent::down(
            LSHIFT,
            Mods {
                ctrl: true,
                shift: true,
                ..Mods::NONE
            },
            0,
        ));
        assert!(!s.observe(&up(LSHIFT, 100)));
    }

    #[test]
    fn respects_side_setting() {
        let mut s = ShiftTap::new(ShiftToggle::Left, 300);
        s.observe(&down(RSHIFT, 0));
        assert!(!s.observe(&up(RSHIFT, 100)));
        s.observe(&down(LSHIFT, 200));
        assert!(s.observe(&up(LSHIFT, 300)));
    }

    #[test]
    fn off_never_toggles() {
        let mut s = ShiftTap::new(ShiftToggle::Off, 300);
        s.observe(&down(LSHIFT, 0));
        assert!(!s.observe(&up(LSHIFT, 50)));
    }
}
