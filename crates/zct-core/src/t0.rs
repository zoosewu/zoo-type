//! T0：包住 chewing 的轉換引擎，用字 n-gram 語言模型替前幾名候選句重新排序（需求 4.13）
//!
//! chewing 用「詞」的 bigram 算出前 10 名候選句（cost 越低越好，log10 尺度），
//! 這裡再加上「字」n-gram 的分數（log10，越高越通順）：`cost - weight × score`，取最低的當首選。
//! 使用者選過的字（組字區的選取）chewing 已經放進候選句，重新排序不會改掉它們。

use std::sync::Arc;

use chewing::conversion::{
    ChewingEngine, Composition, ConversionEngine, Decoder, LatticeBuilder, Outcome,
};
use chewing::dictionary::{CompositeDict, LookupStrategy, StringTable};
use chewing::lm::{StaticDict, StaticLm};
use chewing::user::{HistoryDict, SessionHistoryDict, UserDict};

use crate::ngram::NgramModel;

/// 字 n-gram 分數的權重（評測調出來的）
pub const DEFAULT_WEIGHT: f64 = 0.5;
/// 換掉 chewing 首選需要的差距（log10）
pub const DEFAULT_MARGIN: f64 = 0.0;

#[derive(Debug)]
pub struct T0Engine {
    inner: ChewingEngine,
    model: Arc<NgramModel>,
    weight: f64,
    margin: f64,
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

impl T0Engine {
    pub fn new(parts: Parts, model: Arc<NgramModel>, weight: f64, margin: f64) -> T0Engine {
        let session = SessionHistoryDict::new(parts.string_table.clone());
        let dict = CompositeDict::new(
            parts.static_dict,
            parts.rare_dict,
            parts.history_dict.clone(),
            session,
            parts.user_dict,
        );
        T0Engine {
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
            model,
            weight,
            margin,
        }
    }
}

fn text_of(o: &Outcome) -> String {
    o.intervals.iter().map(|i| &*i.text).collect()
}

impl ConversionEngine for T0Engine {
    fn convert<'a>(&'a self, comp: &'a Composition) -> Vec<Outcome> {
        let mut outcomes = ConversionEngine::convert(&self.inner, comp);
        if outcomes.len() < 2 {
            return outcomes;
        }
        let mut scored: Vec<(f64, Outcome)> = outcomes
            .drain(..)
            .map(|o| (o.cost - self.weight * self.model.score(&text_of(&o)), o))
            .collect();
        // chewing 的首選只有在別的候選句明顯更好時才換掉：T0 只是略微偏好時常常是錯的
        // （繳 → 角、花 → 華），寧可維持 chewing 原本的判斷
        let first = scored[0].0 - self.margin;
        scored[0].0 = first;
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        scored.into_iter().map(|(_, o)| o).collect()
    }
}
