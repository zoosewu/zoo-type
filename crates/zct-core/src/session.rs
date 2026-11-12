//! 一個輸入 context 的輸入狀態：中英模式、Shift 判定、chewing 組字
//!
//! 按下事件的處理順序（第一個符合的生效）：
//! 1. 快捷鍵（遊戲模式任何時候都有效；全半形、繁簡只在中文模式有效）
//! 2. 遊戲模式「鎖定英文」→ 放行
//! 3. Ctrl / Alt / Win 組合鍵 → 放行（P3）
//! 4. 英文模式 → 放行
//! 5. 數字鍵盤、Caps Lock 字母、自訂標點、半形標點 → 由輸入法直接送出
//! 6. 候選窗選擇框（↑ ↓ Enter）
//! 7. 其餘交給 chewing

use std::sync::{Arc, OnceLock};

use chewing::editor::{BasicEditor, Editor, EditorKeyBehavior, LanguageMode};
use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;
use serde::{Deserialize, Serialize};

use crate::apps::{AppRule, Effective};
use crate::engine::Engine;
use crate::config::{
    Config, GameMode, HotkeyConfig, InputMode, MixedConfig, OnFocus, Output, PunctuationTable,
    Width,
};
use crate::mixed::{is_cjk, is_latin, is_tone};
use crate::key::{Key, KeyEvent, Mods, ascii_event, shifted, to_chewing};
use crate::shift::ShiftTap;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Preedit {
    pub text: String,
    /// 游標位置（以字元計）
    pub cursor: usize,
    /// 按 ↓ 會影響的範圍（字元起點, 長度）；游標在最後、或正在打注音時為 None
    pub target: Option<(usize, usize)>,
    /// 組字區的字數（chewing 上限 39，超過時最前面的字會自動送出）
    pub len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CandidatePage {
    pub items: Vec<String>,
    /// 從 0 開始
    pub page: usize,
    pub total_pages: usize,
    /// 目前選擇框所在的項目（本頁內的索引），Enter 選它
    pub highlight: usize,
    /// 每個項目是否已固頂（候選窗標 ★）
    pub pinned: Vec<bool>,
    /// 展開多欄（選字時按過 ← / →）：每一頁一欄；空的代表沒有展開
    pub columns: Vec<Vec<String>>,
    /// `columns[0]` 是第幾頁
    pub first_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KeyResult {
    /// `false` → 平台層要把按鍵交還給 App
    pub eaten: bool,
    /// 要送出到 App 的文字
    pub commit: Option<String>,
    pub preedit: Preedit,
    pub candidates: Option<CandidatePage>,
    /// 這次按鍵切換了中英狀態（平台層顯示「中 / A」提示）
    pub mode_changed: Option<InputMode>,
    /// 這次按鍵切換了標點全半形（平台層顯示「全形 / 半形」提示）
    pub width_changed: Option<Width>,
    /// 這次按鍵切換了繁簡輸出（平台層顯示「動 / 动」提示）
    pub output_changed: Option<Output>,
    /// 使用者按了遊戲模式快捷鍵；遊戲模式是全域狀態，由服務程序切換並廣播
    pub toggle_game: bool,
    /// 要顯示給使用者的短訊息（例如「加入：天氣」「已固頂：氣」）
    pub notice: Option<String>,
    /// 要送去 AI 修正的句子（服務程序在背景呼叫，結果用 [`Session::set_ai_suggestion`] 交回）
    pub ai_request: Option<String>,
    /// 要發出提示音（例如組不成字的注音：保留注音並提示，與微軟新注音相同）
    pub bell: bool,
    /// 使用者按了組字提示列快捷鍵：由服務程序切換這個 App 的設定並記住
    pub toggle_composition_bar: bool,
}

#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// 所有 session 共用的詞庫與學習紀錄
    pub engine: Arc<Engine>,
    pub config: Config,
    /// 目前 App 的規則（`apps.toml`）
    pub rule: Option<AppRule>,
}

#[derive(Debug)]
pub struct SessionError(String);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "無法建立輸入 session：{}", self.0)
    }
}

impl std::error::Error for SessionError {}

/// 繁→簡轉換器：第一次使用時載入，所有 session 共用
fn simplifier() -> Option<&'static OpenCC> {
    static CC: OnceLock<Option<OpenCC>> = OnceLock::new();
    CC.get_or_init(|| OpenCC::from_config(BuiltinConfig::Tw2sp).ok())
        .as_ref()
}

/// AI 修正：等待中（suggestion = None）或已有建議
#[derive(Debug)]
struct AiPick {
    original: String,
    suggestion: Option<String>,
}

/// 候選窗最後一頁的額外選項
#[derive(Debug, Clone, PartialEq, Eq)]
enum Extra {
    Emoji(String),
    /// 整段組字的注音
    Bopomofo(String),
}

impl Extra {
    fn label(&self) -> &str {
        match self {
            Extra::Emoji(e) | Extra::Bopomofo(e) => e,
        }
    }
}

/// Emoji 選擇中的狀態
#[derive(Debug)]
struct EmojiPick {
    items: Vec<String>,
    page: usize,
    highlight: usize,
}

#[derive(Debug)]
pub struct Session {
    engine: Arc<Engine>,
    editor: Editor,
    eff: Effective,
    /// App 規則明確指定的全半形（優先於上次切換的狀態）
    rule_width: Option<Width>,
    mode: InputMode,
    width: Width,
    output: Output,
    /// 手動快捷鍵開啟的全域遊戲模式（由服務程序設定）
    global_game: bool,
    shift: ShiftTap,
    hotkeys: HotkeyConfig,
    punct: PunctuationTable,
    per_page: usize,
    highlight: usize,
    mixed: MixedConfig,
    /// 目前這個音節的原始按鍵（中英混打判斷用）
    raw: String,
    /// 目前這段按鍵的位置可能是英文：組字開頭，或緊接在空白之後（「我 love」可以，「我love」不行）
    raw_english_ok: bool,
    /// B：剛完成的音節按鍵也是英文單字 → 候選窗附上的英文
    english_alt: Option<String>,
    /// E：游標前最後一個字（本輸入法送出的，或交給 App 的按鍵）；未知時為 None
    last_char: Option<char>,
    emoji: Option<EmojiPick>,
    ai: Option<AiPick>,
    ai_enabled: bool,
    /// 候選窗目前在額外選項頁（Emoji、注音；選字時按 Tab 切換）
    extras_page: bool,
    /// 候選窗已展開成多欄（選字時按過 ← / →）
    expanded: bool,
    /// 組字區中用 Shift 切英文打的單字：正在打的與已打完的，整句送出時加入自訂英文單字
    latin_word: String,
    shift_words: Vec<String>,
    /// 這次組字中的選字修正（改成和原本不同的字），送出時確認還在才算一次
    picks: Vec<Pick>,
    /// 候選清單（選字中）：詞與單字合併成同一份，由 ZCT 自己翻頁與選擇
    menu: Option<Menu>,
    /// 剛用 Backspace 刪掉完整英文單字後面的空白：下一次 Backspace 刪整個單字
    word_backspace: bool,
}

impl Session {
    pub fn new(opts: &SessionOptions) -> Result<Session, SessionError> {
        let mut editor: Editor = opts.engine.editor();
        let sort = opts.config.candidates.sort_by_frequency;

        let per_page = opts.config.candidates.per_page.clamp(1, 10);
        editor.set_editor_options(|o| {
            o.candidates_per_page = per_page;
            // 空白不開候選窗（↓ 才開）：注音未完成時是一聲，否則在組字區插入空白
            o.space_is_select_key = false;
            // 選完字後游標跳到該詞後面（今天|天氣很好 → 今天天氣|很好）
            o.auto_shift_cursor = true;
            // 全半形切換由 ZCT 自己處理，避免 chewing 吃掉 Shift+Space
            o.enable_fullwidth_toggle_key = false;
            o.sort_candidates_by_frequency = sort;
        });

        let eff = Effective::resolve(&opts.config, opts.rule.as_ref());
        Ok(Session {
            engine: opts.engine.clone(),
            editor,
            eff,
            mode: eff.default_mode,
            width: initial_width(opts, &eff),
            rule_width: opts.rule.as_ref().and_then(|r| r.punctuation),
            output: eff.output,
            global_game: false,
            shift: ShiftTap::new(opts.config.toggle.shift, opts.config.toggle.shift_tap_max_ms),
            hotkeys: opts.config.hotkeys.clone(),
            punct: opts.config.punctuation.compile().map_err(SessionError)?,
            per_page,
            highlight: 0,
            mixed: opts.config.mixed.clone(),
            raw: String::new(),
            raw_english_ok: false,
            english_alt: None,
            last_char: None,
            emoji: None,
            ai: None,
            ai_enabled: opts.config.ai.enabled,
            extras_page: false,
            expanded: false,
            latin_word: String::new(),
            shift_words: Vec::new(),
            picks: Vec::new(),
            menu: None,
            word_backspace: false,
        })
    }

    pub fn mode(&self) -> InputMode {
        self.mode
    }

    pub fn width(&self) -> Width {
        self.width
    }

    pub fn output(&self) -> Output {
        self.output
    }

    /// 目前生效的遊戲模式：手動開啟時一律鎖定英文，否則看 App 規則
    pub fn game_mode(&self) -> GameMode {
        if self.global_game {
            GameMode::English
        } else {
            self.eff.game_mode
        }
    }

    /// 平台層要做的啟發式偵測：（舊式程式 + 沒有游標，獨佔全螢幕）
    pub fn detection(&self) -> (bool, bool) {
        (self.eff.detect_legacy, self.eff.detect_fullscreen)
    }

    /// 點輸入法圖示切換中英（組字中先送出）
    pub fn toggle_mode(&mut self) -> KeyResult {
        let mut commit = String::new();
        let text = self.flush();
        self.join(&mut commit, &text);
        self.mode = self.mode.toggled();
        let r = KeyResult {
            eaten: true,
            mode_changed: Some(self.mode),
            ..Default::default()
        };
        self.finish(r, commit)
    }

    pub fn set_global_game(&mut self, on: bool) {
        self.global_game = on;
    }

    /// 共用的詞庫與學習紀錄
    pub fn engine(&self) -> Arc<Engine> {
        self.engine.clone()
    }

    pub fn is_composing(&self) -> bool {
        !self.editor.is_empty()
            || self.editor.entering_syllable()
            || self.editor.is_selecting()
            || self.emoji.is_some()
    }

    fn chinese_hotkey(&self, ev: &KeyEvent) -> bool {
        self.mode == InputMode::Chinese
            && self.game_mode() != GameMode::English
            && (self.hotkeys.width.matches(ev)
                || self.hotkeys.output.matches(ev)
                || self.hotkeys.emoji.matches(ev)
                || self.hotkeys.ai.matches(ev))
    }

    /// 由輸入法直接送出的文字（不經過 chewing）
    fn direct_text(&self, ev: &KeyEvent) -> Option<String> {
        if let Key::Numpad(c) = ev.key {
            // 選字時數字鍵盤也能選字，交給 chewing
            if self.editor.is_selecting() && c.is_ascii_digit() {
                return None;
            }
            return Some((c as char).to_string());
        }
        // Caps Lock 不影響中英：中文模式照常打注音，只影響英文的大小寫
        let Key::Char(_) = ev.key else { return None };
        let table = match self.width {
            Width::Full => &self.punct.full,
            Width::Half => &self.punct.half,
        };
        let plain = KeyEvent {
            mods: Mods {
                caps_lock: false,
                ..ev.mods
            },
            ..*ev
        };
        if let Some((_, out)) = table.iter().find(|(hk, _)| hk.matches(&plain)) {
            return Some(out.clone());
        }
        let Key::Char(c) = ev.key else { return None };
        let right_alt = self.right_alt_symbol(ev);
        if ev.mods.has_command_modifier() && !right_alt {
            return None;
        }
        crate::symbols::symbol(c, ev.mods.shift, right_alt, self.width, self.punct.style).map(String::from)
    }

    /// 開啟「右 Alt + 注音鍵」時，這是不是右 Alt + , . / ; -
    fn right_alt_symbol(&self, ev: &KeyEvent) -> bool {
        self.punct.right_alt
            && ev.mods.right_alt
            && !ev.mods.ctrl
            && !ev.mods.meta
            && !ev.mods.shift
            && matches!(ev.key, Key::Char(c) if crate::symbols::BOPOMOFO_SYMBOL_KEYS.contains(&c))
    }

    /// 只判斷「按下」事件是否會被吃掉，不改變任何狀態（給 TSF `OnTestKeyDown` 用）
    ///
    /// 放開事件請直接呼叫 [`Session::process_key`]：放開事件從不被吃掉，但 Shift 判定需要它。
    pub fn test_key(&self, ev: &KeyEvent) -> bool {
        if ev.released || ev.key.is_modifier() || ev.key == Key::Other {
            return false;
        }
        // AI 建議顯示中：Enter / 1 採用、Esc 放棄，其他鍵照常（並放棄建議）
        if self.ai_suggestion().is_some()
            && matches!(ev.key, Key::Enter | Key::Esc | Key::Char(b'1') | Key::Numpad(b'1'))
        {
            return true;
        }
        // Emoji 選擇中：除了組合鍵，按鍵都由 Emoji 選擇處理
        if self.emoji.is_some() {
            return !ev.mods.has_command_modifier();
        }
        if self.hotkeys.game_mode.matches(ev)
            || self.hotkeys.composition_bar.matches(ev)
            || self.chinese_hotkey(ev)
        {
            return true;
        }
        if self.game_mode() == GameMode::English {
            return false;
        }
        // 右 Alt + 注音鍵：打那個鍵本身的符號（設定開啟時）
        if self.mode == InputMode::Chinese && self.right_alt_symbol(ev) && !self.editor.is_selecting() {
            return true;
        }
        // P3：沒有綁定的 Ctrl / Alt / Meta 組合鍵一律交給 App
        if ev.mods.has_command_modifier() {
            return false;
        }
        if self.mode == InputMode::English {
            // 組字中切到英文：按鍵繼續放進組字區，整句一起送出
            if !self.editor.is_empty() {
                return !matches!(ev.key, Key::F(_));
            }
            // E：英文模式下，緊接在中文後的第一個英數字要由輸入法補空格
            return self.space_before_latin(ev).is_some();
        }
        if self.is_composing() {
            return !matches!(ev.key, Key::F(_));
        }
        match ev.key {
            // 沒在組字時，數字鍵盤由 App 自己處理（Caps Lock 不影響中英，字母照常打注音）
            Key::Numpad(_) => false,
            // 只有字元鍵會開始組字；Enter、空白、方向鍵等交給 App
            Key::Char(_) => true,
            _ => false,
        }
    }

    /// 給 TSF `OnTestKeyDown` 用：判斷是否會吃掉；不吃的鍵仍要讓 Shift 判定看到
    ///
    /// TSF 對不吃的鍵不會再呼叫 `OnKeyDown`，若不在這裡觀察，
    /// 英文模式下「按住 Shift 打字母」放開後會被誤判成單按 Shift。重複呼叫是安全的。
    pub fn test_key_down(&mut self, ev: &KeyEvent) -> bool {
        let eat = self.test_key(ev);
        if !eat {
            self.shift.observe(ev);
            self.observe_passthrough(ev);
        }
        eat
    }

    /// 交給 App 的按鍵：記下游標前的字，供盤古之白判斷；移動游標的鍵讓前文變成未知
    fn observe_passthrough(&mut self, ev: &KeyEvent) {
        if ev.released || ev.mods.has_command_modifier() {
            return;
        }
        match ev.key {
            Key::Char(c) => {
                self.last_char = Some(if ev.mods.shift { shifted(c) } else { c } as char);
            }
            Key::Numpad(c) => self.last_char = Some(c as char),
            Key::Space => self.last_char = Some(' '),
            Key::Shift(_) | Key::Ctrl | Key::Alt | Key::Meta | Key::CapsLock | Key::F(_) => {}
            _ => self.last_char = None,
        }
    }

    /// E：英文模式下，前一個字是中文時要補空格的英數字
    fn space_before_latin(&self, ev: &KeyEvent) -> Option<char> {
        if !self.mixed.auto_space || ev.released || ev.mods.has_command_modifier() {
            return None;
        }
        if !self.last_char.is_some_and(is_cjk) {
            return None;
        }
        match ev.key {
            Key::Char(c) if c.is_ascii_alphabetic() => {
                let upper = ev.mods.shift ^ ev.mods.caps_lock;
                Some(if upper { c.to_ascii_uppercase() } else { c } as char)
            }
            Key::Char(c) | Key::Numpad(c) if c.is_ascii_digit() && !ev.mods.shift => Some(c as char),
            _ => None,
        }
    }

    /// 把一段文字接到送出內容後面；中英文交界依設定補空格（E：盤古之白）
    fn join(&self, out: &mut String, piece: &str) {
        let Some(first) = piece.chars().next() else { return };
        let prev = out.chars().last().or(self.last_char);
        if self.mixed.auto_space
            && let Some(prev) = prev
            && ((is_cjk(prev) && is_latin(first)) || (is_latin(prev) && is_cjk(first)))
        {
            out.push(' ');
        }
        out.push_str(piece);
    }

    /// C：按空白時，剛打的按鍵是英文 → 回傳該單字
    ///
    /// 只在空白鍵判斷，而且必須是單字表中的單字、組不成注音；
    /// 亂打或打錯的注音一律當中文（組不成字就保留並提示），不會自動變英文。
    fn auto_english(&self, ev: &KeyEvent) -> Option<String> {
        let raw = &self.raw;
        let ok = self.mixed.auto_english
            && ev.key == Key::Space
            && !ev.mods.shift
            && !self.editor.is_selecting()
            && self.editor.entering_syllable()
            && self.raw_english_ok
            && self.engine.auto_english(raw);
        ok.then(|| raw.clone())
    }

    /// 組字中的英文模式：要放進組字區的字元（選字中交給候選窗，數字用來選字）
    fn latin_in_composition(&self, ev: &KeyEvent) -> Option<String> {
        if self.mode != InputMode::English || self.editor.is_empty() || self.editor.is_selecting() {
            return None;
        }
        let c = match ev.key {
            Key::Char(c) if c.is_ascii_alphabetic() => {
                let upper = ev.mods.shift ^ ev.mods.caps_lock;
                if upper { c.to_ascii_uppercase() } else { c }
            }
            Key::Char(c) if ev.mods.shift => shifted(c),
            Key::Char(c) | Key::Numpad(c) => c,
            Key::Space => b' ',
            _ => return None,
        };
        Some((c as char).to_string())
    }

    /// 記下組字區中用 Shift 打的英文單字
    fn track_latin(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_ascii_alphabetic() {
                self.latin_word.push(c);
            } else {
                self.end_latin_word();
            }
        }
    }

    fn end_latin_word(&mut self) {
        let word = std::mem::take(&mut self.latin_word);
        if word.len() >= 2 {
            self.shift_words.push(word);
        }
    }

    /// 整句送出：還留在送出內容裡的 Shift 英文單字，加入自訂英文單字（送出前刪掉的不算）
    fn learn_shift_words(&mut self, committed: &str) {
        self.end_latin_word();
        for word in std::mem::take(&mut self.shift_words) {
            if contains_word(committed, &word) {
                self.engine.learn_english(&word.to_ascii_lowercase());
            }
        }
    }

    /// 游標所在（或剛打完、緊接在游標前）的英文單字
    fn english_word_at_cursor(&self) -> Option<String> {
        if self.editor.is_selecting() || self.editor.entering_syllable() {
            return None;
        }
        let c = self.editor.cursor();
        if !self.sym_letter(c) && !(c > 0 && self.sym_letter(c - 1)) {
            return None;
        }
        let start = c - self.letters_before(c);
        let n = self.letters_after(start);
        let symbols = self.editor.symbols();
        let word: String = symbols[start..start + n]
            .iter()
            .filter_map(|s| s.to_char())
            .collect();
        (word.len() >= 2).then(|| word.to_ascii_lowercase())
    }

    /// 這個位置開始的按鍵可能是英文嗎：組字開頭，或前一個是空白
    fn english_position(&self) -> bool {
        let c = self.editor.cursor();
        c == 0 || self.sym_space(c - 1)
    }

    /// 把英文字母、空白或符號放進組字區（chewing 英文模式下會插入為一般字元）
    ///
    /// chewing 在組字區是空的時候，英文會直接送出而不是放進組字區；
    /// 這時先放一個「，」占位，插入英文後再刪掉它。
    fn insert_latin(&mut self, text: &str) {
        let placeholder = self.editor.is_empty();
        if placeholder {
            self.editor.process_keyevent(ascii_event(b'<')); // Shift + , → ，
        }
        self.editor
            .set_editor_options(|o| o.language_mode = LanguageMode::English);
        for ch in text.chars() {
            let ev = if ch.is_ascii() {
                ascii_event(ch as u8)
            } else {
                // 全形符號：chewing 英文模式會把任何 Unicode 按鍵原樣放進組字區
                chewing::input::KeyboardEvent::builder()
                    .ksym(chewing::input::keysym::Keysym::from_char(ch))
                    .build()
            };
            self.editor.process_keyevent(ev);
        }
        self.editor
            .set_editor_options(|o| o.language_mode = LanguageMode::Chinese);
        if placeholder {
            self.chewing_key(Key::Home);
            self.chewing_key(Key::Delete);
            self.chewing_key(Key::End);
        }
    }

    /// 組字區第 i 個符號是不是英文字母 / 空白
    fn sym_letter(&self, i: usize) -> bool {
        self.editor
            .symbols()
            .get(i)
            .and_then(|s| s.to_char())
            .is_some_and(|c| c.len() == 1 && c.as_bytes()[0].is_ascii_alphabetic())
    }

    fn sym_space(&self, i: usize) -> bool {
        self.editor
            .symbols()
            .get(i)
            .and_then(|s| s.to_char())
            .is_some_and(|c| c == " ")
    }

    /// 從 i 往前（含 i）連續英文字母的長度
    fn letters_before(&self, end: usize) -> usize {
        (0..end).rev().take_while(|&i| self.sym_letter(i)).count()
    }

    fn letters_after(&self, start: usize) -> usize {
        (start..self.editor.len()).take_while(|&i| self.sym_letter(i)).count()
    }

    /// 組字區中英文單字當成一個單位：← → 跳過整個單字；Backspace / Delete 刪整個完整單字
    fn word_key(&mut self, ev: &KeyEvent) -> bool {
        let backspace_armed = std::mem::take(&mut self.word_backspace);
        if ev.mods.shift
            || self.editor.is_selecting()
            || self.editor.entering_syllable()
            || self.editor.is_empty()
        {
            return false;
        }
        let c = self.editor.cursor();
        let (key, times) = match ev.key {
            Key::Left if c > 0 && self.sym_letter(c - 1) => (Key::Left, self.letters_before(c)),
            Key::Right if self.sym_letter(c) => (Key::Right, self.letters_after(c)),
            // 完整單字（後面有空白）：第一次刪空白，並記下來
            Key::Backspace if c >= 2 && self.sym_space(c - 1) && self.sym_letter(c - 2) => {
                self.word_backspace = true;
                (Key::Backspace, 1)
            }
            // 剛刪過完整單字後面的空白：刪整個單字
            Key::Backspace if backspace_armed && c > 0 && self.sym_letter(c - 1) => {
                (Key::Backspace, self.letters_before(c))
            }
            // 游標在完整單字前面：Delete 刪整個單字
            Key::Delete if self.sym_letter(c) && self.sym_space(c + self.letters_after(c)) => {
                (Key::Delete, self.letters_after(c))
            }
            _ => return false,
        };
        for _ in 0..times {
            self.chewing_key(key);
        }
        true
    }

    /// ↓ 會作用的位置：游標在最後時是最後一個字
    fn select_target(&self) -> Option<usize> {
        let (c, len) = (self.editor.cursor(), self.editor.len());
        (len > 0).then(|| if c < len { c } else { len - 1 })
    }

    /// B：英文選項目前可以出現嗎（英文對應的是最後一個音節；候選窗必須正在選最後一個字）
    fn english_applicable(&self) -> bool {
        self.mixed.english_candidate
            && self.english_alt.is_some()
            && self.editor.is_selecting()
            && self.editor.cursor() + 1 == self.editor.len()
    }

    /// B：選了英文選項 → 最後一個音節換成英文（仍留在組字區）
    fn apply_english(&mut self) {
        let word = self.english_alt.take().unwrap_or_default();
        let _ = self.editor.cancel_selecting();
        let len = self.editor.len();
        while self.editor.cursor() < len {
            self.chewing_key(Key::Right);
        }
        self.chewing_key(Key::Backspace);
        self.insert_latin(&word);
    }

    fn ai_suggestion(&self) -> Option<&str> {
        self.ai.as_ref()?.suggestion.as_deref()
    }

    /// 送出 AI 修正請求；回傳提示訊息
    fn request_ai(&mut self, request: &mut Option<String>) -> String {
        if !self.ai_enabled {
            return "AI 修正未啟用（設定 → AI）".into();
        }
        let text = self.editor.display();
        if text.is_empty() {
            return "先打出要修正的句子".into();
        }
        self.ai = Some(AiPick {
            original: text.clone(),
            suggestion: None,
        });
        *request = Some(text);
        "AI 修正中…".into()
    }

    /// 服務程序交回 AI 結果；組字內容已經改變（使用者繼續打字）時忽略。回傳是否採用為候選
    pub fn set_ai_suggestion(&mut self, original: &str, suggestion: String) -> bool {
        let current = self.editor.display();
        let valid = self
            .ai
            .as_ref()
            .is_some_and(|a| a.original == original && current == original && suggestion != original);
        if valid {
            self.ai = Some(AiPick {
                original: original.to_owned(),
                suggestion: Some(suggestion),
            });
        } else {
            self.ai = None;
        }
        valid
    }

    /// AI 建議顯示中的按鍵；回傳是否已處理
    fn ai_key(&mut self, ev: &KeyEvent, commit: &mut String) -> bool {
        let Some(suggestion) = self.ai_suggestion().map(str::to_owned) else {
            // 等待中按了其他鍵：組字內容會改變，等結果回來時自然失效
            return false;
        };
        match ev.key {
            Key::Enter | Key::Char(b'1') | Key::Numpad(b'1') if !ev.mods.shift => {
                self.ai = None;
                self.editor.clear();
                self.raw.clear();
                self.english_alt = None;
                self.join(commit, &suggestion);
                true
            }
            Key::Esc => {
                self.ai = None;
                true
            }
            _ => {
                self.ai = None;
                false
            }
        }
    }

    /// 以目前組字內容（不含還在打的注音）當關鍵字開啟 Emoji 選擇；找不到時回傳提示
    fn open_emoji(&mut self) -> Option<String> {
        let keyword = self.editor.display();
        let items = self.engine.emoji().search(&keyword);
        if items.is_empty() {
            return Some(format!("找不到「{keyword}」的 Emoji"));
        }
        self.emoji = Some(EmojiPick {
            items,
            page: 0,
            highlight: 0,
        });
        None
    }

    fn emoji_key(&mut self, ev: &KeyEvent, commit: &mut String) {
        let per_page = self.per_page;
        let Some(pick) = self.emoji.as_mut() else { return };
        let pages = pick.items.len().div_ceil(per_page).max(1);
        let page_len = |page: usize| (pick.items.len() - page * per_page).min(per_page);
        let mut chosen = None;
        match ev.key {
            Key::Esc => {
                self.emoji = None;
                return;
            }
            Key::Down if pick.highlight + 1 < page_len(pick.page) => pick.highlight += 1,
            Key::Down | Key::Right | Key::PageDown | Key::Space => {
                pick.page = (pick.page + 1) % pages;
                pick.highlight = 0;
            }
            Key::Up if pick.highlight > 0 => pick.highlight -= 1,
            Key::Up | Key::Left | Key::PageUp => {
                pick.page = (pick.page + pages - 1) % pages;
                pick.highlight = if ev.key == Key::Up {
                    page_len(pick.page) - 1
                } else {
                    0
                };
            }
            Key::Enter => chosen = Some(pick.highlight),
            Key::Char(c) | Key::Numpad(c) if c.is_ascii_digit() => {
                let d = (c - b'0') as usize;
                let i = if d == 0 { 9 } else { d - 1 };
                if i < page_len(pick.page) {
                    chosen = Some(i);
                }
            }
            _ => {}
        }
        if let Some(i) = chosen {
            let emoji = pick.items[pick.page * per_page + i].clone();
            // 關鍵字（組字內容）由 Emoji 取代
            self.emoji = None;
            self.editor.clear();
            self.raw.clear();
            self.english_alt = None;
            commit.push_str(&emoji);
        }
    }

    /// 送給 chewing 之前，依按鍵更新目前音節的原始按鍵；回傳這個鍵是否完成一個音節
    fn track_raw(&mut self, ev: &KeyEvent, was_selecting: bool) -> bool {
        match ev.key {
            Key::Char(c) if !ev.mods.shift && !was_selecting => {
                if is_tone(c) {
                    self.complete_segment();
                    return true;
                } else {
                    if self.raw.is_empty() {
                        self.raw_english_ok = self.english_position();
                    }
                    self.raw.push(c as char);
                    self.english_alt = None;
                }
            }
            // 有音節在組時空白 = 一聲，音節完成
            Key::Space if !was_selecting && self.editor.entering_syllable() => {
                self.complete_segment();
                return true;
            }
            Key::Space => {}
            // ↓ 開候選窗：保留英文選項
            Key::Down if !was_selecting => {}
            Key::Backspace => {
                self.raw.pop();
                self.english_alt = None;
            }
            // 候選窗中的操作（翻頁、選擇框移動）不影響英文選項
            _ if was_selecting => {}
            _ => {
                self.raw.clear();
                self.english_alt = None;
            }
        }
        false
    }

    fn complete_segment(&mut self) {
        let raw = std::mem::take(&mut self.raw);
        self.english_alt = (self.mixed.english_candidate
            && self.raw_english_ok
            && self.engine.english_candidate(&raw))
        .then_some(raw);
    }

    pub fn process_key(&mut self, ev: &KeyEvent) -> KeyResult {
        let mut r = KeyResult::default();
        let mut commit = String::new();

        // 遊戲模式下 Shift 不切換中英
        if self.shift.observe(ev) && self.game_mode() == GameMode::Off {
            if self.editor.is_empty() || self.emoji.is_some() {
                let text = self.flush();
                self.join(&mut commit, &text);
            } else {
                // 組字中只切換中英、不送出：英文與數字接著放進組字區（「第」Shift「6」Shift「章」）
                self.end_latin_word();
                if self.editor.is_selecting() {
                    let _ = self.editor.cancel_selecting();
                }
                self.editor.cancel_entering_syllable();
                self.raw.clear();
                self.english_alt = None;
            }
            self.mode = self.mode.toggled();
            r.mode_changed = Some(self.mode);
        }

        if self.test_key(ev) {
            r.eaten = true;
            // Enter 送出整句之前：確認這次的選字修正
            if ev.key == Key::Enter
                && !ev.mods.shift
                && !self.editor.is_selecting()
                && self.emoji.is_none()
                && self.ai_suggestion().is_none()
            {
                self.learn_corrections();
            }
            // 正在打的注音：Backspace 取消整個音節（「相ㄑㄧㄣ」→「相」）；
            // 卡住（組不成字）時按 Enter 丟掉這個音節，其餘照常送出（「我ㄍㄟ」→「我」）
            let pending_syllable = !ev.mods.has_command_modifier()
                && self.emoji.is_none()
                && self.ai_suggestion().is_none()
                && self.editor.entering_syllable()
                && !self.editor.is_selecting();
            if pending_syllable && matches!(ev.key, Key::Backspace | Key::Enter) {
                self.editor.cancel_entering_syllable();
                self.raw.clear();
                self.english_alt = None;
                // 只有卡住的注音時，Enter 也不交給 App（不要多換一行）
                if ev.key == Key::Backspace || self.editor.is_empty() {
                    return self.finish(r, commit);
                }
            }
            if ev.key == Key::Down && !self.editor.is_selecting() && self.emoji.is_none() {
                // 游標停在英文上時沒有候選可選
                if self.select_target().is_some_and(|i| self.sym_letter(i) || self.sym_space(i)) {
                    return self.finish(r, commit);
                }
            }
            if self.ai_key(ev, &mut commit) {
            } else if self.emoji.is_some() {
                self.emoji_key(ev, &mut commit);
            } else if self.hotkeys.ai.matches(ev) && self.chinese_hotkey(ev) {
                r.notice = Some(self.request_ai(&mut r.ai_request));
            } else if self.hotkeys.emoji.matches(ev) && self.chinese_hotkey(ev) {
                r.notice = self.open_emoji();
            } else if self.hotkeys.composition_bar.matches(ev) {
                r.toggle_composition_bar = true;
            } else if self.hotkeys.game_mode.matches(ev) {
                let text = self.flush();
                self.join(&mut commit, &text);
                r.toggle_game = true;
            } else if let Some(text) = self.latin_in_composition(ev) {
                self.track_latin(&text);
                self.insert_latin(&text);
            } else if self.hotkeys.forget.matches(ev)
                && let Some(word) = self.english_word_at_cursor()
            {
                // 組字區中游標停在英文單字上：之後不再把它當英文
                self.engine.forget_english(&word);
                r.notice = Some(format!("不再當英文：{word}"));
            } else if self.mode == InputMode::English && self.editor.is_empty() {
                // 只有 E（盤古之白）會走到這裡
                if let Some(c) = self.space_before_latin(ev) {
                    commit.push(' ');
                    commit.push(c);
                }
            } else if self.hotkeys.width.matches(ev) && self.chinese_hotkey(ev) {
                self.width = self.width.toggled();
                self.engine.set_width(self.width);
                r.width_changed = Some(self.width);
            } else if self.hotkeys.output.matches(ev) && self.chinese_hotkey(ev) {
                self.output = self.output.toggled();
                r.output_changed = Some(self.output);
            } else if let Some(word) = self.auto_english(ev).filter(|_| ev.key == Key::Space) {
                // C：英文單字與使用者按的空白都放進組字區
                self.editor.cancel_entering_syllable();
                self.raw.clear();
                self.insert_latin(&format!("{word} "));
            } else if self.word_key(ev) || self.extras_key(ev, &mut commit) {
            } else if let Key::Numpad(c) = ev.key
                && !self.editor.is_empty()
                && !self.editor.is_selecting()
                && !self.editor.entering_syllable()
            {
                // 組字中的數字鍵盤：放進組字區（「第 6 章」整句一起選字、送出）
                self.insert_latin(&(c as char).to_string());
            } else if let Some(text) = self.direct_text(ev) {
                let ascii = text.bytes().all(|b| b.is_ascii_punctuation());
                if (!ascii || !self.editor.is_empty())
                    && !self.editor.entering_syllable()
                    && !self.editor.is_selecting()
                {
                    // 符號放進組字區：全形符號一律放（和 chewing 的中文標點一致），半形符號只在組字中放
                    self.insert_latin(&text);
                } else {
                    // 組字中先送出組字，順序才正確
                    let composed = self.flush();
                    self.join(&mut commit, &composed);
                    self.join(&mut commit, &text);
                }
            } else if let Some(notice) = self.learning_key(ev) {
                r.notice = Some(notice);
            } else if !self.navigate_candidates(ev) {
                let was_selecting = self.editor.is_selecting();
                let key = match ev.key {
                    Key::Numpad(c) => Key::Char(c),
                    k => k,
                };
                let len_before = self.editor.len();
                let was_entering = self.editor.entering_syllable();
                let raw_before = self.raw.clone();
                let completed = self.track_raw(ev, was_selecting);
                match to_chewing(key, ev.mods) {
                    Some(chewing_ev) => {
                        let was_composing = self.is_composing();
                        let behavior = self.editor.process_keyevent(chewing_ev);
                        // 音節組不成字（例如 ㄍㄧˇ）：chewing 會直接清掉注音；
                        // 改成保留注音並提示，讓使用者自己修正（與微軟新注音相同）
                        // 注意：組字超過 39 字時 chewing 會自動送出前面的字，長度一樣不變，
                        // 所以還要確認這次沒有自動送出（否則字送出了、注音卻被補回去）
                        if completed
                            && was_entering
                            && !self.editor.entering_syllable()
                            && self.editor.len() == len_before
                            && self.editor.display_commit().is_empty()
                        {
                            for b in raw_before.bytes() {
                                self.chewing_key(Key::Char(b));
                            }
                            self.raw = raw_before;
                            self.english_alt = None;
                            r.bell = true;
                        } else if completed && self.editor.len() != len_before + 1 {
                            self.english_alt = None;
                        }
                        r.eaten = was_composing || behavior != EditorKeyBehavior::Ignore;
                        // 翻頁、重新開啟或關閉候選窗後，選擇框回到第一個
                        self.highlight = 0;
                    }
                    None => r.eaten = false,
                }
            }
            let text = self.take_commit();
            self.join(&mut commit, &text);
            if !self.is_composing() {
                self.raw.clear();
                self.english_alt = None;
            }
        }
        if !r.eaten {
            self.observe_passthrough(ev);
        }

        let notice = self.editor.notification().to_owned();
        if !notice.is_empty() && r.notice.is_none() {
            r.notice = Some(self.added_phrase_notice(&notice));
        }
        self.finish(r, commit)
    }

    /// chewing 的加詞提示（Shift + ← / → 選範圍 + Enter）：加入的詞設為優先，提示改清楚
    fn added_phrase_notice(&self, notice: &str) -> String {
        let (word, existed) = match (notice.strip_prefix("加入："), notice.strip_prefix("已有：")) {
            (Some(w), _) => (w, false),
            (_, Some(w)) => (w, true),
            _ => return notice.to_owned(),
        };
        let word = word.trim();
        for p in self.engine.phrases().into_iter().filter(|p| p.word == word) {
            let _ = self.engine.set_phrase_state(
                &p.word,
                &p.bopomofo,
                crate::sync::PhraseState::Pinned,
                None,
            );
        }
        if existed {
            format!("已在詞庫：{word}（設為優先）")
        } else {
            format!("已加入詞庫：{word}")
        }
    }

    fn finish(&mut self, mut r: KeyResult, commit: String) -> KeyResult {
        if !self.editor.is_selecting() {
            self.expanded = false;
            self.extras_page = false;
            self.menu = None;
        } else if self.menu.is_none() {
            self.build_menu();
        }
        if !commit.is_empty() {
            self.last_char = commit.chars().last();
            r.commit = Some(self.convert(&commit));
        }
        r.preedit = self.preedit();
        r.candidates = self.candidates();
        r
    }

    /// 候選窗開啟時的操作（候選清單由 ZCT 管理，見 [`Menu`]）
    ///
    /// - ↑ / ↓：移動選擇框，超出本頁就換頁
    /// - ← / →（空白、PageUp / PageDown）：換頁並展開成多欄（與微軟新注音相同），選擇框停在同一行
    /// - Enter / 數字：選字
    /// - 其他鍵（Esc 取消…）交給 chewing
    fn navigate_candidates(&mut self, ev: &KeyEvent) -> bool {
        if !self.editor.is_selecting() || ev.mods.shift || self.extras_page {
            return false;
        }
        let per_page = self.per_page;
        let Some(menu) = self.menu.as_mut() else { return false };
        let len = menu.items.len();
        if len == 0 {
            return false;
        }
        let pages = len.div_ceil(per_page);
        let page = menu.sel / per_page;
        let page_len = |p: usize| (len - p * per_page).min(per_page);
        match ev.key {
            Key::Up => menu.sel = if menu.sel == 0 { len - 1 } else { menu.sel - 1 },
            Key::Down => menu.sel = (menu.sel + 1) % len,
            Key::Left | Key::Right | Key::Space | Key::PageUp | Key::PageDown => {
                let forward = matches!(ev.key, Key::Right | Key::Space | Key::PageDown);
                let row = menu.sel % per_page;
                let to = if forward { (page + 1) % pages } else { (page + pages - 1) % pages };
                menu.sel = to * per_page + row.min(page_len(to) - 1);
                self.expanded = pages > 1;
            }
            Key::Enter => {
                let i = menu.sel;
                self.select_menu(i);
            }
            Key::Char(c) | Key::Numpad(c) if c.is_ascii_digit() => {
                let d = (c - b'0') as usize;
                let row = if d == 0 { 9 } else { d - 1 };
                if row < page_len(page) {
                    self.select_menu(page * per_page + row);
                }
            }
            _ => return false,
        }
        true
    }

    /// 選取候選清單第 `i` 項：換算回 chewing 的（詞長那一組, 頁, 索引）後選字
    fn select_menu(&mut self, i: usize) {
        let Some(menu) = self.menu.as_ref() else { return };
        let Some(item) = menu.items.get(i).cloned() else { return };
        let pick = self.pick_at(i);
        match item.kind {
            MenuKind::English => self.apply_english(),
            MenuKind::Word { group, index } => {
                let per_page = self.per_page;
                let menu = self.menu.as_ref().expect("menu");
                let offset = |g: usize, p: usize| menu.pages[..g].iter().sum::<usize>() + p;
                let total: usize = menu.pages.iter().sum();
                let (g, p) = menu.pos;
                let target = offset(group, index / per_page);
                let presses = (target + total - offset(g, p)) % total.max(1);
                for _ in 0..presses {
                    self.chewing_key(Key::Down);
                }
                let _ = self.editor.select(index % per_page);
            }
        }
        self.menu = None;
        self.highlight = 0;
        self.record_pick(pick);
    }

    /// 選字開始：依序走過 chewing 每一種詞長的候選（長的詞在前、單字在後），合併成一份清單
    ///
    /// 單字依常用程度排序（固頂或學到的字仍在最前），隱藏簡體字。
    fn build_menu(&mut self) {
        let mut groups: Vec<Vec<String>> = Vec::new();
        let mut pages: Vec<usize> = Vec::new();
        let mut wrapped = false;
        for _ in 0..12 {
            let Ok(all) = self.editor.all_candidates() else { break };
            if groups.first() == Some(&all) {
                wrapped = true;
                break;
            }
            let n = self.editor.total_page().unwrap_or(1).max(1);
            groups.push(all);
            pages.push(n);
            // chewing：最後一頁再按 ↓ 換到下一種詞長
            for _ in 0..n {
                self.chewing_key(Key::Down);
            }
        }
        let start = self.editor.cursor();
        let symbols = self.editor.symbols();
        let mut items = Vec::new();
        for (group, words) in groups.iter().enumerate() {
            let mut entries: Vec<(usize, &String)> = words.iter().enumerate().collect();
            let syl = |w: &str| -> Option<Vec<chewing::zhuyin::Syllable>> {
                symbols
                    .get(start..start + w.chars().count())?
                    .iter()
                    .map(|s| s.to_syllable())
                    .collect()
            };
            // 只顯示繁體：含有只屬於簡體的字（机、烛…）的候選都不出現
            entries.retain(|(_, w)| w.chars().all(|c| self.engine.simplified_only(c).is_none()));
            let single = entries.first().is_some_and(|(_, w)| w.chars().count() == 1);
            if single && let Some(reading) = syl(&entries[0].1[..]).and_then(|v| v.first().map(|s| s.to_string())) {
                // 使用者加權的（固頂、學到的）在前；其餘依常用程度的數量級分組，
                // 同一級維持 chewing 的順序（企 86884 / 氣 86740 同級 → 仍是「氣」在前；
                // 重 40186 比 蟲 7618 高一級 → 「重」往前）
                entries.sort_by_key(|(i, w)| {
                    let boost = syl(w)
                        .and_then(|s| self.engine.boost_of(&s, w))
                        .filter(|&b| b > crate::engine::PhraseEntry::NORMAL)
                        .unwrap_or(0);
                    let c = w.chars().next().unwrap_or_default();
                    let tier = (self.engine.char_usage(c, &reading) + 1).ilog10();
                    (std::cmp::Reverse(boost), std::cmp::Reverse(tier), *i)
                });
            }
            items.extend(entries.into_iter().map(|(index, w)| MenuItem {
                word: w.clone(),
                kind: MenuKind::Word { group, index },
            }));
        }
        // 目前畫面上從游標開始的那個詞（或字）排第一：打開選單直接按 Enter 不會改掉原本的內容
        //（犒 → 犒、靠…；天氣 → 天氣、…；游標在詞中間時是那個單字）
        let current = self
            .editor
            .intervals()
            .find(|it| it.start == start)
            .map(|it| it.text.to_string())
            .or_else(|| self.editor.display().chars().nth(start).map(String::from));
        if let Some(current) = current
            && let Some(i) = items.iter().position(|it| it.word == current)
        {
            let it = items.remove(i);
            items.insert(0, it);
        }
        // B：英文選項放在第一頁最後一格
        if self.english_applicable()
            && let Some(w) = &self.english_alt
        {
            let at = (self.per_page - 1).min(items.len());
            items.insert(
                at,
                MenuItem {
                    word: w.clone(),
                    kind: MenuKind::English,
                },
            );
        }
        let pos = if wrapped { (0, 0) } else { (groups.len() % pages.len().max(1), 0) };
        self.menu = Some(Menu {
            items,
            pages,
            pos,
            sel: 0,
        });
    }

    /// 候選窗最後一頁的額外選項：Emoji（以第一個候選詞為關鍵字）與整段注音
    fn extras(&self) -> Vec<Extra> {
        if !self.editor.is_selecting() {
            return Vec::new();
        }
        let mut out: Vec<Extra> = Vec::new();
        if let Some(word) = self.first_word() {
            let emoji = self.engine.emoji();
            if !emoji.is_empty() {
                out.extend(
                    emoji
                        .search(&word)
                        .into_iter()
                        .take(self.per_page.saturating_sub(1))
                        .map(Extra::Emoji),
                );
            }
        }
        let bopomofo = self.bopomofo_text();
        if !bopomofo.is_empty() {
            out.push(Extra::Bopomofo(bopomofo));
        }
        out
    }

    /// 整段組字的注音文字：音節之間以空白分隔，英文與符號保留原樣
    fn bopomofo_text(&self) -> String {
        let mut out = String::new();
        let mut prev_syllable = false;
        for sym in self.editor.symbols() {
            match sym.to_syllable() {
                Some(syl) => {
                    if prev_syllable {
                        out.push(' ');
                    }
                    out.push_str(&syl.to_string());
                    prev_syllable = true;
                }
                None => {
                    out.push_str(&sym.to_char().unwrap_or_default());
                    prev_syllable = false;
                }
            }
        }
        out
    }

    /// 額外選項頁的按鍵：Tab 在候選與額外頁之間切換；數字 / Enter 選取
    fn extras_key(&mut self, ev: &KeyEvent, commit: &mut String) -> bool {
        if !self.editor.is_selecting() || ev.mods.shift {
            return false;
        }
        let extras = self.extras();
        if extras.is_empty() {
            self.extras_page = false;
            // 沒有額外選項時 Tab 什麼都不做（不要交給 chewing）
            return ev.key == Key::Tab;
        }
        if ev.key == Key::Tab {
            self.extras_page = !self.extras_page;
            self.highlight = 0;
            return true;
        }
        if !self.extras_page {
            return false;
        }
        let pick = match ev.key {
            // ← → 回到候選字
            Key::Left | Key::Right | Key::PageUp | Key::PageDown => {
                self.extras_page = false;
                self.highlight = 0;
                return true;
            }
            Key::Up => {
                self.highlight = self.highlight.saturating_sub(1);
                return true;
            }
            Key::Down => {
                self.highlight = (self.highlight + 1).min(extras.len() - 1);
                return true;
            }
            Key::Esc => {
                self.extras_page = false;
                return false;
            }
            Key::Enter => Some(self.highlight),
            Key::Char(c) | Key::Numpad(c) if c.is_ascii_digit() => {
                let d = (c - b'0') as usize;
                Some(if d == 0 { 9 } else { d - 1 })
            }
            _ => return true,
        };
        let Some(extra) = pick.and_then(|i| extras.get(i)).cloned() else {
            return true;
        };
        let text = match extra {
            // Emoji 取代游標處的詞（第一個候選詞的長度）
            Extra::Emoji(e) => {
                let chars: Vec<char> = self.editor.display().chars().collect();
                let start = self.editor.cursor().min(chars.len());
                let n = self.first_word().map_or(1, |w| w.chars().count());
                let end = (start + n).min(chars.len());
                let before: String = chars[..start].iter().collect();
                let after: String = chars[end..].iter().collect();
                format!("{before}{e}{after}")
            }
            Extra::Bopomofo(b) => b,
        };
        self.extras_page = false;
        self.editor.clear();
        self.raw.clear();
        self.english_alt = None;
        self.highlight = 0;
        self.join(commit, &text);
        true
    }





    /// 候選清單第一個詞（Emoji 關鍵字）
    fn first_word(&self) -> Option<String> {
        self.menu
            .as_ref()
            .and_then(|m| m.items.iter().find(|i| i.kind != MenuKind::English))
            .map(|i| i.word.clone())
    }

    /// 候選清單第 `index` 項：字、讀音，以及它是不是改掉了原本的字
    fn pick_at(&self, index: usize) -> Option<Pick> {
        let item = self.menu.as_ref()?.items.get(index)?;
        if item.kind == MenuKind::English {
            return None;
        }
        let word = item.word.clone();
        let n = word.chars().count();
        let start = self.editor.cursor();
        let syllables: Vec<_> = self
            .editor
            .symbols()
            .get(start..start + n)?
            .iter()
            .map(|s| s.to_syllable())
            .collect::<Option<_>>()?;
        let current: String = self.editor.display().chars().skip(start).take(n).collect();
        Some(Pick {
            corrected: current != word,
            syllables,
            word,
        })
    }

    /// 選完字：改成和原本不同的字 = 自動選字錯了一次，送出時再確認要不要學
    fn record_pick(&mut self, pick: Option<Pick>) {
        if let Some(pick) = pick.filter(|p| p.corrected) {
            self.engine.record(0, 1);
            self.picks.push(pick);
        }
        self.engine.mark_dirty();
    }

    /// 送出前：還留在組字裡的修正各算一次（同一個修正兩次才學起來，見 `Engine::note_correction`）
    ///
    /// 單字修正和相鄰的詞一起學（重 + 開機 → 重開機），影響範圍最小；前後都沒有詞時才學單字。
    fn learn_corrections(&mut self) {
        if self.picks.is_empty() {
            return;
        }
        let picks = std::mem::take(&mut self.picks);
        let intervals: Vec<_> = self.editor.intervals().collect();
        let symbols = self.editor.symbols();
        let syllables_of = |it: &chewing::conversion::Interval| -> Option<Vec<chewing::zhuyin::Syllable>> {
            symbols.get(it.start..it.end)?.iter().map(|s| s.to_syllable()).collect()
        };
        for pick in picks {
            let Some(i) = intervals
                .iter()
                .position(|it| *it.text == *pick.word && syllables_of(it).as_ref() == Some(&pick.syllables))
            else {
                continue; // 送出前又改掉了
            };
            if pick.word.chars().count() > 1 {
                self.engine.note_correction(&pick.syllables, &pick.word);
                continue;
            }
            let mut merged = false;
            let neighbors = [i.checked_sub(1).map(|p| (p, i)), Some((i, i + 1))];
            for (a, b) in neighbors.into_iter().flatten() {
                let (Some(x), Some(y)) = (intervals.get(a), intervals.get(b)) else { continue };
                let (Some(sx), Some(sy)) = (syllables_of(x), syllables_of(y)) else { continue };
                let other = if a == i { y } else { x };
                if !other.is_phrase || sx.len() + sy.len() > 4 {
                    continue;
                }
                let word = format!("{}{}", x.text, y.text);
                self.engine.note_correction(&[sx, sy].concat(), &word);
                merged = true;
            }
            if !merged {
                self.engine.note_correction(&pick.syllables, &pick.word);
            }
        }
    }

    /// 候選窗中的「忘記」「固頂」；回傳提示訊息
    fn learning_key(&mut self, ev: &KeyEvent) -> Option<String> {
        if !self.editor.is_selecting() {
            return None;
        }
        let forget = self.hotkeys.forget.matches(ev);
        let pin = self.hotkeys.pin.matches(ev);
        if !forget && !pin {
            return None;
        }
        let (syllables, word) = self.highlighted()?;
        if forget {
            self.engine.forget(&syllables, &word);
            Some(format!("已忘記：{}", self.convert(&word)))
        } else {
            // 先選字再加權：加權後候選順序會改變，選擇框的索引就不再指向同一個詞
            let i = self.menu.as_ref().map_or(0, |m| m.sel);
            self.select_menu(i);
            self.engine.pin(&syllables, &word);
            Some(format!("已固頂：{}", self.convert(&word)))
        }
    }

    /// 選擇框所在的候選詞與它的讀音（候選詞從游標位置開始）
    fn highlighted(&self) -> Option<(Vec<chewing::zhuyin::Syllable>, String)> {
        let pick = self.pick_at(self.menu.as_ref()?.sel)?;
        Some((pick.syllables, pick.word))
    }

    fn chewing_key(&mut self, key: Key) {
        if let Some(ev) = to_chewing(key, Mods::NONE) {
            self.editor.process_keyevent(ev);
        }
    }

    /// 送出的文字：先把只屬於簡體的字換成繁體，簡體模式再整句轉成簡體
    fn convert(&self, text: &str) -> String {
        let text = self.engine.to_traditional(text);
        match (self.output, simplifier()) {
            (Output::Simplified, Some(cc)) => cc.convert(&text),
            _ => text,
        }
    }

    /// 送出目前所有組字內容（失焦、切換中英時使用）；回傳的是 chewing 原始（繁體）文字
    pub fn flush(&mut self) -> String {
        if self.editor.is_selecting() {
            let _ = self.editor.cancel_selecting();
        }
        self.learn_corrections();
        self.editor.cancel_entering_syllable();
        if !self.editor.is_empty() {
            let _ = self.editor.commit();
        }
        let text = self.take_commit();
        self.editor.clear();
        text
    }

    /// 丟棄組字內容與 Shift 狀態（App 中止組字時）
    pub fn reset(&mut self) {
        self.editor.clear();
        self.shift.reset();
        self.highlight = 0;
        self.raw.clear();
        self.english_alt = None;
        self.last_char = None;
        self.emoji = None;
        self.ai = None;
        self.extras_page = false;
        self.word_backspace = false;
        self.latin_word.clear();
        self.shift_words.clear();
        self.picks.clear();
        self.menu = None;
    }

    /// 輸入焦點回到這個 context：依 `on_focus` 決定是否重設中英狀態
    pub fn focus(&mut self) {
        self.reset();
        if self.eff.on_focus == OnFocus::Reset {
            self.mode = self.eff.default_mode;
            self.width = match self.rule_width {
                Some(w) => w,
                None => self.engine.width().unwrap_or(self.eff.width),
            };
        }
    }

    pub fn preedit(&self) -> Preedit {
        let converted: Vec<char> = self.editor.display().chars().collect();
        let syllable = self.editor.syllable_buffer_display();
        let cursor = self.editor.cursor().min(converted.len());
        // 自動轉換偶爾會選到只屬於簡體的字（手机）：一律顯示成繁體（一對一換字，位置不變）
        let before = self.engine.to_traditional(&converted[..cursor].iter().collect::<String>());
        let after = self.engine.to_traditional(&converted[cursor..].iter().collect::<String>());
        let shift = before.chars().count() + syllable.chars().count();
        // 選字目標：候選窗開啟時是正在選的詞；否則是游標後的字（英文單字整個）；
        // 游標在最後或正在打注音時不標示（使用者可能根本不打算選字）
        let target = if self.editor.is_selecting() {
            let n = self
                .menu
                .as_ref()
                .and_then(|m| m.items.get(m.sel))
                .map_or(1, |i| i.word.chars().count());
            Some((cursor, n))
        } else if syllable.is_empty() && cursor < converted.len() {
            let n = self.letters_after(cursor).max(1);
            Some((shift, n))
        } else {
            None
        };
        Preedit {
            cursor: shift,
            text: format!("{before}{syllable}{after}"),
            target,
            len: converted.len(),
        }
    }

    pub fn candidates(&self) -> Option<CandidatePage> {
        if let Some(s) = self.ai_suggestion() {
            return Some(CandidatePage {
                items: vec![s.to_owned()],
                page: 0,
                total_pages: 1,
                highlight: 0,
                pinned: vec![false],
                ..Default::default()
            });
        }
        if let Some(pick) = &self.emoji {
            let start = pick.page * self.per_page;
            let items = pick.items[start..(start + self.per_page).min(pick.items.len())].to_vec();
            return Some(CandidatePage {
                pinned: vec![false; items.len()],
                items,
                page: pick.page,
                total_pages: pick.items.len().div_ceil(self.per_page).max(1),
                highlight: pick.highlight,
                ..Default::default()
            });
        }
        if !self.editor.is_selecting() {
            return None;
        }
        if self.extras_page {
            let extras = self.extras();
            if !extras.is_empty() {
                let items: Vec<String> = extras.iter().map(|e| e.label().to_owned()).collect();
                return Some(CandidatePage {
                    pinned: vec![false; items.len()],
                    items,
                    page: 0,
                    total_pages: 1,
                    highlight: self.highlight,
                    ..Default::default()
                });
            }
        }
        let menu = self.menu.as_ref()?;
        let per_page = self.per_page;
        let total_pages = menu.items.len().div_ceil(per_page).max(1);
        let page = menu.sel / per_page;
        let slice = &menu.items[(page * per_page).min(menu.items.len())..((page + 1) * per_page).min(menu.items.len())];
        let start = self.editor.cursor();
        let symbols = self.editor.symbols();
        let pinned = slice
            .iter()
            .map(|it| {
                it.kind != MenuKind::English
                    && symbols
                        .get(start..start + it.word.chars().count())
                        .and_then(|s| s.iter().map(|s| s.to_syllable()).collect::<Option<Vec<_>>>())
                        .and_then(|syl| self.engine.boost_of(&syl, &it.word))
                        .is_some_and(|b| b >= crate::engine::PhraseEntry::PINNED)
            })
            .collect();
        let (columns, first_column) = if self.expanded {
            const MAX_COLUMNS: usize = 6;
            let first = page / MAX_COLUMNS * MAX_COLUMNS;
            let columns = menu
                .items
                .chunks(per_page)
                .skip(first)
                .take(MAX_COLUMNS)
                .map(|c| c.iter().map(|i| i.word.clone()).collect())
                .collect();
            (columns, first)
        } else {
            (Vec::new(), 0)
        };
        Some(CandidatePage {
            items: slice.iter().map(|i| i.word.clone()).collect(),
            page,
            total_pages,
            highlight: menu.sel % per_page,
            pinned,
            columns,
            first_column,
        })
    }



    fn take_commit(&mut self) -> String {
        let text = self.editor.display_commit().to_owned();
        if !text.is_empty() {
            self.learn_shift_words(&text);
            self.editor.ack();
            // chewing 送出時會更新學習紀錄（記憶體中），存檔交給服務程序
            self.engine.record(text.chars().count() as u64, 0);
        }
        text
    }
}

/// `text` 中有沒有完整的英文單字 `word`（前後不是英文字母）
fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphabetic()) && !after.is_some_and(|c| c.is_ascii_alphabetic())
    })
}

/// 全半形初始值：App 規則明確指定 > 上次切換的狀態 > 設定檔預設
fn initial_width(opts: &SessionOptions, eff: &Effective) -> Width {
    match opts.rule.as_ref().and_then(|r| r.punctuation) {
        Some(w) => w,
        None => opts.engine.width().unwrap_or(eff.width),
    }
}

/// 一次選字：候選字、讀音，以及它是否改掉了原本的字
#[derive(Debug, Clone)]
struct Pick {
    syllables: Vec<chewing::zhuyin::Syllable>,
    word: String,
    corrected: bool,
}

/// 候選清單（選字中）：詞與單字合併成同一份，由 ZCT 自己翻頁與選擇
#[derive(Debug, Clone)]
struct Menu {
    items: Vec<MenuItem>,
    /// chewing 每一種詞長（長→短）的頁數；選字時用來換算要按幾次 ↓
    pages: Vec<usize>,
    /// chewing 目前停在（第幾種詞長, 第幾頁）
    pos: (usize, usize),
    /// 選擇框（清單中的絕對位置）
    sel: usize,
}

#[derive(Debug, Clone)]
struct MenuItem {
    word: String,
    kind: MenuKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuKind {
    /// chewing 第 `group` 種詞長的第 `index` 個候選
    Word { group: usize, index: usize },
    /// B：最後一個音節換成英文
    English,
}
