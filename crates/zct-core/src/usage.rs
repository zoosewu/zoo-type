//! 詞庫 tsi.csv 的使用量統計：單字在各讀音下的常用程度、詞的少見讀音
//!
//! chewing 的語言模型只看字，不看讀音：「那些」比「哪些」常見，打「ㄋㄚˇ ㄒㄧㄝ」時就選了「那些」，
//! 雖然詞庫記錄「那些」大多讀「ㄋㄚˋ ㄒㄧㄝ」。[`Usage::reading_penalty`] 補上這個資訊。

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default)]
pub struct Usage {
    /// （字, 讀音）→ 含有這個字這個讀音的詞的頻率加總
    by_reading: HashMap<(char, String), u64>,
    /// 字 → 所有讀音合計
    total: HashMap<char, u64>,
    /// （詞, 讀音）→ 這個讀音比這個詞最常見的讀音少見多少（log10，> 0）；只記少見的讀音
    rare_reading: HashMap<(Box<str>, Box<str>), f32>,
    /// 去掉聲調的單一讀音 → 這個讀音（各聲調合計）的字，常用的在前
    by_toneless: HashMap<Box<str>, Vec<char>>,
}

/// 去掉聲調符號（ˊ ˇ ˋ ˙；一聲本來就沒有符號）
fn toneless(reading: &str) -> String {
    reading
        .chars()
        .filter(|c| !matches!(c, 'ˊ' | 'ˇ' | 'ˋ' | '˙'))
        .collect()
}

impl Usage {
    /// 讀 libchewing-data 的 tsi.csv（`詞,頻率,注音`）；沒有檔案時全部是空的
    pub fn load(path: &Path) -> Usage {
        match std::fs::read_to_string(path) {
            Ok(text) => Usage::parse(&text),
            Err(_) => {
                log::warn!("沒有 tsi.csv，單字候選維持 chewing 的順序");
                Usage::default()
            }
        }
    }

    pub fn parse(text: &str) -> Usage {
        let mut usage = Usage::default();
        let mut words: HashMap<&str, Vec<(&str, u64)>> = HashMap::new();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut it = line.split(',');
            let (Some(word), Some(freq), Some(bopomofo)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let Ok(freq) = freq.trim().parse::<u64>() else {
                continue;
            };
            if bopomofo.split(' ').count() != word.chars().count() {
                continue;
            }
            for (c, r) in word.chars().zip(bopomofo.split(' ')) {
                *usage.by_reading.entry((c, r.to_owned())).or_insert(0) += freq;
                *usage.total.entry(c).or_insert(0) += freq;
            }
            let readings = words.entry(word).or_default();
            match readings.iter_mut().find(|(r, _)| *r == bopomofo) {
                Some((_, f)) => *f += freq,
                None => readings.push((bopomofo, freq)),
            }
        }
        let mut toneless_usage: HashMap<String, HashMap<char, u64>> = HashMap::new();
        for ((c, r), freq) in &usage.by_reading {
            *toneless_usage
                .entry(toneless(r))
                .or_default()
                .entry(*c)
                .or_insert(0) += freq;
        }
        usage.by_toneless = toneless_usage
            .into_iter()
            .map(|(reading, chars)| {
                let mut chars: Vec<(char, u64)> = chars.into_iter().collect();
                chars.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                (reading.into(), chars.into_iter().map(|(c, _)| c).collect())
            })
            .collect();
        for (word, readings) in words.into_iter().filter(|(_, r)| r.len() > 1) {
            let top = readings.iter().map(|(_, f)| *f).max().unwrap_or(0).max(1) as f64;
            for (reading, freq) in readings {
                let gap = (top / freq.max(1) as f64).log10();
                if gap > 0.0 {
                    usage
                        .rare_reading
                        .insert((word.into(), reading.into()), gap as f32);
                }
            }
        }
        usage
    }

    /// 單字在這個讀音下的常用程度
    pub fn char_usage(&self, c: char, reading: &str) -> u64 {
        self.by_reading
            .get(&(c, reading.to_owned()))
            .copied()
            .unwrap_or(0)
    }

    /// 字所有讀音合計的常用程度
    pub fn total(&self, c: char) -> u64 {
        self.total.get(&c).copied().unwrap_or(0)
    }

    /// 這個注音（各聲調）的字，常用的在前；`syllable` 可以帶聲調，比對時去掉
    pub fn chars_for(&self, syllable: &str) -> &[char] {
        self.by_toneless
            .get(toneless(syllable).as_str())
            .map_or(&[], Vec::as_slice)
    }

    /// 這個讀音對這個詞來說少見多少（log10，常見讀音或查不到是 0）。
    /// `reading`：tsi.csv 的格式，音節用空白隔開
    pub fn reading_penalty(&self, word: &str, reading: &str) -> f64 {
        self.rare_reading
            .get(&(Box::from(word), Box::from(reading)))
            .map_or(0.0, |&g| f64::from(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "\
哪些,5893,ㄋㄚˇ ㄒㄧㄝ
那些,1536,ㄋㄚˇ ㄒㄧㄝ
那些,5893,ㄋㄚˋ ㄒㄧㄝ
天氣,100,ㄊㄧㄢ ㄑㄧˋ
";

    #[test]
    fn rare_readings_get_a_penalty() {
        let u = Usage::parse(CSV);
        let p = u.reading_penalty("那些", "ㄋㄚˇ ㄒㄧㄝ");
        assert!((p - (5893.0f64 / 1536.0).log10()).abs() < 1e-3, "{p}");
        assert_eq!(u.reading_penalty("那些", "ㄋㄚˋ ㄒㄧㄝ"), 0.0);
        assert_eq!(u.reading_penalty("哪些", "ㄋㄚˇ ㄒㄧㄝ"), 0.0);
        assert_eq!(u.reading_penalty("天氣", "ㄊㄧㄢ ㄑㄧˋ"), 0.0);
    }

    #[test]
    fn chars_for_lists_all_tones_by_usage() {
        let u = Usage::parse(CSV);
        // ㄋㄚˇ（那 1536）＋ ㄋㄚˋ（那 5893）合計勝過 哪 5893
        assert_eq!(u.chars_for("ㄋㄚ"), &['那', '哪']);
        assert_eq!(u.chars_for("ㄋㄚˋ"), &['那', '哪'], "帶聲調也一樣");
        assert!(u.chars_for("ㄅㄨ").is_empty());
    }

    #[test]
    fn char_usage_sums_words() {
        let u = Usage::parse(CSV);
        assert_eq!(u.char_usage('那', "ㄋㄚˇ"), 1536);
        assert_eq!(u.total('那'), 1536 + 5893);
    }
}
