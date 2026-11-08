//! 使用者詞庫的同步格式（需求 4.8）：`dict/user.tsv`
//!
//! 每行：`詞 \t 注音 \t 狀態 \t 最後修改時間（Unix 秒）`，依注音、詞排序。
//!
//! - 兩台電腦改了同一個詞時，以修改時間較新的為準
//! - git 對這個檔案用 `merge=union`：衝突的行兩邊都保留，讀入時再依時間挑選
//! - 修改時間由匯出時比對上一份檔案得出：狀態沒變就沿用舊時間，變了就用現在時間
//!
//! 限制：chewing 的自動學習紀錄是二進位格式、沒有逐筆讀取的介面，無法合併，因此不同步。

use std::collections::BTreeMap;

use crate::engine::{Engine, PhraseEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseState {
    Normal,
    /// 選字修正學到的（加權介於一般與固頂之間）
    Learned,
    Pinned,
    Banned,
}

impl PhraseState {
    fn of(p: &PhraseEntry) -> PhraseState {
        if p.is_banned() {
            PhraseState::Banned
        } else if p.is_pinned() {
            PhraseState::Pinned
        } else if p.is_learned() {
            PhraseState::Learned
        } else {
            PhraseState::Normal
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            PhraseState::Normal => "normal",
            PhraseState::Learned => "learned",
            PhraseState::Pinned => "pinned",
            PhraseState::Banned => "banned",
        }
    }

    fn parse(s: &str) -> Option<PhraseState> {
        Some(match s {
            "normal" => PhraseState::Normal,
            "learned" => PhraseState::Learned,
            "pinned" => PhraseState::Pinned,
            "banned" => PhraseState::Banned,
            _ => return None,
        })
    }
}

type Key = (String, String); // (注音, 詞)

/// 讀入同步檔；同一個詞出現多次（union 合併的結果）時取時間最新的一筆
pub fn parse(text: &str) -> BTreeMap<Key, (PhraseState, u64)> {
    let mut map: BTreeMap<Key, (PhraseState, u64)> = BTreeMap::new();
    for line in text.lines() {
        let mut it = line.split('\t');
        let (Some(word), Some(bopo), Some(state), Some(ts)) = (it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        let (Some(state), Ok(ts)) = (PhraseState::parse(state), ts.trim().parse::<u64>()) else {
            continue;
        };
        let key = (bopo.to_owned(), word.to_owned());
        match map.get(&key) {
            Some((_, old)) if *old >= ts => {}
            _ => {
                map.insert(key, (state, ts));
            }
        }
    }
    map
}

/// 依本機詞庫產生同步檔內容；`prev` 是目前檔案內容（用來保留沒變動的修改時間）
pub fn export(engine: &Engine, prev: &str, now: u64) -> String {
    let mut map = parse(prev);
    for p in engine.phrases() {
        let state = PhraseState::of(&p);
        let key = (p.bopomofo.clone(), p.word.clone());
        match map.get(&key) {
            Some((s, _)) if *s == state => {}
            _ => {
                map.insert(key, (state, now));
            }
        }
    }
    let mut out = String::new();
    for ((bopo, word), (state, ts)) in map {
        out.push_str(&format!("{word}\t{bopo}\t{}\t{ts}\n", state.as_str()));
    }
    out
}

/// 把同步檔的狀態套用到本機詞庫；回傳變更的詞數
pub fn import(engine: &Engine, text: &str) -> usize {
    let local: BTreeMap<Key, PhraseState> = engine
        .phrases()
        .iter()
        .map(|p| ((p.bopomofo.clone(), p.word.clone()), PhraseState::of(p)))
        .collect();
    let mut changed = 0;
    for ((bopo, word), (state, _)) in parse(text) {
        let current = local.get(&(bopo.clone(), word.clone())).copied();
        if current == Some(state) {
            continue;
        }
        if engine.set_phrase_state(&word, &bopo, state, current).is_ok() {
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_newest_duplicate() {
        let text = "酷音\tㄎㄨˋ ㄧㄣ\tpinned\t100\n酷音\tㄎㄨˋ ㄧㄣ\tbanned\t200\n壞行\n";
        let map = parse(text);
        assert_eq!(map.len(), 1);
        assert_eq!(map.values().next(), Some(&(PhraseState::Banned, 200)));
    }
}
