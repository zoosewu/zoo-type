//! Emoji 搜尋（需求 4.10）：以 Unicode CLDR 繁體中文註解為關鍵字
//!
//! 用法：先用注音打出關鍵字（例如「開心」），再按 Emoji 快捷鍵，候選窗列出相符的 Emoji。

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

/// 沒有組字時列出的常用 Emoji
pub const COMMON: &[&str] = &[
    "😀", "😂", "🥰", "😍", "😊", "👍", "🙏", "😭", "😅", "🤔", "❤️", "🎉", "🔥", "✨", "👏", "💪",
    "🙌", "😢", "😡", "😎", "🤣", "😉", "😘", "🥺", "👌", "✅", "❌", "⭐", "💯", "🙂",
];

#[derive(Deserialize)]
struct File {
    annotations: Inner,
}

#[derive(Deserialize)]
struct Inner {
    annotations: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    default: Vec<String>,
    #[serde(default)]
    tts: Vec<String>,
}

#[derive(Debug, Default)]
pub struct EmojiIndex {
    /// （Emoji, 名稱, 關鍵字）
    entries: Vec<(String, String, Vec<String>)>,
}

/// CLDR 的鍵不含 U+FE0F；BMP 範圍的符號（例如 ❤）沒有它會顯示成單色文字符號
fn presentation(key: &str) -> String {
    let last = key.chars().last().unwrap_or(' ');
    if (last as u32) < 0x1F000 && last != '\u{FE0F}' && !last.is_ascii() {
        format!("{key}\u{FE0F}")
    } else {
        key.to_owned()
    }
}

/// 只留下 Emoji（略過 →、© 這類一般符號）
fn is_emoji(key: &str) -> bool {
    key.chars().any(|c| {
        let u = c as u32;
        (0x1F000..=0x1FAFF).contains(&u) || (0x2600..=0x27BF).contains(&u) || u == 0x2B50
    })
}

impl EmojiIndex {
    pub fn load(path: &Path) -> std::io::Result<EmojiIndex> {
        let text = std::fs::read_to_string(path)?;
        EmojiIndex::from_json(&text).map_err(std::io::Error::other)
    }

    pub fn from_json(text: &str) -> serde_json::Result<EmojiIndex> {
        let file: File = serde_json::from_str(text)?;
        let entries = file
            .annotations
            .annotations
            .into_iter()
            .filter(|(k, _)| is_emoji(k))
            .map(|(k, e)| {
                let name = e.tts.into_iter().next().unwrap_or_default();
                (presentation(&k), name, e.default)
            })
            .collect();
        Ok(EmojiIndex { entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 依符合程度排序：名稱完全相同 > 關鍵字完全相同 > 名稱包含 > 關鍵字包含
    pub fn search(&self, keyword: &str) -> Vec<String> {
        let keyword = keyword.trim();
        if keyword.is_empty() {
            return COMMON.iter().map(|s| s.to_string()).collect();
        }
        let mut hits: Vec<(u8, usize, &str)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, (emoji, name, words))| {
                let rank = if name == keyword {
                    0
                } else if words.iter().any(|w| w == keyword) {
                    1
                } else if name.contains(keyword) {
                    2
                } else if words.iter().any(|w| w.contains(keyword)) {
                    3
                } else {
                    return None;
                };
                Some((rank, i, emoji.as_str()))
            })
            .collect();
        hits.sort();
        hits.into_iter().map(|(_, _, e)| e.to_owned()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"annotations":{"identity":{},"annotations":{
        "😀":{"default":["微笑","笑臉"],"tts":["笑臉"]},
        "😂":{"default":["哈哈","大笑"],"tts":["感動"]},
        "❤":{"default":["愛心"],"tts":["愛心"]},
        "→":{"default":["箭頭"],"tts":["右箭頭"]}
    }}}"#;

    #[test]
    fn search_ranks_exact_matches_first() {
        let idx = EmojiIndex::from_json(SAMPLE).unwrap();
        assert_eq!(idx.len(), 3, "→ 不是 Emoji");
        assert_eq!(idx.search("笑臉"), vec!["😀"]);
        assert_eq!(idx.search("笑"), vec!["😀", "😂"]);
        assert_eq!(idx.search("愛心"), vec!["❤\u{FE0F}"]);
        assert!(idx.search("不存在").is_empty());
    }

    #[test]
    fn empty_keyword_lists_common() {
        let idx = EmojiIndex::default();
        assert_eq!(idx.search("").len(), COMMON.len());
    }
}
