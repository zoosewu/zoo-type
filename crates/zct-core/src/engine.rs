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
use chewing::zhuyin::Syllable;

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

/// 每日選字統計（需求 4.14：個人首選正確率）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    dir: PathBuf,
    dirty: AtomicBool,
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

        Ok(Engine {
            string_table,
            static_dict,
            rare_dict,
            lm,
            symbols,
            user_dict,
            history,
            dir: dir.to_path_buf(),
            dirty: AtomicBool::new(false),
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

    /// 忘記一個詞的學習紀錄，並取消固頂
    pub(crate) fn forget(&self, syllables: &[Syllable], word: &str) {
        self.history.remove(syllables, word);
        if let Some(boost) = self.boost_of(syllables, word)
            && boost > 0
        {
            for _ in 0..(boost as i32 + 9) / 10 {
                self.user_dict.deboost(syllables, word);
            }
        }
        self.mark_dirty();
    }

    /// 固頂：在使用者詞庫中大幅加權
    pub(crate) fn pin(&self, syllables: &[Syllable], word: &str) {
        if self.boost_of(syllables, word).is_none() {
            // 新條目第一次 boost 只有 +10
            self.user_dict.boost(syllables, word);
        }
        self.user_dict.boost(syllables, word);
        self.mark_dirty();
    }

    pub fn boost_of(&self, syllables: &[Syllable], word: &str) -> Option<i8> {
        self.user_dict
            .lookup(syllables, LookupStrategy::Standard)
            .into_iter()
            .find(|(wid, _)| self.user_dict.get_text(*wid).as_deref() == Some(word))
            .map(|(_, b)| b)
    }

    /// 有變更才寫檔；回傳是否有寫入
    pub fn save(&self) -> std::io::Result<bool> {
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
        if !target.exists() {
            std::fs::create_dir_all(&target)?;
            for name in [USER_DICT, HISTORY, STATS] {
                let src = self.dir.join(name);
                if src.exists() {
                    std::fs::copy(&src, target.join(name))?;
                }
            }
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
