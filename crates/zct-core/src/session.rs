//! 一個輸入 context 的輸入狀態：中英模式、Shift 判定、chewing 組字

use std::path::PathBuf;

use chewing::editor::{BasicEditor, Editor, EditorKeyBehavior};

use crate::config::{Config, InputMode};
use crate::key::{Key, KeyEvent, to_chewing};
use crate::shift::ShiftTap;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preedit {
    pub text: String,
    /// 游標位置（以字元計）
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePage {
    pub items: Vec<String>,
    /// 從 0 開始
    pub page: usize,
    pub total_pages: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyResult {
    /// `false` → 平台層要把按鍵交還給 App
    pub eaten: bool,
    /// 要送出到 App 的文字
    pub commit: Option<String>,
    pub preedit: Preedit,
    pub candidates: Option<CandidatePage>,
    /// 這次按鍵切換了中英狀態（平台層顯示「中 / A」提示）
    pub mode_changed: Option<InputMode>,
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
    shift: ShiftTap,
    per_page: usize,
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
            // 全半形切換由 ZCT 自己處理（M3），避免 chewing 吃掉 Shift+Space
            o.enable_fullwidth_toggle_key = false;
        });

        Ok(Session {
            editor,
            mode: opts.config.input.default_mode,
            shift: ShiftTap::new(opts.config.toggle.shift, opts.config.toggle.shift_tap_max_ms),
            per_page,
        })
    }

    pub fn mode(&self) -> InputMode {
        self.mode
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
        if self.is_composing() {
            return true;
        }
        // 沒有組字時，只有字元鍵會開始組字；Enter、空白、方向鍵等交給 App
        matches!(ev.key, Key::Char(_))
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
        if self.test_key(ev)
            && let Some(chewing_ev) = to_chewing(ev.key, ev.mods)
        {
            let was_composing = self.is_composing();
            let behavior = self.editor.process_keyevent(chewing_ev);
            eaten = was_composing || behavior != EditorKeyBehavior::Ignore;
            commit.push_str(&self.take_commit());
        }

        self.result(eaten, commit, mode_changed)
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
        }
    }
}
