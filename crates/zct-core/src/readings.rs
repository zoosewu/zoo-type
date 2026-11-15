//! 詞 → 讀音反查（libchewing-data 的 `tsi.csv`）：設定程式自動填注音、重新選字、選字評測共用

use std::collections::HashMap;
use std::path::Path;

/// 詞 → 讀音（同一個詞有多種讀音時依頻率排序）
#[derive(Debug, Default)]
pub struct Readings {
    map: HashMap<String, Vec<(u64, Vec<String>)>>,
    max_len: usize,
}

impl Readings {
    /// libchewing-data 的 `tsi.csv`：`詞,頻率,注音 注音 …`
    pub fn load(path: &Path) -> std::io::Result<Readings> {
        Ok(Readings::from_csv(&std::fs::read_to_string(path)?))
    }

    pub fn from_csv(text: &str) -> Readings {
        Readings::parse(text, |_| true)
    }

    /// 只留下 `text` 裡出現的詞：重新選字時才讀檔，不必把整個詞庫留在記憶體
    pub fn for_text(csv: &str, text: &str) -> Readings {
        Readings::parse(csv, |word| text.contains(word))
    }

    fn parse(text: &str, keep: impl Fn(&str) -> bool) -> Readings {
        let mut r = Readings::default();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut it = line.splitn(3, ',');
            let (Some(word), Some(freq), Some(bopo)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            if !keep(word) {
                continue;
            }
            let Ok(freq) = freq.trim().parse::<u64>() else {
                continue;
            };
            let syllables: Vec<String> = bopo.split_whitespace().map(str::to_owned).collect();
            if syllables.len() != word.chars().count() {
                continue;
            }
            r.max_len = r.max_len.max(syllables.len());
            let list = r.map.entry(word.to_owned()).or_default();
            match list.iter_mut().find(|(_, s)| *s == syllables) {
                Some(e) => e.0 = e.0.max(freq),
                None => list.push((freq, syllables)),
            }
        }
        for list in r.map.values_mut() {
            list.sort_by_key(|a| std::cmp::Reverse(a.0));
        }
        r
    }

    /// 以最長詞優先切詞：每一段（詞, 讀音）；有任何字查不到時回傳 None
    fn segments(&self, text: &str) -> Option<Vec<(String, Vec<String>)>> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let mut found = None;
            for len in (1..=self.max_len.min(chars.len() - i)).rev() {
                let w: String = chars[i..i + len].iter().collect();
                if let Some(list) = self.map.get(&w) {
                    found = Some((w, list[0].1.clone()));
                    break;
                }
            }
            let (w, syl) = found?;
            i += syl.len();
            out.push((w, syl));
        }
        Some(out)
    }

    /// 每個字最常用的讀音；有任何字查不到時回傳 None
    pub fn read(&self, text: &str) -> Option<Vec<String>> {
        Some(
            self.segments(text)?
                .into_iter()
                .flat_map(|(_, s)| s)
                .collect(),
        )
    }

    /// 一段文字可能的讀音，最可能的在前，最多 `max` 種：
    /// 整段是一個詞時它的各種讀音；切詞後最常用的讀音；切出來的單字換成它的其他讀音
    pub fn choices(&self, text: &str, max: usize) -> Vec<Vec<String>> {
        let mut out: Vec<Vec<String>> = Vec::new();
        let mut push = |s: Vec<String>| {
            if out.len() < max && !out.contains(&s) {
                out.push(s);
            }
        };
        if let Some(list) = self.map.get(text) {
            for (_, s) in list {
                push(s.clone());
            }
        }
        let Some(segments) = self.segments(text) else {
            return out;
        };
        let best: Vec<String> = segments.iter().flat_map(|(_, s)| s.clone()).collect();
        push(best.clone());
        // 詞裡的字讀音大多就是那樣；單獨切出來的字才可能是破音字讀錯
        let mut pos = 0;
        for (w, syl) in &segments {
            if syl.len() == 1
                && let Some(list) = self.map.get(w)
            {
                for (_, alt) in list.iter().skip(1) {
                    let mut s = best.clone();
                    s[pos] = alt[0].clone();
                    push(s);
                }
            }
            pos += syl.len();
        }
        out
    }
}

impl Readings {
    /// 一個詞最常用的讀音，音節之間以空白分隔（例如 `ㄊㄧㄢ ㄑㄧˋ`）
    pub fn reading_of(&self, word: &str) -> Option<String> {
        self.read(word.trim()).map(|s| s.join(" "))
    }
}

/// 注音 → 大千按鍵
fn key_of(b: char) -> Option<char> {
    const MAP: &[(char, char)] = &[
        ('ㄅ', '1'),
        ('ㄆ', 'q'),
        ('ㄇ', 'a'),
        ('ㄈ', 'z'),
        ('ㄉ', '2'),
        ('ㄊ', 'w'),
        ('ㄋ', 's'),
        ('ㄌ', 'x'),
        ('ㄍ', 'e'),
        ('ㄎ', 'd'),
        ('ㄏ', 'c'),
        ('ㄐ', 'r'),
        ('ㄑ', 'f'),
        ('ㄒ', 'v'),
        ('ㄓ', '5'),
        ('ㄔ', 't'),
        ('ㄕ', 'g'),
        ('ㄖ', 'b'),
        ('ㄗ', 'y'),
        ('ㄘ', 'h'),
        ('ㄙ', 'n'),
        ('ㄧ', 'u'),
        ('ㄨ', 'j'),
        ('ㄩ', 'm'),
        ('ㄚ', '8'),
        ('ㄛ', 'i'),
        ('ㄜ', 'k'),
        ('ㄝ', ','),
        ('ㄞ', '9'),
        ('ㄟ', 'o'),
        ('ㄠ', 'l'),
        ('ㄡ', '.'),
        ('ㄢ', '0'),
        ('ㄣ', 'p'),
        ('ㄤ', ';'),
        ('ㄥ', '/'),
        ('ㄦ', '-'),
    ];
    MAP.iter().find(|(z, _)| *z == b).map(|(_, k)| *k)
}

/// 一個音節的大千按鍵；一聲用空白，聲調符號不論寫在哪裡都放最後
pub fn keys_for(syllable: &str) -> Option<String> {
    let mut keys = String::new();
    let mut tone = ' ';
    for c in syllable.chars() {
        match c {
            'ˊ' => tone = '6',
            'ˇ' => tone = '3',
            'ˋ' => tone = '4',
            '˙' => tone = '7',
            c => keys.push(key_of(c)?),
        }
    }
    keys.push(tone);
    Some(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "\
# 註解
銀行,500,ㄧㄣˊ ㄏㄤˊ
銀行,20,ㄧㄣˊ ㄒㄧㄥˊ
銀,300,ㄧㄣˊ
行,900,ㄒㄧㄥˊ
行,400,ㄏㄤˊ
走,800,ㄗㄡˇ
";

    #[test]
    fn choices_list_word_readings_then_single_char_alternatives() {
        let r = Readings::from_csv(CSV);
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            r.choices("銀行", 9),
            vec![s(&["ㄧㄣˊ", "ㄏㄤˊ"]), s(&["ㄧㄣˊ", "ㄒㄧㄥˊ"])]
        );
        // 「行」單獨切出來：最常用的讀音在前，再列它的其他讀音
        assert_eq!(
            r.choices("走行", 9),
            vec![s(&["ㄗㄡˇ", "ㄒㄧㄥˊ"]), s(&["ㄗㄡˇ", "ㄏㄤˊ"])]
        );
        assert!(r.choices("天", 9).is_empty(), "查不到的字");
        assert_eq!(r.choices("銀行", 1).len(), 1);
    }

    #[test]
    fn for_text_keeps_only_words_in_the_text() {
        let r = Readings::for_text(CSV, "走");
        assert_eq!(r.map.len(), 1);
        assert_eq!(r.reading_of("走").as_deref(), Some("ㄗㄡˇ"));
    }

    #[test]
    fn keys_for_tones() {
        assert_eq!(keys_for("ㄊㄧㄢ").as_deref(), Some("wu0 "));
        assert_eq!(keys_for("ㄑㄧˋ").as_deref(), Some("fu4"));
        assert_eq!(keys_for("˙ㄉㄜ").as_deref(), Some("2k7"));
    }
}
