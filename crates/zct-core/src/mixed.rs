//! 中英混打（需求 4.5：B + C + E）
//!
//! - C 自動判斷：按空白時，剛打的按鍵是英文單字、且不是合法的注音音節 → 送出英文
//! - B 英文候選：剛完成的音節，其按鍵剛好也是英文單字（例如 `go` = ㄕㄟ）→ 候選窗附上英文
//! - E 盤古之白：中英文之間自動加空格

use std::path::Path;

/// 大千排列：鍵 → 注音的種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Initial,
    Medial,
    Rime,
    Tone,
}

/// 大千排列的聲調鍵（ˇ ˋ ˊ ˙）
pub fn is_tone(c: u8) -> bool {
    kind(c) == Some(Kind::Tone)
}

fn kind(c: u8) -> Option<Kind> {
    Some(match c {
        b'1' | b'q' | b'a' | b'z' | b'2' | b'w' | b's' | b'x' | b'e' | b'd' | b'c' | b'r' | b'f'
        | b'v' | b'5' | b't' | b'g' | b'b' | b'y' | b'h' | b'n' => Kind::Initial,
        b'u' | b'j' | b'm' => Kind::Medial,
        b'8' | b'i' | b'k' | b',' | b'9' | b'o' | b'l' | b'.' | b'0' | b'p' | b';' | b'/' | b'-' => {
            Kind::Rime
        }
        b'3' | b'4' | b'6' | b'7' => Kind::Tone,
        _ => return None,
    })
}

/// 這串按鍵（不含聲調）能不能組成「一個」合法結構的注音音節：聲母? 介音? 韻母?，依序且各至多一個
pub fn is_syllable(keys: &str) -> bool {
    let mut last: Option<Kind> = None;
    for b in keys.bytes() {
        let Some(k) = kind(b) else { return false };
        if k == Kind::Tone || last.is_some_and(|l| k <= l) {
            return false;
        }
        last = Some(k);
    }
    last.is_some()
}

/// 常見的兩個字母英文單字；其他兩字母組合在單字表中多半是罕用字，不拿來自動判斷
const COMMON_2: &[&str] = &[
    "am", "an", "as", "at", "be", "by", "do", "go", "he", "hi", "if", "in", "is", "it", "me", "my",
    "no", "of", "ok", "on", "or", "so", "to", "up", "us", "we",
];

/// 排序過的英文單字表（每行一字），以二分搜尋查詢
#[derive(Debug, Default)]
pub struct WordList {
    text: String,
    /// 每個單字在 `text` 中的起點
    starts: Vec<u32>,
}

impl WordList {
    pub fn load(path: &Path) -> std::io::Result<WordList> {
        Ok(WordList::from_text(std::fs::read_to_string(path)?))
    }

    pub fn from_text(text: String) -> WordList {
        let mut words: Vec<&str> = text.lines().map(str::trim).filter(|w| !w.is_empty()).collect();
        words.sort_unstable();
        words.dedup();
        let mut packed = String::with_capacity(text.len());
        let mut starts = Vec::with_capacity(words.len());
        for w in words {
            starts.push(packed.len() as u32);
            packed.push_str(w);
            packed.push('\n');
        }
        WordList {
            text: packed,
            starts,
        }
    }

    fn word(&self, i: usize) -> &str {
        let s = self.starts[i] as usize;
        let e = self.text[s..].find('\n').map_or(self.text.len(), |n| s + n);
        &self.text[s..e]
    }

    pub fn contains(&self, w: &str) -> bool {
        let w = w.to_ascii_lowercase();
        let (mut lo, mut hi) = (0, self.starts.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.word(mid).cmp(w.as_str()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return true,
            }
        }
        false
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// C：適合自動判斷成英文的單字（3 個字母以上，或常見的兩字母單字）
    pub fn auto_english(&self, keys: &str) -> bool {
        let ok_len = keys.len() >= 3 || COMMON_2.contains(&keys);
        ok_len && keys.bytes().all(|b| b.is_ascii_lowercase()) && !is_syllable(keys) && self.contains(keys)
    }

    /// B：可以當英文候選的單字
    pub fn candidate(&self, keys: &str) -> bool {
        keys.len() >= 2 && keys.bytes().all(|b| b.is_ascii_lowercase()) && self.contains(keys)
    }
}

/// 是否為中日韓文字（決定盤古之白要不要加空格）
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F | 0x3100..=0x312F)
}

/// 是否為英數字
pub fn is_latin(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> WordList {
        WordList::from_text("hello\ngo\nwe\naa\nworld\niphone\nzzz\n".into())
    }

    #[test]
    fn syllable_structure() {
        assert!(is_syllable("su")); // ㄋㄧ
        assert!(is_syllable("go")); // ㄕㄟ
        assert!(is_syllable("cl")); // ㄏㄠ
        assert!(is_syllable("u")); // ㄧ
        assert!(!is_syllable("hello")); // ㄘㄍㄠㄠㄟ
        assert!(!is_syllable("we")); // 兩個聲母
        assert!(!is_syllable("lo")); // 兩個韻母
        assert!(!is_syllable("us")); // 介音在聲母前
        assert!(!is_syllable(""));
    }

    #[test]
    fn lookup() {
        let w = words();
        assert!(w.contains("hello") && w.contains("Hello"));
        assert!(!w.contains("hell"));
        assert_eq!(w.len(), 7);
    }

    #[test]
    fn auto_english_requires_invalid_bopomofo() {
        let w = words();
        assert!(w.auto_english("hello"));
        assert!(w.auto_english("we"), "常見兩字母單字");
        assert!(!w.auto_english("go"), "go 是合法音節 ㄕㄟ，交給 B");
        assert!(!w.auto_english("aa"), "罕用兩字母不自動判斷");
        assert!(!w.auto_english("helo"), "不在單字表");
    }

    #[test]
    fn candidate_allows_valid_syllables() {
        let w = words();
        assert!(w.candidate("go"));
        assert!(!w.candidate("g"));
    }

    #[test]
    fn cjk_detection() {
        assert!(is_cjk('天') && is_cjk('𠀀'));
        assert!(!is_cjk('a') && !is_cjk('，'));
    }
}
