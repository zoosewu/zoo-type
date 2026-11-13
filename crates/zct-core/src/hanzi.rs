//! 繁簡字判斷：只屬於簡體的字與它對應的繁體字
//!
//! 資料來自 OpenCC 的 STCharacters（簡 → 繁字表）：對應清單裡沒有自己的字才算「只屬於簡體」，
//! 例如 `机 → 機`；`后 → 後 后`、`种 → 種 种` 這類繁體也正常使用的字不算。
//! 台灣詞庫的使用量另外由 [`crate::Engine::simplified_only`] 判斷（秘 / 祕 這類兩種都常用的寫法）。

use std::collections::HashMap;
use std::sync::OnceLock;

fn table() -> &'static HashMap<char, char> {
    static TABLE: OnceLock<HashMap<char, char>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("../data/opencc-STCharacters.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (key, values) = l.split_once('\t')?;
                let mut kc = key.chars();
                let (k, None) = (kc.next()?, kc.next()) else {
                    return None;
                };
                let targets: Vec<char> =
                    values.split(' ').filter_map(|v| v.chars().next()).collect();
                if targets.contains(&k) {
                    return None;
                }
                Some((k, *targets.first()?))
            })
            .collect()
    })
}

/// 只屬於簡體的字 → 對應的繁體字；不是的話回傳 None
pub fn traditional_of(c: char) -> Option<char> {
    table().get(&c).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplified_only_chars() {
        assert_eq!(traditional_of('机'), Some('機'));
        assert_eq!(traditional_of('烛'), Some('燭'));
        // 繁體也正常使用的字
        for c in ['后', '种', '里', '只', '干', '才', '機', '天'] {
            assert_eq!(traditional_of(c), None, "{c}");
        }
    }
}
