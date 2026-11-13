//! 自訂縮寫：打英文縮寫再按空白，換成中文詞（`tw` → 台灣）
//!
//! 和自訂詞共用詞庫頁與同步檔：讀音欄是英文字母時就是縮寫。
//! 存在學習資料夾的 `abbrev.tsv`，每行 `縮寫 \t 詞`；同一個縮寫可以對應多個詞，先加的優先。

use std::collections::BTreeMap;

/// 讀音欄的內容是不是縮寫（2 個以上英文字母；大小寫不分）
pub fn is_abbrev(reading: &str) -> bool {
    reading.len() >= 2 && reading.bytes().all(|b| b.is_ascii_alphabetic())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Abbrevs(BTreeMap<String, Vec<String>>);

impl Abbrevs {
    pub fn parse(text: &str) -> Abbrevs {
        let mut a = Abbrevs::default();
        for line in text.lines() {
            if let Some((abbrev, word)) = line.split_once('\t') {
                let _ = a.add(abbrev, word);
            }
        }
        a
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (abbrev, words) in &self.0 {
            for w in words {
                out.push_str(&format!("{abbrev}\t{w}\n"));
            }
        }
        out
    }

    /// 加入；回傳是否是新的（已有同樣的縮寫與詞時不變）
    pub fn add(&mut self, abbrev: &str, word: &str) -> Result<bool, String> {
        let (abbrev, word) = (abbrev.trim().to_ascii_lowercase(), word.trim());
        if !is_abbrev(&abbrev) {
            return Err(format!("縮寫只能是 2 個以上的英文字母：{abbrev}"));
        }
        if word.is_empty() || word.contains(['\t', '\n', '\r']) {
            return Err("詞不能是空的".into());
        }
        let words = self.0.entry(abbrev).or_default();
        if words.iter().any(|w| w == word) {
            return Ok(false);
        }
        words.push(word.to_owned());
        Ok(true)
    }

    /// 移除；回傳是否原本就有
    pub fn remove(&mut self, abbrev: &str, word: &str) -> bool {
        let abbrev = abbrev.trim().to_ascii_lowercase();
        let Some(words) = self.0.get_mut(&abbrev) else {
            return false;
        };
        let before = words.len();
        words.retain(|w| w != word.trim());
        let removed = words.len() != before;
        if words.is_empty() {
            self.0.remove(&abbrev);
        }
        removed
    }

    pub fn contains(&self, abbrev: &str, word: &str) -> bool {
        self.0
            .get(&abbrev.to_ascii_lowercase())
            .is_some_and(|w| w.iter().any(|w| w == word))
    }

    /// 縮寫對應的第一個詞
    pub fn lookup(&self, abbrev: &str) -> Option<&str> {
        self.0
            .get(&abbrev.to_ascii_lowercase())
            .and_then(|w| w.first())
            .map(String::as_str)
    }

    /// 全部（縮寫, 詞）
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .flat_map(|(a, ws)| ws.iter().map(move |w| (a.as_str(), w.as_str())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_lookup_remove_roundtrip() {
        let mut a = Abbrevs::default();
        assert_eq!(a.add("TW", "台灣"), Ok(true));
        assert_eq!(a.add("tw", "臺灣"), Ok(true));
        assert_eq!(a.add("tw", "台灣"), Ok(false));
        assert!(a.add("t", "太").is_err(), "至少兩個字母");
        assert!(a.add("su3", "你").is_err(), "只能是字母");
        assert_eq!(a.lookup("Tw"), Some("台灣"), "先加的優先，大小寫不分");
        let b = Abbrevs::parse(&a.to_text());
        assert_eq!(a, b);
        assert!(a.remove("tw", "台灣"));
        assert_eq!(a.lookup("tw"), Some("臺灣"));
        assert!(a.remove("tw", "臺灣"));
        assert_eq!(a.entries().count(), 0);
    }
}
