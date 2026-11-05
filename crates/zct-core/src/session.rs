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

use chewing::editor::{BasicEditor, Editor, EditorKeyBehavior};
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
use crate::key::{Key, KeyEvent, Mods, shifted, to_chewing};
use crate::shift::ShiftTap;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Preedit {
    pub text: String,
    /// 游標位置（以字元計）
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidatePage {
    pub items: Vec<String>,
    /// 從 0 開始
    pub page: usize,
    pub total_pages: usize,
    /// 目前選擇框所在的項目（本頁內的索引），Enter 選它
    pub highlight: usize,
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
    /// 這次按鍵切換了繁簡輸出（平台層顯示「繁 / 简」提示）
    pub output_changed: Option<Output>,
    /// 使用者按了遊戲模式快捷鍵；遊戲模式是全域狀態，由服務程序切換並廣播
    pub toggle_game: bool,
    /// 要顯示給使用者的短訊息（例如「加入：天氣」「已固頂：氣」）
    pub notice: Option<String>,
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

#[derive(Debug)]
pub struct Session {
    engine: Arc<Engine>,
    editor: Editor,
    eff: Effective,
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
    /// B：剛完成的音節按鍵也是英文單字 → 候選窗附上的英文
    english_alt: Option<String>,
    /// E：游標前最後一個字（本輸入法送出的，或交給 App 的按鍵）；未知時為 None
    last_char: Option<char>,
}

impl Session {
    pub fn new(opts: &SessionOptions) -> Result<Session, SessionError> {
        let mut editor: Editor = opts.engine.editor();
        let sort = opts.config.candidates.sort_by_frequency;

        let per_page = opts.config.candidates.per_page.clamp(1, 10);
        editor.set_editor_options(|o| {
            o.candidates_per_page = per_page;
            o.space_is_select_key = true;
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
            width: eff.width,
            output: eff.output,
            global_game: false,
            shift: ShiftTap::new(opts.config.toggle.shift, opts.config.toggle.shift_tap_max_ms),
            hotkeys: opts.config.hotkeys.clone(),
            punct: opts.config.punctuation.compile().map_err(SessionError)?,
            per_page,
            highlight: 0,
            mixed: opts.config.mixed.clone(),
            raw: String::new(),
            english_alt: None,
            last_char: None,
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

    pub fn set_global_game(&mut self, on: bool) {
        self.global_game = on;
    }

    pub fn is_composing(&self) -> bool {
        !self.editor.is_empty() || self.editor.entering_syllable() || self.editor.is_selecting()
    }

    fn chinese_hotkey(&self, ev: &KeyEvent) -> bool {
        self.mode == InputMode::Chinese
            && self.game_mode() != GameMode::English
            && (self.hotkeys.width.matches(ev) || self.hotkeys.output.matches(ev))
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
        let Key::Char(c) = ev.key else { return None };
        // Caps Lock：字母直接輸出英文（與微軟新注音相同）
        if ev.mods.caps_lock && c.is_ascii_alphabetic() {
            let upper = !ev.mods.shift;
            let c = if upper { c.to_ascii_uppercase() } else { c };
            return Some((c as char).to_string());
        }
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
        self.halfwidth_symbol(ev).map(|c| (c as char).to_string())
    }

    /// 半形模式下，這個鍵要輸出的 ASCII 標點；字母、注音鍵與 ` 符號選單不受影響
    fn halfwidth_symbol(&self, ev: &KeyEvent) -> Option<u8> {
        if self.width != Width::Half {
            return None;
        }
        match ev.key {
            Key::Char(c) if ev.mods.shift && !c.is_ascii_alphabetic() => Some(shifted(c)),
            // 大千排列中不是注音的鍵
            Key::Char(c @ (b'[' | b']' | b'\\' | b'\'' | b'=')) if !ev.mods.shift => Some(c),
            _ => None,
        }
    }

    /// 只判斷「按下」事件是否會被吃掉，不改變任何狀態（給 TSF `OnTestKeyDown` 用）
    ///
    /// 放開事件請直接呼叫 [`Session::process_key`]：放開事件從不被吃掉，但 Shift 判定需要它。
    pub fn test_key(&self, ev: &KeyEvent) -> bool {
        if ev.released || ev.key.is_modifier() || ev.key == Key::Other {
            return false;
        }
        if self.hotkeys.game_mode.matches(ev) || self.chinese_hotkey(ev) {
            return true;
        }
        if self.game_mode() == GameMode::English {
            return false;
        }
        // P3：沒有綁定的 Ctrl / Alt / Meta 組合鍵一律交給 App
        if ev.mods.has_command_modifier() {
            return false;
        }
        if self.mode == InputMode::English {
            // E：英文模式下，緊接在中文後的第一個英數字要由輸入法補空格
            return self.space_before_latin(ev).is_some();
        }
        if self.is_composing() {
            return !matches!(ev.key, Key::F(_));
        }
        match ev.key {
            // 沒在組字時，數字鍵盤與 Caps Lock 字母由 App 自己處理
            Key::Numpad(_) => false,
            Key::Char(c) if ev.mods.caps_lock && c.is_ascii_alphabetic() => false,
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

    /// C：按空白時，剛打的按鍵若是英文單字且不是合法注音 → 回傳該單字
    fn auto_english(&self, ev: &KeyEvent) -> Option<String> {
        let ok = self.mixed.auto_english
            && ev.key == Key::Space
            && !ev.mods.shift
            && !self.editor.is_selecting()
            && self.editor.entering_syllable()
            && self.engine.words().auto_english(&self.raw);
        ok.then(|| self.raw.clone())
    }

    /// B：候選窗中英文選項的位置（本頁索引）
    fn english_index(&self) -> Option<usize> {
        if !self.mixed.english_candidate
            || self.english_alt.is_none()
            || !self.editor.is_selecting()
            || self.editor.current_page_no().unwrap_or(0) != 0
            // 英文對應的是最後一個音節；候選窗必須正在選最後一個字
            || self.editor.cursor() + 1 != self.editor.len()
        {
            return None;
        }
        let n = self.editor.paginated_candidates().ok()?.len().min(self.per_page);
        Some(n)
    }

    /// B：選了英文選項 → 最後一個音節換成英文送出
    fn pick_english(&mut self, ev: &KeyEvent, commit: &mut String) -> bool {
        let Some(i) = self.english_index() else { return false };
        let chosen = match ev.key {
            Key::Enter => self.highlight == i,
            Key::Char(c) | Key::Numpad(c) if c.is_ascii_digit() => {
                let d = (c - b'0') as usize;
                (if d == 0 { 9 } else { d - 1 }) == i
            }
            _ => false,
        };
        if !chosen || ev.mods.shift {
            return false;
        }
        let word = self.english_alt.take().unwrap_or_default();
        let text = self.editor.display();
        let n = text.chars().count();
        let prefix: String = text.chars().take(n.saturating_sub(1)).collect();
        self.editor.clear();
        self.highlight = 0;
        self.join(commit, &prefix);
        self.join(commit, &word);
        true
    }

    /// 送給 chewing 之前，依按鍵更新目前音節的原始按鍵；回傳這個鍵是否完成一個音節
    fn track_raw(&mut self, ev: &KeyEvent, was_selecting: bool) -> bool {
        match ev.key {
            Key::Char(c) if !ev.mods.shift && !was_selecting => {
                if is_tone(c) {
                    self.complete_segment();
                    return true;
                } else {
                    self.raw.push(c as char);
                    self.english_alt = None;
                }
            }
            // 有音節在組時空白 = 一聲，音節完成；否則空白是開候選窗，保留英文選項
            Key::Space if !was_selecting && self.editor.entering_syllable() => {
                self.complete_segment();
                return true;
            }
            Key::Space => {}
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
        self.english_alt =
            (self.mixed.english_candidate && self.engine.words().candidate(&raw)).then_some(raw);
    }

    pub fn process_key(&mut self, ev: &KeyEvent) -> KeyResult {
        let mut r = KeyResult::default();
        let mut commit = String::new();

        // 遊戲模式下 Shift 不切換中英
        if self.shift.observe(ev) && self.game_mode() == GameMode::Off {
            let text = self.flush();
            self.join(&mut commit, &text);
            self.mode = self.mode.toggled();
            r.mode_changed = Some(self.mode);
        }

        if self.test_key(ev) {
            r.eaten = true;
            if self.hotkeys.game_mode.matches(ev) {
                let text = self.flush();
                self.join(&mut commit, &text);
                r.toggle_game = true;
            } else if self.mode == InputMode::English {
                // 只有 E（盤古之白）會走到這裡
                if let Some(c) = self.space_before_latin(ev) {
                    commit.push(' ');
                    commit.push(c);
                }
            } else if self.hotkeys.width.matches(ev) && self.chinese_hotkey(ev) {
                self.width = self.width.toggled();
                r.width_changed = Some(self.width);
            } else if self.hotkeys.output.matches(ev) && self.chinese_hotkey(ev) {
                self.output = self.output.toggled();
                r.output_changed = Some(self.output);
            } else if let Some(word) = self.auto_english(ev) {
                // C：送出前面的中文與這個英文單字，空白鍵本身也送出
                self.editor.cancel_entering_syllable();
                self.raw.clear();
                let chinese = self.flush();
                self.join(&mut commit, &chinese);
                self.join(&mut commit, &word);
                commit.push(' ');
            } else if self.pick_english(ev, &mut commit) {
            } else if let Some(text) = self.direct_text(ev) {
                // 組字中先送出組字，順序才正確
                let composed = self.flush();
                self.join(&mut commit, &composed);
                self.join(&mut commit, &text);
            } else if let Some(notice) = self.learning_key(ev) {
                r.notice = Some(notice);
            } else if !self.navigate_candidates(ev) {
                let was_selecting = self.editor.is_selecting();
                let picked = self.candidate_index(ev);
                let key = match ev.key {
                    Key::Numpad(c) => Key::Char(c),
                    k => k,
                };
                let len_before = self.editor.len();
                let completed = self.track_raw(ev, was_selecting);
                match to_chewing(key, ev.mods) {
                    Some(chewing_ev) => {
                        let was_composing = self.is_composing();
                        let behavior = self.editor.process_keyevent(chewing_ev);
                        // 音節沒有真的組成字（例如 ㄕㄟ 沒有一聲）：英文選項不能對應到前一個字
                        if completed && self.editor.len() != len_before + 1 {
                            self.english_alt = None;
                        }
                        r.eaten = was_composing || behavior != EditorKeyBehavior::Ignore;
                        if was_selecting && !self.editor.is_selecting() {
                            self.record_pick(picked);
                        }
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

        let notice = self.editor.notification();
        if !notice.is_empty() && r.notice.is_none() {
            r.notice = Some(notice.to_owned());
        }
        if !commit.is_empty() {
            self.last_char = commit.chars().last();
            r.commit = Some(self.convert(&commit));
        }
        r.preedit = self.preedit();
        r.candidates = self.candidates();
        r
    }

    /// 候選窗開啟時的選擇框操作（chewing 本身沒有選擇框）
    ///
    /// - ↑ / ↓：移動選擇框，超出本頁就翻頁
    /// - Enter：選擇框所在的項目
    /// - 其他鍵（← → 翻頁、數字直接選、Esc 取消…）交給 chewing
    fn navigate_candidates(&mut self, ev: &KeyEvent) -> bool {
        if !self.editor.is_selecting() || ev.mods.shift {
            return false;
        }
        let len = self.candidates().map(|p| p.items.len()).unwrap_or(0);
        if len == 0 {
            return false;
        }
        match ev.key {
            Key::Up if self.highlight > 0 => self.highlight -= 1,
            Key::Up => {
                self.chewing_key(Key::Left);
                let len = self.candidates().map(|p| p.items.len()).unwrap_or(1);
                self.highlight = len.saturating_sub(1);
            }
            Key::Down if self.highlight + 1 < len => self.highlight += 1,
            Key::Down => {
                self.chewing_key(Key::Down);
                self.highlight = 0;
            }
            Key::Enter => {
                let picked = self.page_offset() + self.highlight.min(len - 1);
                let _ = self.editor.select(self.highlight.min(len - 1));
                self.highlight = 0;
                self.record_pick(Some(picked));
            }
            _ => return false,
        }
        true
    }

    fn page_offset(&self) -> usize {
        self.editor.current_page_no().unwrap_or(0) * self.per_page
    }

    /// 數字鍵選字時，選的是第幾個候選（跨頁的絕對位置）
    fn candidate_index(&self, ev: &KeyEvent) -> Option<usize> {
        if !self.editor.is_selecting() {
            return None;
        }
        let digit = match ev.key {
            Key::Char(c) | Key::Numpad(c) if c.is_ascii_digit() => c - b'0',
            _ => return None,
        };
        let i = if digit == 0 { 9 } else { digit as usize - 1 };
        Some(self.page_offset() + i)
    }

    /// 選了第一個以外的候選 = 自動選字錯了一次
    fn record_pick(&self, picked: Option<usize>) {
        if picked.is_some_and(|i| i > 0) {
            self.engine.record(0, 1);
        }
        self.engine.mark_dirty();
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
            let _ = self.editor.select(self.highlight);
            self.highlight = 0;
            self.engine.pin(&syllables, &word);
            Some(format!("已固頂：{}", self.convert(&word)))
        }
    }

    /// 選擇框所在的候選詞與它的讀音（候選詞從游標位置開始）
    fn highlighted(&self) -> Option<(Vec<chewing::zhuyin::Syllable>, String)> {
        let word = self
            .editor
            .paginated_candidates()
            .ok()?
            .into_iter()
            .nth(self.highlight)?;
        let n = word.chars().count();
        let start = self.editor.cursor();
        let syllables: Option<Vec<_>> = self
            .editor
            .symbols()
            .get(start..start + n)?
            .iter()
            .map(|s| s.to_syllable())
            .collect();
        Some((syllables?, word))
    }

    fn chewing_key(&mut self, key: Key) {
        if let Some(ev) = to_chewing(key, Mods::NONE) {
            self.editor.process_keyevent(ev);
        }
    }

    /// 依目前輸出字集轉換顯示與送出的文字
    fn convert(&self, text: &str) -> String {
        match (self.output, simplifier()) {
            (Output::Simplified, Some(cc)) => cc.convert(text),
            _ => text.to_owned(),
        }
    }

    /// 送出目前所有組字內容（失焦、切換中英時使用）；回傳的是 chewing 原始（繁體）文字
    pub fn flush(&mut self) -> String {
        if self.editor.is_selecting() {
            let _ = self.editor.cancel_selecting();
        }
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
    }

    /// 輸入焦點回到這個 context：依 `on_focus` 決定是否重設中英狀態
    pub fn focus(&mut self) {
        self.reset();
        if self.eff.on_focus == OnFocus::Reset {
            self.mode = self.eff.default_mode;
            self.width = self.eff.width;
        }
    }

    pub fn preedit(&self) -> Preedit {
        let converted: Vec<char> = self.editor.display().chars().collect();
        let syllable = self.editor.syllable_buffer_display();
        let cursor = self.editor.cursor().min(converted.len());
        let before: String = converted[..cursor].iter().collect();
        let after: String = converted[cursor..].iter().collect();
        let before = self.convert(&before);
        let after = self.convert(&after);
        Preedit {
            cursor: before.chars().count() + syllable.chars().count(),
            text: format!("{before}{syllable}{after}"),
        }
    }

    pub fn candidates(&self) -> Option<CandidatePage> {
        if !self.editor.is_selecting() {
            return None;
        }
        let items = self
            .editor
            .paginated_candidates()
            .ok()?
            .into_iter()
            .take(self.per_page)
            .map(|c| self.convert(&c))
            .collect::<Vec<_>>();
        let mut items = items;
        if self.english_index().is_some()
            && let Some(w) = &self.english_alt
        {
            items.push(w.clone());
        }
        Some(CandidatePage {
            items,
            page: self.editor.current_page_no().unwrap_or(0),
            total_pages: self.editor.total_page().unwrap_or(1),
            highlight: self.highlight,
        })
    }

    fn take_commit(&mut self) -> String {
        let text = self.editor.display_commit().to_owned();
        if !text.is_empty() {
            self.editor.ack();
            // chewing 送出時會更新學習紀錄（記憶體中），存檔交給服務程序
            self.engine.record(text.chars().count() as u64, 0);
        }
        text
    }
}
