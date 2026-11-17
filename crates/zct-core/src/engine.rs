//! 共用的引擎資料：系統詞庫只載入一次，所有 session 共用同一份使用者詞庫與學習紀錄
//!
//! 先前每個連線各自載入、各自寫檔，多個 App 的學習會互相覆蓋（P1 的可能成因）。
//! 這裡建立的 chewing Editor 不會自己寫檔；由服務程序在背景定期呼叫 [`Engine::save`]。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
/// 自訂縮寫（`tw` → 台灣），見 [`crate::abbrev`]
const ABBREVS: &str = "abbrev.tsv";
/// 選字修正的累計次數（讀音 + 字），同一個修正兩次才學起來；單機資料，不同步
const CORRECTIONS: &str = "corrections.tsv";

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
    /// 加權：`0` 一般自訂詞、`LEARNED` 選字修正學到的、`PINNED` 以上固頂、`-100` 封鎖（不出現在候選）
    pub boost: i8,
}

impl PhraseEntry {
    pub const BANNED: i8 = UserDict::MIN;
    /// chewing 新加入的自訂詞的加權（0.14.0-rc.1 起是 0，之前是 10）
    pub const NORMAL: i8 = 0;
    /// chewing 0.14.0-rc.1 之前的一般自訂詞加權；讀檔時換成 `NORMAL`
    const LEGACY_NORMAL: i8 = 10;
    /// 選字修正學到的詞：只要進了使用者詞庫就足以勝過語言模型的預設（實測「重開機」10 就夠），
    /// 比手動加的一般詞高一些，離固頂很遠
    pub const LEARNED: i8 = 30;
    /// 固頂（Shift + Enter）
    pub const PINNED: i8 = 100;

    pub fn is_pinned(&self) -> bool {
        self.boost >= Self::PINNED
    }

    pub fn is_learned(&self) -> bool {
        self.boost > Self::NORMAL && self.boost < Self::PINNED
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
    /// 設為優先（固頂）
    Pin,
    /// 從使用者詞庫移除（連同學習紀錄）；系統詞庫的詞恢復成預設的排序，自己加的詞不再出現
    Remove,
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
    /// 使用者的英文單字（自動學到、手動加入、排除）；執行中會變動
    user_words: std::sync::RwLock<crate::mixed::UserWords>,
    /// 上次切換的全形 / 半形：所有 App、切到別的輸入法再切回來都沿用
    width: Mutex<Option<crate::config::Width>>,
    /// Emoji 關鍵字（CLDR 繁中註解）；沒有檔案時只能用常用 Emoji
    emoji: crate::emoji::EmojiIndex,
    dir: PathBuf,
    /// 系統詞庫資料夾（`tsi.csv` 在這裡，用來估計單字的常用程度）
    sys_dir: PathBuf,
    /// tsi.csv 的使用量統計；開啟時在背景載入
    usage: Arc<std::sync::OnceLock<crate::usage::Usage>>,
    dirty: AtomicBool,
    /// 還原備份後停止存檔，避免記憶體中的舊資料蓋掉剛還原的檔案
    frozen: AtomicBool,
    stats: Mutex<Vec<DayStats>>,
    /// 選字修正累計次數：`詞 \t 注音` → 次數
    corrections: Mutex<std::collections::BTreeMap<String, u32>>,
    /// 自訂縮寫
    abbrevs: std::sync::RwLock<crate::abbrev::Abbrevs>,
    /// 最近送出的中文與當時的讀音（新的在後）：重新選字時最準的讀音來源
    recent: Mutex<std::collections::VecDeque<(String, Vec<String>)>>,
    /// T0：字 n-gram 模型與權重（系統詞庫資料夾有 ngram.bin 時才啟用）
    t0: Option<crate::rank::T0>,
}

/// 記住最近幾段送出的文字
const RECENT_COMMITS: usize = 50;

/// 人稱代名詞：你／妳、他／她／它／牠／祂要看說話的對象，AI 看不出來，一律不讓 AI 改
fn is_pronoun(c: char) -> bool {
    matches!(c, '你' | '妳' | '他' | '她' | '它' | '牠' | '祂')
}

/// T0 模型：系統詞庫資料夾的 `ngram.bin`；開發時可用環境變數 `ZCT_T0_MODEL`（`off` 停用）、
/// `ZCT_T0_WEIGHT`、`ZCT_T0_MARGIN` 指定模型、權重與門檻（評測比較用）
fn load_t0(sys_dir: &Path) -> Option<crate::rank::T0> {
    let path = match std::env::var_os("ZCT_T0_MODEL") {
        Some(v) if v == "off" => return None,
        Some(v) => PathBuf::from(v),
        None => sys_dir.join("ngram.bin"),
    };
    if !path.exists() {
        return None;
    }
    match crate::ngram::NgramModel::load(&path) {
        Ok(m) => Some(crate::rank::T0 {
            model: Arc::new(m),
            weight: env_f64("ZCT_T0_WEIGHT", crate::rank::DEFAULT_WEIGHT),
            margin: env_f64("ZCT_T0_MARGIN", crate::rank::DEFAULT_MARGIN),
        }),
        Err(e) => {
            log::warn!("T0 模型讀取失敗，不使用：{e}");
            None
        }
    }
}

/// 評測比較用的參數（環境變數），沒設定時用預設值
fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|w| w.parse().ok())
        .unwrap_or(default)
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
fn write_atomic(
    path: &Path,
    f: impl FnOnce(&mut BufWriter<File>) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut w = BufWriter::new(File::create(&tmp)?);
        f(&mut w)?;
        w.flush()?;
        w.get_ref().sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn load_corrections(path: &Path) -> std::collections::BTreeMap<String, u32> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (key, n) = l.rsplit_once('\t')?;
            Some((key.to_owned(), n.trim().parse().ok()?))
        })
        .collect()
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

        let mut words =
            crate::mixed::WordList::load(&sys_dir.join("english.txt")).unwrap_or_else(|e| {
                log::warn!("沒有英文單字表，只用內建的補充單字：{e}");
                crate::mixed::WordList::default()
            });
        // 一般字典沒有的技術用語與縮寫（repo、config、api…）
        words.extend(include_str!("../data/english-extra.txt"));

        let emoji = crate::emoji::EmojiIndex::load(&sys_dir.join("emoji-zh-Hant.json"))
            .unwrap_or_else(|e| {
                log::warn!("沒有 Emoji 關鍵字資料，只能使用常用 Emoji：{e}");
                crate::emoji::EmojiIndex::default()
            });

        let engine = Engine {
            words,
            user_words: Default::default(),
            width: Mutex::new(None),
            emoji,
            string_table,
            static_dict,
            rare_dict,
            lm,
            symbols,
            user_dict,
            history,
            dir: dir.to_path_buf(),
            sys_dir: sys_dir.to_path_buf(),
            usage: Arc::default(),
            dirty: AtomicBool::new(false),
            frozen: AtomicBool::new(false),
            stats: Mutex::new(load_stats(&dir.join(STATS))),
            corrections: Mutex::new(load_corrections(&dir.join(CORRECTIONS))),
            abbrevs: std::sync::RwLock::new(crate::abbrev::Abbrevs::parse(
                &std::fs::read_to_string(dir.join(ABBREVS)).unwrap_or_default(),
            )),
            recent: Mutex::default(),
            t0: load_t0(sys_dir),
        };
        // 第一次排候選字、轉換時就要用到；臨時讀檔會讓那個按鍵超過 TSF 等待的時限
        let usage = engine.usage.clone();
        let path = sys_dir.join("tsi.csv");
        std::thread::spawn(move || {
            usage.get_or_init(|| crate::usage::Usage::load(&path));
        });
        engine.migrate_normal_boost();
        Ok(engine)
    }

    /// chewing 0.14.0-rc.1 起一般自訂詞的加權從 10 改成 0：舊檔的 10 換成 0
    ///
    /// 現在的流程不會留下 10（`set_boost` 只設 0、`LEARNED`、`PINNED`），重複執行也安全。
    fn migrate_normal_boost(&self) {
        let old: Vec<_> = self
            .user_dict
            .entries()
            .filter(|(syl, word)| {
                self.boost_of(syl.as_ref(), word) == Some(PhraseEntry::LEGACY_NORMAL)
            })
            .collect();
        for (syl, word) in &old {
            self.set_boost(syl.as_ref(), word, PhraseEntry::NORMAL);
        }
        if !old.is_empty() {
            log::info!("自訂詞加權換成新版的一般值：{} 筆", old.len());
            self.mark_dirty();
        }
    }

    /// 上次切換的全形 / 半形（尚未切換過時為 None，用設定檔的預設）
    pub fn width(&self) -> Option<crate::config::Width> {
        *self.width.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn set_width(&self, w: crate::config::Width) {
        *self.width.lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
    }

    /// 單字在這個讀音下的常用程度
    ///
    /// 語言模型給破音字的單字分數常常偏低（「重（ㄔㄨㄥˊ）」排在罕用字之後），
    /// 因為使用量都記在詞裡（重新、重複…）；改用「含有這個字這個讀音的詞的頻率加總」排序單字候選。
    /// 沒有 tsi.csv 時全部是 0（維持 chewing 原本的順序）。
    pub fn char_usage(&self, c: char, reading: &str) -> u64 {
        self.usage().char_usage(c, reading)
    }

    /// AI 修正只接受讀音和使用者打的注音相同的字：讀音不符（或不是中文音節）的位置還原成原字
    ///
    /// `syllables`：原文每個字的讀音（英文、符號是 None）。判斷用詞庫裡這個字有沒有這個讀音，
    /// 查不到的罕用字也還原（寧可不修）
    pub fn keep_same_reading(
        &self,
        original: &str,
        syllables: &[Option<String>],
        suggestion: &str,
    ) -> String {
        if original.chars().count() != suggestion.chars().count() {
            return original.to_owned();
        }
        original
            .chars()
            .zip(suggestion.chars())
            .enumerate()
            .map(|(i, (o, s))| {
                let ok = o == s
                    || (!is_pronoun(o)
                        && !is_pronoun(s)
                        && syllables
                            .get(i)
                            .and_then(Option::as_deref)
                            .is_some_and(|r| self.char_usage(s, r) > 0));
                if ok { s } else { o }
            })
            .collect()
    }

    /// 還在背景載入時等它載完
    fn usage(&self) -> &crate::usage::Usage {
        self.usage
            .get_or_init(|| crate::usage::Usage::load(&self.sys_dir.join("tsi.csv")))
    }

    /// 只屬於簡體的字 → 對應的繁體字（繁體模式要隱藏或換掉的字）
    ///
    /// 條件：OpenCC 字表標成只屬於簡體（見 [`crate::hanzi`]），而且在台灣詞庫裡的使用量
    /// 不到對應繁體字的 1/10。「秘 / 祕」「群 / 羣」這類台灣也常用的寫法因此保留。
    pub fn simplified_only(&self, c: char) -> Option<char> {
        let t = crate::hanzi::traditional_of(c)?;
        let total = |c: char| self.usage().total(c);
        (total(c) * 10 < total(t)).then_some(t)
    }

    /// 把只屬於簡體的字換成繁體（一對一，字數不變，游標位置不受影響）
    pub fn to_traditional(&self, text: &str) -> String {
        text.chars()
            .map(|c| self.simplified_only(c).unwrap_or(c))
            .collect()
    }

    /// 使用者的英文單字檔（資料資料夾 `dict/english.txt`，隨 git 同步）
    pub fn load_user_words(&mut self, path: &Path) {
        self.reload_user_words(path);
    }

    /// Reload synchronized English entries without replacing active sessions.
    pub fn reload_user_words(&self, path: &Path) {
        *self.user_words.write().unwrap_or_else(|e| e.into_inner()) =
            crate::mixed::UserWords::load(path);
    }

    fn user_words(&self) -> std::sync::RwLockReadGuard<'_, crate::mixed::UserWords> {
        self.user_words.read().unwrap_or_else(|e| e.into_inner())
    }

    /// 是不是英文單字：使用者的設定優先，其次看字典
    pub fn is_english(&self, word: &str) -> bool {
        let w = word.to_ascii_lowercase();
        self.user_words()
            .verdict(&w)
            .unwrap_or_else(|| self.words.contains(&w))
    }

    /// C：按空白時要自動變成英文嗎（使用者加入的字不限長度）
    pub(crate) fn auto_english(&self, keys: &str) -> bool {
        let added = self.user_words().verdict(keys) == Some(true);
        let shape = crate::mixed::auto_english_shape(keys)
            || (added && keys.len() >= 2 && !crate::mixed::is_syllable(keys));
        shape && self.is_english(keys)
    }

    /// B：候選窗要附上英文嗎
    pub(crate) fn english_candidate(&self, keys: &str) -> bool {
        crate::mixed::english_candidate_shape(keys) && self.is_english(keys)
    }

    /// 學到一個英文單字（在組字區用 Shift 打的）；字典已有的不重複記
    pub fn learn_english(&self, word: &str) {
        if word.len() < 2 || !word.bytes().all(|b| b.is_ascii_alphabetic()) || self.is_english(word)
        {
            return;
        }
        log::info!("學到英文單字（{} 個字母）", word.len());
        self.user_words
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .add(word);
    }

    /// 不再把這個字當英文；回傳原本是不是英文
    pub fn forget_english(&self, word: &str) -> bool {
        let was = self.is_english(word);
        let in_dictionary = self.words.contains(word);
        self.user_words
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(word, in_dictionary);
        was
    }

    /// 記住剛送出的中文與讀音（只在記憶體，不寫檔）
    pub(crate) fn remember_reading(&self, text: String, syllables: Vec<String>) {
        if text.is_empty() || text.chars().count() != syllables.len() {
            return;
        }
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        if recent.len() == RECENT_COMMITS {
            recent.pop_front();
        }
        recent.push_back((text, syllables));
    }

    /// 重新選字：一段文字可能的讀音，最可能的在前。最近送出過的用當時的讀音，其次查系統詞庫的 tsi.csv
    pub fn system_dictionary(&self) -> &Path {
        &self.sys_dir
    }

    pub fn reading_choices(&self, text: &str) -> Vec<Vec<String>> {
        const MAX: usize = 9;
        let mut out: Vec<Vec<String>> = Vec::new();
        {
            let recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
            for (sent, syl) in recent.iter().rev() {
                if let Some(at) = sent.find(text) {
                    let start = sent[..at].chars().count();
                    let s = syl[start..start + text.chars().count()].to_vec();
                    if !out.contains(&s) {
                        out.push(s);
                    }
                    break;
                }
            }
        }
        let csv = std::fs::read_to_string(self.sys_dir.join("tsi.csv")).unwrap_or_default();
        for s in crate::readings::Readings::for_text(&csv, text).choices(text, MAX) {
            if out.len() < MAX && !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// 使用者的英文單字（設定程式的詞庫頁）：讀音留空；加入的是一般、排除的是封鎖。
    /// 不在 [`Engine::phrases`] 裡：英文單字檔自己隨 git 同步，不經過詞庫的同步匯出
    pub fn english_entries(&self) -> Vec<PhraseEntry> {
        let words = self.user_words();
        let entry = |w: &str, boost| PhraseEntry {
            word: w.to_owned(),
            bopomofo: String::new(),
            boost,
        };
        words
            .added()
            .map(|w| entry(w, PhraseEntry::NORMAL))
            .chain(words.removed().map(|w| entry(w, PhraseEntry::BANNED)))
            .collect()
    }

    fn user_words_mut(&self) -> std::sync::RwLockWriteGuard<'_, crate::mixed::UserWords> {
        self.user_words.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 詞庫頁對英文單字（讀音留空）的操作：一般 = 當英文、封鎖 = 不當英文、移除 = 照字典
    fn english_action(&self, word: &str, action: PhraseAction) -> Result<(), String> {
        let w = word.trim();
        if !crate::abbrev::is_abbrev(w) {
            return Err(format!("讀音留空時，詞要是英文單字：{w}"));
        }
        match action {
            PhraseAction::Unban => self.user_words_mut().add(w),
            PhraseAction::Ban => self.user_words_mut().remove(w, true),
            PhraseAction::Remove => {
                self.user_words_mut().clear(w);
            }
            PhraseAction::Pin | PhraseAction::Unpin => return Err("英文單字沒有優先".into()),
        }
        Ok(())
    }

    /// 建立共用詞庫的 Editor（不會自己寫檔）
    /// `language_model`：使用 T0（設定 `candidates.language_model`，有模型時才有作用）
    pub(crate) fn editor(&self, language_model: bool) -> Editor {
        let parts = crate::rank::Parts {
            string_table: self.string_table.clone(),
            static_dict: self.static_dict.clone(),
            rare_dict: self.rare_dict.clone(),
            history_dict: self.history.clone(),
            user_dict: self.user_dict.clone(),
            lm: self.lm.clone(),
        };
        let t0 = self.t0.clone().filter(|_| language_model);
        let reading_weight = env_f64(
            "ZCT_READING_WEIGHT",
            if t0.is_some() {
                crate::rank::READING_WEIGHT_T0
            } else {
                crate::rank::READING_WEIGHT
            },
        );
        let mut editor = EditorBuilder::new()
            .string_table(self.string_table.clone())
            .static_dict(self.static_dict.clone())
            .rare_dict(self.rare_dict.clone())
            .user_dict(self.user_dict.clone())
            .history_dict(self.history.clone())
            .static_lm(self.lm.clone())
            .symbol_selector(self.symbols.clone())
            .build();
        editor.set_conversion_engine(Box::new(crate::rank::RankEngine::new(
            parts,
            self.usage.clone(),
            reading_weight,
            t0,
        )));
        editor
    }

    pub(crate) fn emoji(&self) -> &crate::emoji::EmojiIndex {
        &self.emoji
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
        if self
            .boost_of(syllables, word)
            .is_some_and(|b| b > PhraseEntry::NORMAL)
        {
            self.set_boost(syllables, word, PhraseEntry::NORMAL);
        }
        self.corrections_lock()
            .remove(&correction_key(syllables, word));
        self.mark_dirty();
    }

    /// 把使用者詞庫中的加權設成 `target`（0、10 … 100）
    ///
    /// chewing 只提供 insert（設為 0）、boost（+100，上限 100）、deboost（-10），而且是 i8：
    /// 加權 28 以上再 boost 會溢位成負數（「取消優先」失效的根因），所以先降到 0 再往上調。
    /// 每一步都讀回實際值，不自己推算。
    fn set_boost(&self, syllables: &[Syllable], word: &str, target: i8) {
        match self.boost_of(syllables, word) {
            Some(b) if b != PhraseEntry::BANNED => {}
            _ => self.user_dict.insert(syllables, word),
        }
        let cur = || {
            self.boost_of(syllables, word)
                .unwrap_or(PhraseEntry::NORMAL)
        };
        // 次數上限只是保險，正常情況幾次就結束
        for _ in 0..30 {
            if cur() <= PhraseEntry::NORMAL {
                break;
            }
            self.user_dict.deboost(syllables, word);
        }
        if target > cur() {
            self.user_dict.boost(syllables, word);
        }
        for _ in 0..30 {
            if cur() <= target {
                break;
            }
            self.user_dict.deboost(syllables, word);
        }
    }

    fn corrections_lock(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<String, u32>> {
        self.corrections.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 使用者把自動選的字改成 `word` 並送出：同一個修正第二次時學起來（回傳 true）
    pub(crate) fn note_correction(&self, syllables: &[Syllable], word: &str) -> bool {
        let n = {
            let mut c = self.corrections_lock();
            let n = c.entry(correction_key(syllables, word)).or_insert(0);
            *n += 1;
            *n
        };
        self.mark_dirty();
        if n < 2 {
            return false;
        }
        match self.boost_of(syllables, word) {
            // 封鎖的詞不自動學；已經學過或固頂的不動
            Some(b) if b == PhraseEntry::BANNED || b >= PhraseEntry::LEARNED => false,
            _ => {
                self.set_boost(syllables, word, PhraseEntry::LEARNED);
                true
            }
        }
    }

    /// 固頂：在使用者詞庫中大幅加權
    ///
    /// chewing 的 boost 是 `i8 + 100` 之後才夾在上限，加權超過 27 時再 boost 會溢位
    /// （debug 版 panic、release 版繞回負數，詞反而被降權），所以已經固頂的詞不再加權。
    pub(crate) fn pin(&self, syllables: &[Syllable], word: &str) {
        if !self
            .boost_of(syllables, word)
            .is_some_and(|b| b >= PhraseEntry::PINNED)
        {
            self.set_boost(syllables, word, PhraseEntry::PINNED);
        }
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
        list.extend(self.abbrevs_read().entries().map(|(a, w)| PhraseEntry {
            word: w.to_owned(),
            bopomofo: a.to_owned(),
            boost: PhraseEntry::NORMAL,
        }));
        list.sort_by(|a, b| a.bopomofo.cmp(&b.bopomofo).then(a.word.cmp(&b.word)));
        list
    }

    fn abbrevs_read(&self) -> std::sync::RwLockReadGuard<'_, crate::abbrev::Abbrevs> {
        self.abbrevs.read().unwrap_or_else(|e| e.into_inner())
    }

    fn abbrevs_write(&self) -> std::sync::RwLockWriteGuard<'_, crate::abbrev::Abbrevs> {
        self.abbrevs.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 縮寫對應的詞（打縮寫再按空白時換成這個詞）
    pub fn abbreviation(&self, keys: &str) -> Option<String> {
        self.abbrevs_read().lookup(keys).map(str::to_owned)
    }

    fn parse(word: &str, bopomofo: &str) -> Result<SyllableVec, String> {
        let syl: SyllableVec = bopomofo
            .trim()
            .parse()
            .map_err(|_| format!("注音格式錯誤：{bopomofo}"))?;
        let n = word.chars().count();
        if n == 0 || n != syl.as_ref().len() {
            return Err(format!(
                "「{word}」有 {n} 個字，注音有 {} 個音節",
                syl.as_ref().len()
            ));
        }
        Ok(syl)
    }

    /// 加入自訂詞：使用者明確加入的詞要能打得出來，所以和固頂一樣優先
    ///
    /// 讀音是英文字母時加入的是縮寫（`tw` → 台灣）
    /// Canonical identity shared by settings editors on both platforms.
    pub fn normalize_phrase(word: &str, reading: &str) -> Result<(String, String), String> {
        let word = word.trim();
        let reading = reading.trim();
        if word.is_empty() {
            return Err("詞語不能是空白".into());
        }
        if reading.is_empty() {
            return Ok((word.to_ascii_lowercase(), String::new()));
        }
        if crate::abbrev::is_abbrev(reading) {
            return Ok((word.to_owned(), reading.to_ascii_lowercase()));
        }
        let syllables = Self::parse(word, reading)?;
        Ok((
            word.to_owned(),
            syllables
                .as_ref()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        ))
    }

    pub fn add_phrase(&self, word: &str, bopomofo: &str) -> Result<(), String> {
        let word = word.trim();
        if bopomofo.trim().is_empty() {
            if self.user_words().verdict(&word.to_ascii_lowercase()) == Some(true) {
                return Err(format!("詞庫已有：{word}"));
            }
            return self.english_action(word, PhraseAction::Unban);
        }
        if crate::abbrev::is_abbrev(bopomofo.trim()) {
            return match self.abbrevs_write().add(bopomofo, word)? {
                true => {
                    self.mark_dirty();
                    Ok(())
                }
                false => Err(format!("詞庫已有：{word}")),
            };
        }
        let syl = Self::parse(word, bopomofo)?;
        match self.boost_of(syl.as_ref(), word) {
            Some(b) if b != PhraseEntry::BANNED => return Err(format!("詞庫已有：{word}")),
            _ => self.pin(syl.as_ref(), word),
        }
        Ok(())
    }

    pub fn phrase_action(
        &self,
        word: &str,
        bopomofo: &str,
        action: PhraseAction,
    ) -> Result<(), String> {
        if bopomofo.trim().is_empty() {
            return self.english_action(word, action);
        }
        if crate::abbrev::is_abbrev(bopomofo) {
            if action != PhraseAction::Remove {
                return Err("縮寫只能移除".into());
            }
            self.abbrevs_write().remove(bopomofo, word);
            self.mark_dirty();
            return Ok(());
        }
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
            PhraseAction::Pin => {
                self.pin(syl.as_ref(), word);
                return Ok(());
            }
            PhraseAction::Remove => self.remove_phrase(syl.as_ref(), word),
        }
        self.mark_dirty();
        Ok(())
    }

    fn remove_phrase(&self, syllables: &[Syllable], word: &str) {
        self.user_dict.remove(syllables, word);
        self.history.remove(syllables, word);
        self.corrections_lock()
            .remove(&correction_key(syllables, word));
    }

    /// 同步匯入：把一個詞設成指定狀態（`current` 是本機目前狀態，None 表示不在詞庫）
    pub(crate) fn set_phrase_state(
        &self,
        word: &str,
        bopomofo: &str,
        target: crate::sync::PhraseState,
        current: Option<crate::sync::PhraseState>,
    ) -> Result<(), String> {
        use crate::sync::PhraseState;
        if crate::abbrev::is_abbrev(bopomofo) {
            let mut a = self.abbrevs_write();
            if target == PhraseState::Removed {
                a.remove(bopomofo, word);
            } else {
                a.add(bopomofo, word)?;
            }
            drop(a);
            self.mark_dirty();
            return Ok(());
        }
        let syl = Self::parse(word, bopomofo)?;
        match (target, current) {
            (PhraseState::Removed, None) => return Ok(()),
            (PhraseState::Removed, Some(_)) => self.remove_phrase(syl.as_ref(), word),
            (PhraseState::Banned, _) => {
                self.user_dict.ban(syl.as_ref(), word);
                self.history.remove(syl.as_ref(), word);
            }
            (PhraseState::Pinned, _) => self.pin(syl.as_ref(), word),
            (PhraseState::Learned, _) => self.set_boost(syl.as_ref(), word, PhraseEntry::LEARNED),
            (PhraseState::Normal, Some(PhraseState::Pinned | PhraseState::Learned)) => {
                self.forget(syl.as_ref(), word)
            }
            // 不在詞庫或被封鎖 → 一般自訂詞
            (PhraseState::Normal, _) => self.user_dict.insert(syl.as_ref(), word),
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
    /// 停止存檔（還原備份後、或已有更新版本時的舊版服務程序）
    pub fn freeze(&self) {
        self.frozen.store(true, Ordering::SeqCst);
    }

    pub fn restore(&self, backup_dir: &Path, day: &str) -> std::io::Result<()> {
        if day.contains(['/', '\\', '.']) {
            return Err(std::io::Error::other("無效的日期"));
        }
        let src = backup_dir.join(day);
        if !src.join(HISTORY).exists() && !src.join(USER_DICT).exists() {
            return Err(std::io::Error::other(format!("找不到備份：{day}")));
        }
        self.frozen.store(true, Ordering::SeqCst);
        for name in [USER_DICT, HISTORY, STATS, ABBREVS] {
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
            })?;
            let corrections = self.corrections_lock().clone();
            write_atomic(&self.dir.join(CORRECTIONS), |w| {
                for (key, n) in &corrections {
                    writeln!(w, "{key}\t{n}")?;
                }
                Ok(())
            })?;
            let abbrevs = self.abbrevs_read().to_text();
            write_atomic(&self.dir.join(ABBREVS), |w| w.write_all(abbrevs.as_bytes()))
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
        let has_data = [USER_DICT, HISTORY, ABBREVS]
            .iter()
            .any(|n| self.dir.join(n).exists());
        if has_data && !target.exists() {
            std::fs::create_dir_all(&target)?;
            for name in [USER_DICT, HISTORY, STATS, ABBREVS] {
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

/// 修正次數的鍵：`詞 \t 注音`
fn correction_key(syllables: &[Syllable], word: &str) -> String {
    let bopomofo: Vec<String> = syllables.iter().map(|s| s.to_string()).collect();
    format!("{word}\t{}", bopomofo.join(" "))
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
