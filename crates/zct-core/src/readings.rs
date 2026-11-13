//! 詞 → 讀音反查（libchewing-data 的 `tsi.csv`）：設定程式自動填注音、選字評測共用

use std::collections::HashMap;
use std::path::Path;

/// 詞 → 讀音（同一個詞有多種讀音時取頻率最高的）
#[derive(Debug, Default)]
pub struct Readings {
    map: HashMap<String, (u64, Vec<String>)>,
    max_len: usize,
}

impl Readings {
    /// libchewing-data 的 `tsi.csv`：`詞,頻率,注音 注音 …`
    pub fn load(path: &Path) -> std::io::Result<Readings> {
        Ok(Readings::from_csv(&std::fs::read_to_string(path)?))
    }

    pub fn from_csv(text: &str) -> Readings {
        let mut r = Readings::default();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut it = line.splitn(3, ',');
            let (Some(word), Some(freq), Some(bopo)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let Ok(freq) = freq.trim().parse::<u64>() else {
                continue;
            };
            let syllables: Vec<String> = bopo.split_whitespace().map(str::to_owned).collect();
            if syllables.len() != word.chars().count() {
                continue;
            }
            r.max_len = r.max_len.max(syllables.len());
            match r.map.get(word) {
                Some((f, _)) if *f >= freq => {}
                _ => {
                    r.map.insert(word.to_owned(), (freq, syllables));
                }
            }
        }
        r
    }

    /// 以最長詞優先切詞，取得每個字的讀音；有任何字查不到時回傳 None
    pub fn read(&self, text: &str) -> Option<Vec<String>> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::with_capacity(chars.len());
        let mut i = 0;
        while i < chars.len() {
            let mut found = None;
            for len in (1..=self.max_len.min(chars.len() - i)).rev() {
                let w: String = chars[i..i + len].iter().collect();
                if let Some((_, syl)) = self.map.get(&w) {
                    found = Some(syl.clone());
                    break;
                }
            }
            let syl = found?;
            i += syl.len();
            out.extend(syl);
        }
        Some(out)
    }
}

impl Readings {
    /// 一個詞最常用的讀音，音節之間以空白分隔（例如 `ㄊㄧㄢ ㄑㄧˋ`）
    pub fn reading_of(&self, word: &str) -> Option<String> {
        self.read(word.trim()).map(|s| s.join(" "))
    }
}
