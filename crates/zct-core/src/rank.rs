//! 包住 chewing 的轉換引擎，替前幾名候選句重新排序
//!
//! chewing 用「詞」的 bigram 算出前 10 名候選句（cost 越低越好，log10 尺度），這裡再調整：
//! - 讀音：詞用了少見的讀音就加分（[`crate::usage::Usage::reading_penalty`]），
//!   打「ㄋㄚˇ ㄒㄧㄝ」是「哪些」、「ㄋㄚˋ ㄒㄧㄝ」是「那些」
//! - T0（需求 4.13）：減去「字」n-gram 的分數（log10，越高越通順）× 權重
//!
//! 使用者選過的字（組字區的選取）chewing 已經放進每個候選句，重新排序不會改掉它們；
//! 使用者學過、設為優先的詞也一定保留。

use std::sync::{Arc, OnceLock};

use chewing::conversion::{
    ChewingEngine, Composition, ConversionEngine, Decoder, Interval, LatticeBuilder, Outcome,
};
use chewing::dictionary::{CompositeDict, LookupStrategy, StringTable};
use chewing::lm::{StaticDict, StaticLm};
use chewing::user::{HistoryDict, SessionHistoryDict, UserDict};

use crate::ngram::NgramModel;
use crate::usage::Usage;

/// 字 n-gram 分數的權重；與門檻一起用評測調出來（2026-11-16：自寫口語不變差、維基保留集錯字少約 25%）
pub const DEFAULT_WEIGHT: f64 = 1.0;
/// T0 換掉 chewing 首選需要的差距（log10）
pub const DEFAULT_MARGIN: f64 = 0.5;
/// 少見讀音的扣分權重（2026-11-17 評測：自寫口語 99.42% → 99.50%、維基 91.08% → 91.28%）
pub const READING_WEIGHT: f64 = 2.0;
/// 開著 T0 時：字 n-gram 也只看字、不看讀音，讀音要加重才平衡（維基 93.29% → 93.39%，自寫不變）
pub const READING_WEIGHT_T0: f64 = 4.0;

/// T0 模型與參數
#[derive(Debug, Clone)]
pub struct T0 {
    pub model: Arc<NgramModel>,
    pub weight: f64,
    pub margin: f64,
}

#[derive(Debug)]
pub struct RankEngine {
    inner: ChewingEngine,
    /// 使用者學過、設為優先的詞（加權高於一般）一定保留
    user_dict: UserDict,
    /// 背景載入中時是空的：先不調整讀音，不讓按鍵等檔案
    usage: Arc<OnceLock<Usage>>,
    reading_weight: f64,
    t0: Option<T0>,
}

/// 組出和 chewing `EditorBuilder` 相同的轉換引擎所需的資料
pub struct Parts {
    pub string_table: StringTable,
    pub static_dict: StaticDict,
    pub rare_dict: StaticDict,
    pub history_dict: HistoryDict,
    pub user_dict: UserDict,
    pub lm: StaticLm,
}

impl RankEngine {
    pub fn new(
        parts: Parts,
        usage: Arc<OnceLock<Usage>>,
        reading_weight: f64,
        t0: Option<T0>,
    ) -> RankEngine {
        let session = SessionHistoryDict::new(parts.string_table.clone());
        let dict = CompositeDict::new(
            parts.static_dict,
            parts.rare_dict,
            parts.history_dict.clone(),
            session,
            parts.user_dict.clone(),
        );
        RankEngine {
            user_dict: parts.user_dict,
            inner: ChewingEngine {
                word_lattice_builder: LatticeBuilder {
                    dict,
                    lookup_strategy: LookupStrategy::Standard,
                },
                decoder: Decoder {
                    lm: parts.lm,
                    hist: parts.history_dict,
                    lambda: Decoder::LAMBDA,
                },
                string_table: parts.string_table,
            },
            usage,
            reading_weight,
            t0,
        }
    }

    /// 區間的讀音（tsi.csv 格式，音節用空白隔開）；有不是音節的符號時 None
    fn reading(comp: &Composition, it: &Interval) -> Option<String> {
        let span = it.src_span.start as usize..it.src_span.end as usize;
        let syl = comp.symbols().get(span)?;
        let mut out = String::new();
        for s in syl {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&s.to_syllable()?.to_string());
        }
        Some(out)
    }

    /// 候選句用了多少少見讀音
    fn reading_penalty(&self, usage: &Usage, comp: &Composition, o: &Outcome) -> f64 {
        o.intervals
            .iter()
            .filter_map(|it| Some(usage.reading_penalty(&it.text, &Self::reading(comp, it)?)))
            .sum()
    }

    /// 候選句裡使用者學過或設為優先的詞：（輸出位置起點, 終點, 詞）
    fn user_words(&self, comp: &Composition, o: &Outcome) -> Vec<(usize, usize, Box<str>)> {
        o.intervals
            .iter()
            .filter(|it| it.is_phrase)
            .filter(|it| {
                let span = it.src_span.start as usize..it.src_span.end as usize;
                let Some(syl) = comp.symbols().get(span).and_then(|s| {
                    s.iter()
                        .map(|s| s.to_syllable())
                        .collect::<Option<Vec<_>>>()
                }) else {
                    return false;
                };
                self.user_dict
                    .lookup(&syl, LookupStrategy::Standard)
                    .into_iter()
                    .any(|(wid, boost)| {
                        boost > crate::engine::PhraseEntry::NORMAL
                            && self.user_dict.get_text(wid).as_deref() == Some(&*it.text)
                    })
            })
            .map(|it| (it.start, it.end, it.text.clone()))
            .collect()
    }
}

fn text_of(o: &Outcome) -> String {
    o.intervals.iter().map(|i| &*i.text).collect()
}

/// 候選句在那些位置是不是同樣的詞
fn keeps(o: &Outcome, words: &[(usize, usize, Box<str>)]) -> bool {
    let chars: Vec<char> = text_of(o).chars().collect();
    words.iter().all(|(s, e, w)| {
        chars
            .get(*s..*e)
            .is_some_and(|c| c.iter().copied().eq(w.chars()))
    })
}

impl ConversionEngine for RankEngine {
    fn convert<'a>(&'a self, comp: &'a Composition) -> Vec<Outcome> {
        let mut outcomes = ConversionEngine::convert(&self.inner, comp);
        let usage = self.usage.get();
        if outcomes.len() < 2 || (usage.is_none() && self.t0.is_none()) {
            return outcomes;
        }
        let mut top_rare = false;
        let mut scored: Vec<(f64, Outcome)> = outcomes
            .drain(..)
            .enumerate()
            .map(|(i, o)| {
                let mut s = o.cost;
                if let Some(u) = usage {
                    let p = self.reading_penalty(u, comp, &o);
                    top_rare |= i == 0 && p > 0.0;
                    s += self.reading_weight * p;
                }
                if let Some(t0) = &self.t0 {
                    s -= t0.weight * t0.model.score(&text_of(&o));
                }
                (s, o)
            })
            .collect();
        // 使用者學過、設為優先的詞一定保留：沒保留它們的候選句排到最後
        let protected = self.user_words(comp, &scored[0].1);
        if !protected.is_empty() {
            for (score, o) in scored.iter_mut().skip(1) {
                if !keeps(o, &protected) {
                    *score = f64::INFINITY;
                }
            }
        }
        // T0 只有在別的候選句明顯更好時才換掉 chewing 的首選：只是略微偏好時常常是錯的
        // （繳 → 角、花 → 華），寧可維持 chewing 原本的判斷。
        // 首選用了少見讀音（打 ㄋㄚˇ 卻是「那」）時 chewing 的判斷本來就不可靠，不保護
        if let Some(t0) = self.t0.as_ref().filter(|_| !top_rare) {
            scored[0].0 -= t0.margin;
        }
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        scored.into_iter().map(|(_, o)| o).collect()
    }
}
