//! 一個輸入 context 的輸入狀態：中英模式、Shift 判定、chewing 組字

use std::path::PathBuf;

use chewing::editor::{BasicEditor, Editor, EditorKeyBehavior};
use serde::{Deserialize, Serialize};

use crate::config::{Width, Config, InputMode};
use crate::key::{Key, KeyEvent, shifted, to_chewing};
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
    /// 這次按鍵切換了標點全半形（平台層顯示「全 / 半」提示）
    pub width_changed: Option<Width>,
}

#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// 系統詞庫（`static_dict.bin` 等）所在資料夾
    pub sys_dict_dir: PathBuf,
    /// 使用者詞庫與學習資料所在資料夾
    pub user_dir: PathBuf,
    pub config: Config,
}

#[derive(Debug)]
pub struct SessionError(String);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "無法建立輸入 session：{}", self.0)
    }
}

impl std::error::Error for SessionError {}

#[derive(Debug)]
pub struct Session {
    editor: Editor,
    mode: InputMode,
    width: Width,
    shift: ShiftTap,
    per_page: usize,
    highlight: usize,
}

impl Session {
    pub fn new(opts: &SessionOptions) -> Result<Session, SessionError> {
        std::fs::create_dir_all(&opts.user_dir).map_err(|e| SessionError(e.to_string()))?;
        let mut editor = Editor::chewing(
            Some(opts.sys_dict_dir.to_string_lossy().into_owned()),
            Some(opts.user_dir.to_string_lossy().into_owned()),
        )
        .map_err(|e| SessionError(e.to_string()))?;

        let per_page = opts.config.candidates.per_page.clamp(1, 10);
        editor.set_editor_options(|o| {
            o.candidates_per_page = per_page;
            o.space_is_select_key = true;
            // 選完字後游標跳到該詞後面（今天|天氣很好 → 今天天氣|很好）
            o.auto_shift_cursor = true;
            // 全半形切換由 ZCT 自己處理（M3），避免 chewing 吃掉 Shift+Space
            o.enable_fullwidth_toggle_key = false;
        });

        Ok(Session {
            editor,
            mode: opts.config.input.default_mode,
            width: opts.config.input.punctuation,
            shift: ShiftTap::new(opts.config.toggle.shift, opts.config.toggle.shift_tap_max_ms),
            per_page,
            highlight: 0,
        })
    }

    pub fn mode(&self) -> InputMode {
        self.mode
    }

    pub fn width(&self) -> Width {
        self.width
    }

    /// Shift + 空白鍵：切換標點全半形（與微軟新注音相同）
    fn is_width_toggle(ev: &KeyEvent) -> bool {
        ev.key == Key::Space && ev.mods.shift
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

    pub fn is_composing(&self) -> bool {
        !self.editor.is_empty() || self.editor.entering_syllable() || self.editor.is_selecting()
    }

    /// 只判斷「按下」事件是否會被吃掉，不改變任何狀態（給 TSF `OnTestKeyDown` 用）
    ///
    /// 放開事件請直接呼叫 [`Session::process_key`]：放開事件從不被吃掉，但 Shift 判定需要它。
    pub fn test_key(&self, ev: &KeyEvent) -> bool {
        if ev.released || ev.key.is_modifier() || ev.key == Key::Other {
            return false;
        }
        // P3：Ctrl / Alt / Meta 組合鍵一律交給 App
        if ev.mods.has_command_modifier() {
            return false;
        }
        if self.mode == InputMode::English {
            return false;
        }
        if Self::is_width_toggle(ev) {
            return true;
        }
        if self.is_composing() {
            return true;
        }
        // 沒有組字時，只有字元鍵會開始組字；Enter、空白、方向鍵等交給 App
        matches!(ev.key, Key::Char(_))
    }

    /// 給 TSF `OnTestKeyDown` 用：判斷是否會吃掉；不吃的鍵仍要讓 Shift 判定看到
    ///
    /// TSF 對不吃的鍵不會再呼叫 `OnKeyDown`，若不在這裡觀察，
    /// 英文模式下「按住 Shift 打字母」放開後會被誤判成單按 Shift。重複呼叫是安全的。
    pub fn test_key_down(&mut self, ev: &KeyEvent) -> bool {
        let eat = self.test_key(ev);
        if !eat {
            self.shift.observe(ev);
        }
        eat
    }

    pub fn process_key(&mut self, ev: &KeyEvent) -> KeyResult {
        let mut commit = String::new();
        let mut mode_changed = None;

        if self.shift.observe(ev) {
            commit.push_str(&self.flush());
            self.mode = self.mode.toggled();
            mode_changed = Some(self.mode);
        }

        let mut eaten = false;
        let mut width_changed = None;
        if self.test_key(ev) {
            if Self::is_width_toggle(ev) {
                self.width = self.width.toggled();
                width_changed = Some(self.width);
                eaten = true;
            } else if let Some(c) = self.halfwidth_symbol(ev) {
                // 由輸入法送出半形標點；組字中則先送出組字，順序才正確
                commit.push_str(&self.flush());
                commit.push(c as char);
                eaten = true;
            } else if self.navigate_candidates(ev) {
                eaten = true;
            } else if let Some(chewing_ev) = to_chewing(ev.key, ev.mods) {
                let was_composing = self.is_composing();
                let behavior = self.editor.process_keyevent(chewing_ev);
                eaten = was_composing || behavior != EditorKeyBehavior::Ignore;
                // 翻頁、重新開啟或關閉候選窗後，選擇框回到第一個
                self.highlight = 0;
            }
            commit.push_str(&self.take_commit());
        }

        let mut r = self.result(eaten, commit, mode_changed);
        r.width_changed = width_changed;
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
                let _ = self.editor.select(self.highlight.min(len - 1));
                self.highlight = 0;
            }
            _ => return false,
        }
        true
    }

    fn chewing_key(&mut self, key: Key) {
        if let Some(ev) = to_chewing(key, crate::key::Mods::NONE) {
            self.editor.process_keyevent(ev);
        }
    }

    /// 送出目前所有組字內容（失焦、切換中英時使用）
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

    /// 丟棄組字內容與 Shift 狀態
    pub fn reset(&mut self) {
        self.editor.clear();
        self.shift.reset();
    }

    pub fn preedit(&self) -> Preedit {
        let converted: Vec<char> = self.editor.display().chars().collect();
        let syllable = self.editor.syllable_buffer_display();
        let cursor = self.editor.cursor().min(converted.len());
        let mut text: String = converted[..cursor].iter().collect();
        text.push_str(&syllable);
        text.extend(&converted[cursor..]);
        Preedit {
            text,
            cursor: cursor + syllable.chars().count(),
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
            .collect();
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
        }
        text
    }

    fn result(&self, eaten: bool, commit: String, mode_changed: Option<InputMode>) -> KeyResult {
        KeyResult {
            eaten,
            commit: (!commit.is_empty()).then_some(commit),
            preedit: self.preedit(),
            candidates: self.candidates(),
            mode_changed,
            width_changed: None,
        }
    }
}
