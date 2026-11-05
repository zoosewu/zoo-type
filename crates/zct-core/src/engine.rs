//! 共用的引擎資料：系統詞庫只載入一次，所有 session 共用同一份使用者詞庫與學習紀錄
//!
//! 先前每個連線各自載入、各自寫檔，多個 App 的學習會互相覆蓋（P1 的可能成因）。
//! 這裡建立的 chewing Editor 不會自己寫檔；由服務程序在背景定期呼叫 [`Engine::save`]。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chewing::dictionary::{LookupStrategy, StringTable};
use chewing::editor::{Editor, EditorBuilder, SymbolSelector};
use chewing::lm::{LoadMode, StaticDict, StaticLm};
use chewing::user::{HistoryDict, UserDict};
use chewing::zhuyin::{Syllable, SyllableVec};
use serde::{Deserialize, Serialize};

const USER_DICT: &str = "user_dict.csv";
const HISTORY: &str = "history_dict.bin";
const STATS: &str = "stats.tsv";

#[derive(Debug)]
pub struct EngineError(String);

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "無法載入詞庫：{}", self.0)
    }
}

impl std::error::Error for EngineError {}

fn err(e: impl std::fmt::Display) -> EngineError {
    EngineError(e.to_string())
}

/// 使用者詞庫中的一筆
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhraseEntry {
    pub word: String,
    /// 注音，字與字之間以空白分隔，例如 `ㄊㄧㄢ ㄑㄧˋ`
    pub bopomofo: String,
    /// 加權：`10` 一般自訂詞、大於 10 固頂、`-100` 封鎖（不出現在候選）
    pub boost: i8,
}

impl PhraseEntry {
    pub const BANNED: i8 = UserDict::MIN;
    /// chewing 新加入的自訂詞的加權
    pub const NORMAL: i8 = 10;

    pub fn is_pinned(&self) -> bool {
        self.boost > Self::NORMAL
    }

    pub fn is_banned(&self) -> bool {
        self.boost == Self::BANNED
    }
}

/// 對使用者詞庫中一筆詞的操作
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhraseAction {
    /// 封鎖：不再出現在候選
    Ban,
    /// 解除封鎖
    Unban,
    /// 取消固頂並清除學習紀錄
    Unpin,
}

/// 每日選字統計（需求 4.14：個人首選正確率）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayStats {
    /// UTC 日期 `YYYY-MM-DD`
    pub day: String,
    /// 送出的中文字數
    pub chars: u64,
    /// 開啟候選窗並選字的次數（= 自動選字不對、需要修正的次數）
    pub corrections: u64,
}

#[derive(Debug)]
pub struct Engine {
    string_table: StringTable,
    static_dict: StaticDict,
    rare_dict: StaticDict,
    lm: StaticLm,
    symbols: SymbolSelector,
    user_dict: UserDict,
    history: HistoryDict,
    /// 英文單字表（中英混打）；沒有 `english.txt` 時為空，功能自動停用
    words: crate::mixed::WordList,
    dir: PathBuf,
    dirty: AtomicBool,
    /// 還原備份後停止存檔，避免記憶體中的舊資料蓋掉剛還原的檔案
    frozen: AtomicBool,
    stats: Mutex<Vec<DayStats>>,
}

/// UTC 日期，避免為此引入時間函式庫
pub fn today() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Howard Hinnant 的 civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// 先寫暫存檔再改名：寫到一半當機也不會留下損壞的檔案
fn write_atomic(path: &Path, f: impl FnOnce(&mut BufWriter<File>) -> std::io::Result<()>) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut w = BufWriter::new(File::create(&tmp)?);
        f(&mut w)?;
        w.flush()?;
        w.get_ref().sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn load_stats(path: &Path) -> Vec<DayStats> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut it = line.split('\t');
            Some(DayStats {
                day: it.next()?.to_owned(),
                chars: it.next()?.parse().ok()?,
                corrections: it.next()?.parse().ok()?,
            })
        })
        .collect()
}

impl Engine {
    /// `sys_dir`：系統詞庫；`dir`：使用者詞庫與學習紀錄（不存在會建立）
    pub fn open(sys_dir: &Path, dir: &Path) -> Result<Engine, EngineError> {
        std::fs::create_dir_all(dir).map_err(err)?;
        let string_table = StringTable::open_bin(sys_dir.join("static_words.bin")).map_err(err)?;
        let static_dict = StaticDict::open(sys_dir.join("static_dict.bin")).map_err(err)?;
        let rare_dict = StaticDict::open(sys_dir.join("rare_dict.bin")).map_err(err)?;
        let lm = StaticLm::open(sys_dir.join("static_lm.bin"), LoadMode::Lazy).map_err(err)?;
        let symbols = SymbolSelector::open(sys_dir.join("symbols.dat")).unwrap_or_default();

        // 使用者檔案壞掉時不能讓輸入法無法啟動：保留原檔、改用空的
        let user_path = dir.join(USER_DICT);
        let user_dict = if user_path.exists() {
            UserDict::open(&user_path, string_table.clone()).unwrap_or_else(|e| {
                log::error!("使用者詞庫讀取失敗，改用空詞庫：{e}");
                let _ = std::fs::copy(&user_path, user_path.with_extension("csv.broken"));
                UserDict::new(string_table.clone())
            })
        } else {
            UserDict::new(string_table.clone())
        };
        let hist_path = dir.join(HISTORY);
        let history = if hist_path.exists() {
            HistoryDict::open(&hist_path, string_table.clone()).unwrap_or_else(|e| {
                log::error!("學習紀錄讀取失敗，改用空紀錄：{e}");
                let _ = std::fs::copy(&hist_path, hist_path.with_extension("bin.broken"));
                HistoryDict::new(string_table.clone())
            })
        } else {
            HistoryDict::new(string_table.clone())
        };

        let words = crate::mixed::WordList::load(&sys_dir.join("english.txt")).unwrap_or_else(|e| {
            log::warn!("沒有英文單字表，中英混打停用：{e}");
            crate::mixed::WordList::default()
        });

        Ok(Engine {
            words,
            string_table,
            static_dict,
            rare_dict,
            lm,
            symbols,
            user_dict,
            history,
            dir: dir.to_path_buf(),
            dirty: AtomicBool::new(false),
            frozen: AtomicBool::new(false),
            stats: Mutex::new(load_stats(&dir.join(STATS))),
        })
    }

    /// 建立共用詞庫的 Editor（不會自己寫檔）
    pub(crate) fn editor(&self) -> Editor {
        EditorBuilder::new()
            .string_table(self.string_table.clone())
            .static_dict(self.static_dict.clone())
            .rare_dict(self.rare_dict.clone())
            .user_dict(self.user_dict.clone())
            .history_dict(self.history.clone())
            .static_lm(self.lm.clone())
            .symbol_selector(self.symbols.clone())
            .build()
    }

    pub(crate) fn words(&self) -> &crate::mixed::WordList {
        &self.words
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// 記錄一次送出與修正次數
    pub(crate) fn record(&self, chars: u64, corrections: u64) {
        if chars == 0 && corrections == 0 {
            return;
        }
        let day = today();
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        match stats.last_mut() {
            Some(s) if s.day == day => {
                s.chars += chars;
                s.corrections += corrections;
            }
            _ => stats.push(DayStats {
                day,
                chars,
                corrections,
            }),
        }
        self.mark_dirty();
    }

    pub fn stats(&self) -> Vec<DayStats> {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 忘記一個詞的學習紀錄，並取消固頂（自訂詞仍保留，加權回到一般值）
    pub(crate) fn forget(&self, syllables: &[Syllable], word: &str) {
        self.history.remove(syllables, word);
        if let Some(boost) = self.boost_of(syllables, word)
            && boost > PhraseEntry::NORMAL
        {
            // deboost 每次 -10
            for _ in 0..(boost - PhraseEntry::NORMAL) / 10 {
                self.user_dict.deboost(syllables, word);
            }
        }
        self.mark_dirty();
    }

    /// 固頂：在使用者詞庫中大幅加權
    pub(crate) fn pin(&self, syllables: &[Syllable], word: &str) {
        // insert：新詞或被封鎖的詞 → 一般自訂詞（10）；再 boost +100（上限 100）
        if matches!(self.boost_of(syllables, word), None | Some(PhraseEntry::BANNED)) {
            self.user_dict.insert(syllables, word);
        }
        self.user_dict.boost(syllables, word);
        self.mark_dirty();
    }

    /// 這個詞在使用者詞庫中的加權；不在詞庫中回傳 None，被封鎖回傳 `PhraseEntry::BANNED`
    pub fn boost_of(&self, syllables: &[Syllable], word: &str) -> Option<i8> {
        let is = |wid| self.user_dict.get_text(wid).as_deref() == Some(word);
        if let Some((_, b)) = self
            .user_dict
            .lookup(syllables, LookupStrategy::Standard)
            .into_iter()
            .find(|(wid, _)| is(*wid))
        {
            return Some(b);
        }
        // 一般查詢會略過被封鎖的詞
        self.user_dict
            .lookup_banned(syllables, LookupStrategy::Standard)
            .into_iter()
            .any(is)
            .then_some(PhraseEntry::BANNED)
    }

    /// 使用者詞庫內容（設定程式的詞庫頁）
    pub fn phrases(&self) -> Vec<PhraseEntry> {
        let mut list: Vec<PhraseEntry> = self
            .user_dict
            .entries()
            .map(|(syl, word)| {
                let boost = self.boost_of(syl.as_ref(), &word).unwrap_or(0);
                let bopomofo = syl
                    .as_ref()
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                PhraseEntry {
                    word,
                    bopomofo,
                    boost,
                }
            })
            .collect();
        list.sort_by(|a, b| a.bopomofo.cmp(&b.bopomofo).then(a.word.cmp(&b.word)));
        list
    }

    fn parse(word: &str, bopomofo: &str) -> Result<SyllableVec, String> {
        let syl: SyllableVec = bopomofo
            .trim()
            .parse()
            .map_err(|_| format!("注音格式錯誤：{bopomofo}"))?;
        let n = word.chars().count();
        if n == 0 || n != syl.as_ref().len() {
            return Err(format!("「{word}」有 {n} 個字，注音有 {} 個音節", syl.as_ref().len()));
        }
        Ok(syl)
    }

    /// 加入自訂詞：使用者明確加入的詞要能打得出來，所以和固頂一樣優先
    pub fn add_phrase(&self, word: &str, bopomofo: &str) -> Result<(), String> {
        let word = word.trim();
        let syl = Self::parse(word, bopomofo)?;
        match self.boost_of(syl.as_ref(), word) {
            Some(b) if b != PhraseEntry::BANNED => return Err(format!("已有：{word}")),
            _ => self.pin(syl.as_ref(), word),
        }
        Ok(())
    }

    pub fn phrase_action(&self, word: &str, bopomofo: &str, action: PhraseAction) -> Result<(), String> {
        let syl = Self::parse(word, bopomofo)?;
        match action {
            PhraseAction::Ban => {
                self.user_dict.ban(syl.as_ref(), word);
                self.history.remove(syl.as_ref(), word);
            }
            // insert 會把被封鎖的詞恢復成一般自訂詞
            PhraseAction::Unban => self.user_dict.insert(syl.as_ref(), word),
            PhraseAction::Unpin => {
                self.forget(syl.as_ref(), word);
                return Ok(());
            }
        }
        self.mark_dirty();
        Ok(())
    }

    /// 備份清單（新的在前）
    pub fn backups(backup_dir: &Path) -> Vec<String> {
        let mut days: Vec<String> = std::fs::read_dir(backup_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        days.sort();
        days.reverse();
        days
    }

    /// 把某天的備份複製回學習資料夾，並停止存檔；呼叫端接著要結束程序，重新啟動時載入
    pub fn restore(&self, backup_dir: &Path, day: &str) -> std::io::Result<()> {
        if day.contains(['/', '\\', '.']) {
            return Err(std::io::Error::other("無效的日期"));
        }
        let src = backup_dir.join(day);
        if !src.join(HISTORY).exists() && !src.join(USER_DICT).exists() {
            return Err(std::io::Error::other(format!("找不到備份：{day}")));
        }
        self.frozen.store(true, Ordering::SeqCst);
        for name in [USER_DICT, HISTORY, STATS] {
            let (from, to) = (src.join(name), self.dir.join(name));
            if from.exists() {
                std::fs::copy(&from, &to)?;
            }
        }
        Ok(())
    }

    /// 有變更才寫檔；回傳是否有寫入
    pub fn save(&self) -> std::io::Result<bool> {
        if self.frozen.load(Ordering::SeqCst) {
            return Ok(false);
        }
        if !self.dirty.swap(false, Ordering::Relaxed) {
            return Ok(false);
        }
        let r = (|| {
            write_atomic(&self.dir.join(USER_DICT), |w| {
                self.user_dict.to_writer(w).map_err(std::io::Error::other)
            })?;
            write_atomic(&self.dir.join(HISTORY), |w| {
                self.history.to_writer(w).map_err(std::io::Error::other)
            })?;
            let stats = self.stats();
            write_atomic(&self.dir.join(STATS), |w| {
                for s in &stats {
                    writeln!(w, "{}\t{}\t{}", s.day, s.chars, s.corrections)?;
                }
                Ok(())
            })
        })();
        if r.is_err() {
            // 寫入失敗：下次再試
            self.mark_dirty();
        }
        r.map(|_| true)
    }

    /// 每日備份到 `backup_dir/<日期>/`，保留最新 `keep` 份；今天已備份過就不重複
    pub fn backup(&self, backup_dir: &Path, keep: usize) -> std::io::Result<()> {
        let target = backup_dir.join(today());
        // 還沒有任何資料時不建立備份：否則會留下空的當日備份，當天之後也不會再補
        let has_data = [USER_DICT, HISTORY].iter().any(|n| self.dir.join(n).exists());
        if has_data && !target.exists() {
            std::fs::create_dir_all(&target)?;
            for name in [USER_DICT, HISTORY, STATS] {
                let src = self.dir.join(name);
                if src.exists() {
                    std::fs::copy(&src, target.join(name))?;
                }
            }
        }
        if !backup_dir.exists() {
            return Ok(());
        }
        let mut days: Vec<PathBuf> = std::fs::read_dir(backup_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        days.sort();
        let excess = days.len().saturating_sub(keep);
        for old in &days[..excess] {
            std::fs::remove_dir_all(old)?;
        }
        Ok(())
    }

    /// 舊版（M2 / M3）把學習資料放在 chewing 自己的資料夾；新位置沒有資料時搬過來
    pub fn migrate_from(old_dir: &Path, dir: &Path) -> std::io::Result<bool> {
        let mut moved = false;
        std::fs::create_dir_all(dir)?;
        for name in [USER_DICT, HISTORY] {
            let (src, dst) = (old_dir.join(name), dir.join(name));
            if src.exists() && !dst.exists() {
                std::fs::copy(&src, &dst)?;
                moved = true;
            }
        }
        Ok(moved)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_is_a_valid_date() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
}
