//! 快捷鍵：設定檔中寫成 `ctrl+shift+f12`、`shift+space` 這類字串

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::key::{Key, KeyEvent, Mods};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Hotkey {
    /// 修飾鍵必須完全一致：`shift+space` 不會被 `ctrl+shift+space` 觸發
    pub fn matches(&self, ev: &KeyEvent) -> bool {
        !ev.released
            && ev.key == self.key
            && ev.mods.ctrl == self.ctrl
            && ev.mods.alt == self.alt
            && ev.mods.shift == self.shift
            && ev.mods.meta == self.meta
    }

    pub fn mods(&self) -> Mods {
        Mods {
            shift: self.shift,
            ctrl: self.ctrl,
            alt: self.alt,
            meta: self.meta,
            caps_lock: false,
        }
    }
}

fn parse_key(name: &str) -> Option<Key> {
    let key = match name {
        "space" => Key::Space,
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Esc,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        f if f.len() > 1 && f.starts_with('f') && f[1..].bytes().all(|b| b.is_ascii_digit()) => {
            let n: u8 = f[1..].parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            Key::F(n)
        }
        c if c.len() == 1 => {
            let b = c.as_bytes()[0];
            if !b.is_ascii_graphic() {
                return None;
            }
            Key::Char(b)
        }
        _ => return None,
    };
    Some(key)
}

impl std::str::FromStr for Hotkey {
    type Err = String;

    fn from_str(s: &str) -> Result<Hotkey, String> {
        let lower = s.trim().to_ascii_lowercase();
        // 最後一段是主鍵；允許主鍵本身是 `+`（例如 `ctrl++`）
        let (mods, key) = match lower.strip_suffix("++") {
            Some(rest) => (rest, "+"),
            None => lower.rsplit_once('+').unwrap_or(("", lower.as_str())),
        };
        let mut hk = Hotkey {
            key: parse_key(key).ok_or_else(|| format!("無法辨識的按鍵「{key}」（{s}）"))?,
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
        };
        for m in mods.split('+').filter(|m| !m.is_empty()) {
            match m {
                "ctrl" | "control" => hk.ctrl = true,
                "alt" | "option" => hk.alt = true,
                "shift" => hk.shift = true,
                "win" | "cmd" | "meta" | "super" => hk.meta = true,
                other => return Err(format!("無法辨識的修飾鍵「{other}」（{s}）")),
            }
        }
        Ok(hk)
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (on, name) in [
            (self.ctrl, "ctrl+"),
            (self.alt, "alt+"),
            (self.shift, "shift+"),
            (self.meta, "win+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        match self.key {
            Key::Space => f.write_str("space"),
            Key::Enter => f.write_str("enter"),
            Key::Tab => f.write_str("tab"),
            Key::Esc => f.write_str("esc"),
            Key::Backspace => f.write_str("backspace"),
            Key::Delete => f.write_str("delete"),
            Key::Left => f.write_str("left"),
            Key::Right => f.write_str("right"),
            Key::Up => f.write_str("up"),
            Key::Down => f.write_str("down"),
            Key::Home => f.write_str("home"),
            Key::End => f.write_str("end"),
            Key::PageUp => f.write_str("pageup"),
            Key::PageDown => f.write_str("pagedown"),
            Key::F(n) => write!(f, "f{n}"),
            Key::Char(c) => write!(f, "{}", c as char),
            other => write!(f, "{other:?}"),
        }
    }
}

/// 設定檔中的快捷鍵欄位：空字串代表不綁定
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Binding(pub Option<Hotkey>);

impl Binding {
    pub fn key(s: &str) -> Binding {
        Binding(Some(s.parse().expect("內建快捷鍵格式錯誤")))
    }

    pub fn matches(&self, ev: &KeyEvent) -> bool {
        self.0.is_some_and(|h| h.matches(ev))
    }
}

impl Serialize for Binding {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            Some(h) => s.serialize_str(&h.to_string()),
            None => s.serialize_str(""),
        }
    }
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Binding, D::Error> {
        let s = String::deserialize(d)?;
        if s.trim().is_empty() {
            return Ok(Binding(None));
        }
        s.parse().map(|h| Binding(Some(h))).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(s: &str) -> Hotkey {
        s.parse().unwrap()
    }

    #[test]
    fn parses_common_forms() {
        let h = hk("Ctrl+Shift+F12");
        assert_eq!(h.key, Key::F(12));
        assert!(h.ctrl && h.shift && !h.alt && !h.meta);
        assert_eq!(hk("shift+space").key, Key::Space);
        assert_eq!(hk("ctrl+,").key, Key::Char(b','));
        assert_eq!(hk("ctrl++").key, Key::Char(b'+'));
        assert_eq!(hk("win+g").key, Key::Char(b'g'));
    }

    #[test]
    fn rejects_garbage() {
        assert!("ctrl+".parse::<Hotkey>().is_err());
        assert!("hyper+a".parse::<Hotkey>().is_err());
        assert!("f99".parse::<Hotkey>().is_err());
        assert!("ctrl+spacebar".parse::<Hotkey>().is_err());
    }

    #[test]
    fn display_roundtrips() {
        for s in ["ctrl+shift+f12", "shift+space", "alt+g", "ctrl+,"] {
            assert_eq!(hk(s).to_string(), s);
        }
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let h = hk("shift+space");
        let ev = |mods| KeyEvent::down(Key::Space, mods, 0);
        assert!(h.matches(&ev(Mods::shift())));
        assert!(!h.matches(&ev(Mods::NONE)));
        assert!(!h.matches(&ev(Mods { ctrl: true, shift: true, ..Mods::NONE })));
        assert!(!h.matches(&KeyEvent::up(Key::Space, Mods::shift(), 0)));
    }

    #[test]
    fn empty_binding_is_unbound() {
        #[derive(Deserialize)]
        struct T {
            k: Binding,
        }
        let t: T = toml::from_str("k = ''").unwrap();
        assert_eq!(t.k, Binding(None));
    }
}
