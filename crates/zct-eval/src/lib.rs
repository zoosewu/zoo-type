//! 選字準確率評測（需求 6.3）：把語料轉成大千按鍵、模擬打字、比對首選結果
//!
//! 讓「亂選字」變成可以量測的數字：每次改動排序或學習機制，都能知道變好還是變壞。

use std::sync::Arc;

use zct_core::config::Config;
pub use zct_core::readings::Readings;
use zct_core::{Engine, Key, KeyEvent, Mods, Session, SessionOptions};

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

/// 一個音節的按鍵；一聲用空白，聲調符號不論寫在哪裡都放最後
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

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// 把句子切成連續的中文片段（標點、英數字略過）
pub fn runs(sentence: &str) -> Vec<String> {
    sentence
        .split(|c: char| !is_cjk(c))
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub runs: usize,
    pub exact_runs: usize,
    pub chars: usize,
    pub correct: usize,
    /// 查不到讀音而略過的片段
    pub skipped: usize,
    /// （原文, 首選結果）
    pub mismatches: Vec<(String, String)>,
}

impl Report {
    pub fn accuracy(&self) -> f64 {
        if self.chars == 0 {
            return 0.0;
        }
        self.correct as f64 / self.chars as f64
    }
}

/// 逐片段模擬打字，比對首選（不送出，所以不會產生學習紀錄影響後面的片段）
pub fn evaluate(engine: Arc<Engine>, config: Config, readings: &Readings, corpus: &str) -> Report {
    let mut session = Session::new(&SessionOptions {
        engine,
        config,
        rule: None,
    })
    .expect("建立 session");
    let mut report = Report::default();
    let mut t = 0u64;
    for line in corpus
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        for run in runs(line) {
            let Some(keys) = readings
                .read(&run)
                .and_then(|syl| syl.iter().map(|s| keys_for(s)).collect::<Option<String>>())
            else {
                report.skipped += 1;
                continue;
            };
            session.reset();
            for b in keys.bytes() {
                let key = if b == b' ' { Key::Space } else { Key::Char(b) };
                t += 20;
                session.process_key(&KeyEvent::down(key, Mods::NONE, t));
                t += 20;
                session.process_key(&KeyEvent::up(key, Mods::NONE, t));
            }
            let got = session.preedit().text;
            let correct = run.chars().zip(got.chars()).filter(|(a, b)| a == b).count();
            report.runs += 1;
            report.chars += run.chars().count();
            report.correct += correct;
            if got == run {
                report.exact_runs += 1;
            } else {
                report.mismatches.push((run, got));
            }
        }
    }
    session.reset();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_for_tones() {
        assert_eq!(keys_for("ㄊㄧㄢ").as_deref(), Some("wu0 "));
        assert_eq!(keys_for("ㄑㄧˋ").as_deref(), Some("fu4"));
        assert_eq!(keys_for("ㄇㄜ˙").as_deref(), Some("ak7"));
        assert_eq!(keys_for("˙ㄇㄜ").as_deref(), Some("ak7"));
    }

    #[test]
    fn readings_pick_most_frequent_and_longest() {
        let r = Readings::from_csv(
            "行,34993,ㄒㄧㄥˊ\n行,4839,ㄏㄤˊ\n銀行,14560,ㄧㄣˊ ㄏㄤˊ\n銀,1,ㄧㄣˊ\n",
        );
        assert_eq!(r.read("行").unwrap(), vec!["ㄒㄧㄥˊ"]);
        assert_eq!(r.read("銀行").unwrap(), vec!["ㄧㄣˊ", "ㄏㄤˊ"]);
        assert!(r.read("火").is_none());
    }

    #[test]
    fn runs_skip_punctuation() {
        assert_eq!(runs("今天，天氣 OK 很好。"), vec!["今天", "天氣", "很好"]);
    }
}
