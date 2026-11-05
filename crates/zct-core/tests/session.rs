//! Session 整合測試（需要先執行 scripts/fetch-dict.sh 下載詞庫）

use zct_core::config::{Config, InputMode};
use zct_core::{Key, KeyEvent, KeyResult, Mods, Session, SessionOptions, Side};

const DICT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/dict");

fn open_engine(dir: &std::path::Path) -> zct_core::Engine {
    zct_core::Engine::open(std::path::Path::new(DICT), dir).expect("需要先執行 scripts/fetch-dict.sh")
}

struct Harness {
    s: Session,
    t: u64,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Harness {
        Harness::with_config(Config::default())
    }

    fn with_config(config: Config) -> Harness {
        Harness::with_rule(config, None)
    }

    fn with_rule(config: Config, rule: Option<zct_core::AppRule>) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let engine = std::sync::Arc::new(open_engine(&dir.path().join("learning")));
        Harness::with_engine(engine, config, rule, dir)
    }

    fn with_engine(
        engine: std::sync::Arc<zct_core::Engine>,
        config: Config,
        rule: Option<zct_core::AppRule>,
        dir: tempfile::TempDir,
    ) -> Harness {
        let opts = SessionOptions {
            engine,
            config,
            rule,
        };
        Harness {
            s: Session::new(&opts).expect("需要先執行 scripts/fetch-dict.sh"),
            t: 1_000,
            _dir: dir,
        }
    }

    /// 按下並放開一個鍵，回傳按下時的結果
    fn press(&mut self, key: Key, mods: Mods) -> KeyResult {
        self.t += 30;
        let r = self.s.process_key(&KeyEvent::down(key, mods, self.t));
        self.t += 30;
        self.s.process_key(&KeyEvent::up(key, mods, self.t));
        r
    }

    /// 依美式鍵盤字元輸入（大千排列）
    fn type_str(&mut self, keys: &str) -> Vec<KeyResult> {
        keys.bytes()
            .map(|b| match b {
                b' ' => self.press(Key::Space, Mods::NONE),
                c => self.press(Key::Char(c), Mods::NONE),
            })
            .collect()
    }

    fn tap_shift(&mut self) -> KeyResult {
        let k = Key::Shift(Side::Left);
        self.t += 30;
        self.s.process_key(&KeyEvent::down(k, Mods::shift(), self.t));
        self.t += 80;
        self.s.process_key(&KeyEvent::up(k, Mods::NONE, self.t))
    }
}

#[test]
fn types_sentence_and_commits_on_enter() {
    let mut h = Harness::new();
    // ㄊㄧㄢ ㄑㄧˋ
    let rs = h.type_str("wu0 fu4");
    assert!(rs.iter().all(|r| r.eaten));
    assert_eq!(h.s.preedit().text, "天氣");
    let r = h.press(Key::Enter, Mods::NONE);
    assert!(r.eaten);
    assert_eq!(r.commit.as_deref(), Some("天氣"));
    assert!(!h.s.is_composing());
}

#[test]
fn preedit_shows_partial_bopomofo() {
    let mut h = Harness::new();
    h.type_str("wu");
    assert_eq!(h.s.preedit().text, "ㄊㄧ");
}

#[test]
fn ctrl_comma_passes_through_when_idle() {
    // P3：VS Code 的 Ctrl+, 不能被吃掉
    let mut h = Harness::new();
    let r = h.press(Key::Char(b','), Mods::ctrl());
    assert!(!r.eaten);
    assert_eq!(r.commit, None);
}

#[test]
fn ctrl_combo_passes_through_while_composing() {
    let mut h = Harness::new();
    h.type_str("wu0");
    let before = h.s.preedit();
    let r = h.press(Key::Char(b'c'), Mods::ctrl());
    assert!(!r.eaten);
    assert_eq!(h.s.preedit(), before);
}

#[test]
fn alt_and_meta_combos_pass_through() {
    let mut h = Harness::new();
    let alt = Mods { alt: true, ..Mods::NONE };
    let meta = Mods { meta: true, ..Mods::NONE };
    assert!(!h.press(Key::Char(b'f'), alt).eaten);
    assert!(!h.press(Key::Char(b'e'), meta).eaten);
}

#[test]
fn idle_navigation_keys_pass_through() {
    let mut h = Harness::new();
    for k in [Key::Space, Key::Enter, Key::Backspace, Key::Left, Key::Tab, Key::Esc] {
        assert!(!h.press(k, Mods::NONE).eaten, "{k:?}");
    }
}

#[test]
fn shift_tap_toggles_mode_and_english_passes_through() {
    let mut h = Harness::new();
    let r = h.tap_shift();
    assert_eq!(r.mode_changed, Some(InputMode::English));
    assert!(!h.press(Key::Char(b'a'), Mods::NONE).eaten);
    assert_eq!(h.tap_shift().mode_changed, Some(InputMode::Chinese));
    assert!(h.press(Key::Char(b'a'), Mods::NONE).eaten);
}

#[test]
fn shift_tap_commits_composition() {
    let mut h = Harness::new();
    h.type_str("wu0 fu4");
    let r = h.tap_shift();
    assert_eq!(r.commit.as_deref(), Some("天氣"));
    assert!(!h.s.is_composing());
}

#[test]
fn shift_letter_does_not_toggle() {
    let mut h = Harness::new();
    let shift = Key::Shift(Side::Left);
    h.s.process_key(&KeyEvent::down(shift, Mods::shift(), 10));
    h.s.process_key(&KeyEvent::down(Key::Char(b'a'), Mods::shift(), 20));
    h.s.process_key(&KeyEvent::up(Key::Char(b'a'), Mods::shift(), 30));
    let r = h.s.process_key(&KeyEvent::up(shift, Mods::NONE, 40));
    assert_eq!(r.mode_changed, None);
    assert_eq!(h.s.mode(), InputMode::Chinese);
}

#[test]
fn default_mode_from_config() {
    let mut c = Config::default();
    c.input.default_mode = InputMode::English;
    let h = Harness::with_config(c);
    assert_eq!(h.s.mode(), InputMode::English);
}

#[test]
fn space_opens_candidates_and_digit_selects() {
    let mut h = Harness::new();
    h.type_str("wu0 fu4"); // 天氣
    h.press(Key::Left, Mods::NONE); // 游標移到「氣」前
    let r = h.press(Key::Space, Mods::NONE);
    let page = r.candidates.expect("候選窗");
    assert!(!page.items.is_empty());
    println!("candidates: {:?}", page.items);
    let pick = page.items[1].clone();
    let r = h.press(Key::Char(b'2'), Mods::NONE);
    assert!(r.candidates.is_none());
    assert!(h.s.preedit().text.contains(&pick), "{:?} {pick}", h.s.preedit());
}

#[test]
fn test_key_has_no_side_effects() {
    let mut h = Harness::new();
    h.type_str("wu");
    let before = h.s.preedit();
    for _ in 0..3 {
        assert!(h.s.test_key(&KeyEvent::down(Key::Char(b'0'), Mods::NONE, 0)));
    }
    assert_eq!(h.s.preedit(), before);
}

#[test]
fn process_key_is_fast() {
    let mut h = Harness::new();
    let keys = "wu0 fu4 ";
    let start = std::time::Instant::now();
    let n = 200;
    for _ in 0..n {
        h.type_str(keys);
        h.press(Key::Esc, Mods::NONE);
        h.s.reset();
    }
    let per_key = start.elapsed() / (n * (keys.len() as u32 + 1) * 2);
    println!("每次事件平均 {per_key:?}");
}

#[test]
fn passthrough_keys_seen_by_test_key_down_cancel_shift_tap() {
    // 英文模式下按住 Shift 打字母：字母被放行（TSF 不會呼叫 OnKeyDown），放開 Shift 不能切換
    let mut c = Config::default();
    c.input.default_mode = InputMode::English;
    let mut h = Harness::with_config(c);
    let shift = Key::Shift(Side::Left);
    assert!(!h.s.test_key_down(&KeyEvent::down(shift, Mods::shift(), 10)));
    assert!(!h.s.test_key_down(&KeyEvent::down(Key::Char(b'a'), Mods::shift(), 20)));
    let r = h.s.process_key(&KeyEvent::up(shift, Mods::NONE, 40));
    assert_eq!(r.mode_changed, None);
}

fn open_candidates_for_qi(h: &mut Harness) -> zct_core::CandidatePage {
    h.type_str("wu0 fu4"); // 天氣
    h.press(Key::Left, Mods::NONE); // 游標移到「氣」前
    h.press(Key::Space, Mods::NONE).candidates.expect("候選窗")
}

#[test]
fn arrow_keys_move_highlight_and_enter_selects_it() {
    let mut h = Harness::new();
    let page = open_candidates_for_qi(&mut h);
    assert_eq!(page.highlight, 0);
    let r = h.press(Key::Down, Mods::NONE);
    assert_eq!(r.candidates.as_ref().unwrap().highlight, 1);
    let r = h.press(Key::Down, Mods::NONE);
    assert_eq!(r.candidates.as_ref().unwrap().highlight, 2);
    let r = h.press(Key::Up, Mods::NONE);
    assert_eq!(r.candidates.as_ref().unwrap().highlight, 1);
    let r = h.press(Key::Enter, Mods::NONE);
    assert!(r.candidates.is_none());
    assert_eq!(r.commit, None, "Enter 在候選窗中只選字，不送出");
    assert_eq!(h.s.preedit().text, format!("天{}", page.items[1]));
}

#[test]
fn down_past_last_item_turns_page() {
    let mut h = Harness::new();
    let page = open_candidates_for_qi(&mut h);
    assert!(page.total_pages > 1);
    let mut r = None;
    for _ in 0..page.items.len() {
        r = Some(h.press(Key::Down, Mods::NONE));
    }
    let p = r.unwrap().candidates.unwrap();
    assert_eq!((p.page, p.highlight), (1, 0));
    let p = h.press(Key::Up, Mods::NONE).candidates.unwrap();
    assert_eq!((p.page, p.highlight), (0, page.items.len() - 1));
}

#[test]
fn cursor_moves_after_selected_phrase() {
    // 今天|天氣很好 選完「天氣」→ 游標停在「很」前
    let mut h = Harness::new();
    h.type_str("wu0 fu4"); // 天氣
    h.press(Key::Home, Mods::NONE);
    assert_eq!(h.s.preedit().cursor, 0);
    h.press(Key::Space, Mods::NONE).candidates.expect("候選窗");
    h.press(Key::Char(b'1'), Mods::NONE);
    assert_eq!(h.s.preedit().cursor, 2);
}

fn half(h: &mut Harness) {
    let r = h.press(Key::Space, Mods::shift());
    assert!(r.eaten);
    assert_eq!(r.width_changed, Some(zct_core::Width::Half));
}

#[test]
fn shift_space_toggles_width() {
    let mut h = Harness::new();
    half(&mut h);
    let r = h.press(Key::Space, Mods::shift());
    assert_eq!(r.width_changed, Some(zct_core::Width::Full));
}

#[test]
fn half_width_outputs_ascii_symbols() {
    let mut h = Harness::new();
    half(&mut h);
    let cases: &[(u8, bool, &str)] = &[
        (b';', true, ":"),
        (b',', true, "<"),
        (b'.', true, ">"),
        (b'/', true, "?"),
        (b'\'', true, "\""),
        (b'[', true, "{"),
        (b']', true, "}"),
        (b'[', false, "["),
        (b']', false, "]"),
        (b'\'', false, "'"),
        (b'1', true, "!"),
    ];
    for &(c, shift, want) in cases {
        let mods = if shift { Mods::shift() } else { Mods::NONE };
        let r = h.press(Key::Char(c), mods);
        assert!(r.eaten);
        assert_eq!(r.commit.as_deref(), Some(want), "{} shift={shift}", c as char);
    }
}

#[test]
fn half_width_keeps_bopomofo_keys() {
    // , . / ; - 不按 Shift 時仍是注音（ㄝ ㄡ ㄥ ㄤ ㄦ）
    let mut h = Harness::new();
    half(&mut h);
    let r = h.press(Key::Char(b';'), Mods::NONE);
    assert_eq!(r.commit, None);
    assert_eq!(h.s.preedit().text, "ㄤ");
}

#[test]
fn half_width_symbol_commits_composition_first() {
    let mut h = Harness::new();
    half(&mut h);
    h.type_str("wu0 fu4");
    let r = h.press(Key::Char(b'/'), Mods::shift());
    assert_eq!(r.commit.as_deref(), Some("天氣?"));
    assert!(!h.s.is_composing());
}

#[test]
fn full_width_is_default() {
    let mut h = Harness::new();
    let r = h.press(Key::Char(b';'), Mods::shift());
    assert_eq!(r.commit, None);
    assert_eq!(h.s.preedit().text, "：");
}

#[test]
fn fullwidth_is_default_and_apostrophe_is_dunhao() {
    let mut h = Harness::new();
    h.press(Key::Char(b'\''), Mods::NONE);
    h.press(Key::Char(b','), Mods::shift());
    assert_eq!(h.s.preedit().text, "、，");
}

#[test]
fn shift_space_passes_through_in_english_mode() {
    let mut h = Harness::new();
    h.tap_shift();
    assert!(!h.press(Key::Space, Mods::shift()).eaten);
}

// ---- M3 ----

use zct_core::{AppRule, GameMode, Output};

fn rule(f: impl FnOnce(&mut AppRule)) -> Option<AppRule> {
    let mut r = AppRule::default();
    f(&mut r);
    Some(r)
}

#[test]
fn app_rule_sets_default_mode_and_width() {
    let h = Harness::with_rule(
        Config::default(),
        rule(|r| {
            r.default_mode = Some(InputMode::English);
            r.punctuation = Some(zct_core::Width::Half);
        }),
    );
    assert_eq!(h.s.mode(), InputMode::English);
    assert_eq!(h.s.width(), zct_core::Width::Half);
}

#[test]
fn game_mode_english_passes_everything_and_ignores_shift() {
    let mut h = Harness::with_rule(Config::default(), rule(|r| r.game_mode = Some(GameMode::English)));
    assert!(!h.press(Key::Char(b'w'), Mods::NONE).eaten);
    assert!(!h.press(Key::Space, Mods::shift()).eaten);
    assert_eq!(h.tap_shift().mode_changed, None);
}

#[test]
fn game_mode_keep_allows_chinese_but_shift_does_not_toggle() {
    let mut h = Harness::with_rule(Config::default(), rule(|r| r.game_mode = Some(GameMode::Keep)));
    assert_eq!(h.tap_shift().mode_changed, None);
    assert!(h.press(Key::Char(b'w'), Mods::NONE).eaten);
}

#[test]
fn game_hotkey_requests_toggle_in_any_mode() {
    let mut h = Harness::new();
    let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::NONE };
    let r = h.press(Key::F(12), ctrl_shift);
    assert!(r.eaten && r.toggle_game);
    // 進入全域遊戲模式後，快捷鍵仍然有效（才能關掉）
    h.s.set_global_game(true);
    assert!(!h.press(Key::Char(b'w'), Mods::NONE).eaten);
    assert!(h.press(Key::F(12), ctrl_shift).toggle_game);
}

#[test]
fn simplified_output_converts_commit_preedit_and_candidates() {
    let mut c = Config::default();
    c.input.output = Output::Simplified;
    let mut h = Harness::with_config(c);
    h.type_str("wu0 fu4");
    assert_eq!(h.s.preedit().text, "天气");
    let page = {
        h.press(Key::Left, Mods::NONE);
        h.press(Key::Space, Mods::NONE).candidates.unwrap()
    };
    assert!(page.items.contains(&"气".to_string()), "{:?}", page.items);
    h.press(Key::Esc, Mods::NONE);
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("天气"));
}

#[test]
fn output_hotkey_toggles_when_bound() {
    let mut c = Config::default();
    c.hotkeys.output = zct_core::hotkey::Binding::key("ctrl+alt+s");
    let mut h = Harness::with_config(c);
    let r = h.press(Key::Char(b's'), Mods { ctrl: true, alt: true, ..Mods::NONE });
    assert_eq!(r.output_changed, Some(Output::Simplified));
}

#[test]
fn custom_punctuation_overrides_default() {
    let mut c = Config::default();
    c.punctuation.full.insert("shift+,".into(), "《".into());
    let mut h = Harness::with_config(c);
    let r = h.press(Key::Char(b','), Mods::shift());
    assert!(r.eaten);
    assert_eq!(r.commit.as_deref(), Some("《"));
    // 沒有覆寫的鍵維持原本行為
    h.press(Key::Char(b'.'), Mods::shift());
    assert_eq!(h.s.preedit().text, "。");
}

#[test]
fn numpad_digits_pass_through_when_idle_and_commit_when_composing() {
    let mut h = Harness::with_config(no_auto_space());
    assert!(!h.press(Key::Numpad(b'5'), Mods::NONE).eaten);
    h.type_str("wu0 "); // 天
    let r = h.press(Key::Numpad(b'5'), Mods::NONE);
    assert_eq!(r.commit.as_deref(), Some("天5"));
}

#[test]
fn numpad_digit_selects_candidate() {
    let mut h = Harness::new();
    let page = open_candidates_for_qi(&mut h);
    h.press(Key::Numpad(b'3'), Mods::NONE);
    assert_eq!(h.s.preedit().text, format!("天{}", page.items[2]));
}

#[test]
fn caps_lock_letters_are_english() {
    let mut h = Harness::with_config(no_auto_space());
    let caps = Mods { caps_lock: true, ..Mods::NONE };
    assert!(!h.press(Key::Char(b'a'), caps).eaten, "沒有組字時交給 App");
    h.type_str("wu0 "); // 天
    let r = h.press(Key::Char(b'a'), caps);
    assert_eq!(r.commit.as_deref(), Some("天A"));
    // Caps Lock 下注音鍵中的數字與標點不受影響
    assert!(h.press(Key::Char(b','), caps).eaten);
}

#[test]
fn on_focus_reset_restores_default_mode() {
    let mut h = Harness::with_rule(Config::default(), rule(|r| r.on_focus = Some(zct_core::config::OnFocus::Reset)));
    h.tap_shift();
    assert_eq!(h.s.mode(), InputMode::English);
    h.s.focus();
    assert_eq!(h.s.mode(), InputMode::Chinese);
}

#[test]
fn on_focus_remember_keeps_mode() {
    let mut h = Harness::new();
    h.tap_shift();
    h.s.focus();
    assert_eq!(h.s.mode(), InputMode::English);
}

#[test]
fn function_keys_pass_through_while_composing() {
    let mut h = Harness::new();
    h.type_str("wu0");
    assert!(!h.press(Key::F(5), Mods::NONE).eaten);
}

// ---- M4 ----

fn candidates_for_qi(h: &mut Harness) -> Vec<String> {
    h.press(Key::Esc, Mods::NONE);
    h.press(Key::Esc, Mods::NONE);
    h.s.reset();
    open_candidates_for_qi(h).items
}

fn highlight_and(h: &mut Harness, target: &str, key: Key, mods: Mods) -> KeyResult {
    let items = candidates_for_qi(h);
    let i = items.iter().position(|w| w == target).expect("候選中要有目標字");
    for _ in 0..i {
        h.press(Key::Down, Mods::NONE);
    }
    h.press(key, mods)
}

#[test]
fn frequency_sort_puts_common_char_first() {
    // P2：「天ㄑㄧˋ」的單字候選，常用的「氣」應排在罕用的「企」之前
    let mut h = Harness::new();
    let items = candidates_for_qi(&mut h);
    let qi = items.iter().position(|w| w == "氣").unwrap();
    let qi2 = items.iter().position(|w| w == "企").unwrap();
    assert!(qi < qi2, "{items:?}");
}

#[test]
fn pin_moves_word_to_top() {
    let mut h = Harness::new();
    let r = highlight_and(&mut h, "汽", Key::Enter, Mods::shift());
    assert_eq!(r.notice.as_deref(), Some("已固頂：汽"));
    assert!(h.s.preedit().text.ends_with('汽'));
    assert_eq!(candidates_for_qi(&mut h)[0], "汽");
}

#[test]
fn forget_undoes_pin() {
    let mut h = Harness::new();
    let before = candidates_for_qi(&mut h);
    highlight_and(&mut h, "汽", Key::Enter, Mods::shift());
    let r = highlight_and(&mut h, "汽", Key::Delete, Mods::shift());
    assert_eq!(r.notice.as_deref(), Some("已忘記：汽"));
    assert_eq!(candidates_for_qi(&mut h)[0], before[0]);
}

#[test]
fn sessions_share_learning() {
    // 不同 App（session）共用同一份學習紀錄，不會互相覆蓋
    let dir = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(open_engine(&dir.path().join("learning")));
    let mut a = Harness::with_engine(engine.clone(), Config::default(), None, tempfile::tempdir().unwrap());
    let mut b = Harness::with_engine(engine, Config::default(), None, dir);
    highlight_and(&mut a, "汽", Key::Enter, Mods::shift());
    assert_eq!(candidates_for_qi(&mut b)[0], "汽");
}

#[test]
fn learning_survives_save_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    let learning = dir.path().join("learning");
    let engine = std::sync::Arc::new(open_engine(&learning));
    let mut h = Harness::with_engine(engine.clone(), Config::default(), None, tempfile::tempdir().unwrap());
    highlight_and(&mut h, "汽", Key::Enter, Mods::shift());
    assert!(engine.save().unwrap(), "有變更就要寫檔");
    assert!(!engine.save().unwrap(), "沒有變更就不寫");
    drop(h);
    let reloaded = std::sync::Arc::new(open_engine(&learning));
    let mut h2 = Harness::with_engine(reloaded, Config::default(), None, dir);
    assert_eq!(candidates_for_qi(&mut h2)[0], "汽");
}

#[test]
fn commit_does_not_write_files_synchronously() {
    let dir = tempfile::tempdir().unwrap();
    let learning = dir.path().join("learning");
    let engine = std::sync::Arc::new(open_engine(&learning));
    let mut h = Harness::with_engine(engine, Config::default(), None, tempfile::tempdir().unwrap());
    h.type_str("wu0 fu4");
    h.press(Key::Enter, Mods::NONE);
    assert!(!learning.join("history_dict.bin").exists());
}

#[test]
fn stats_count_chars_and_corrections() {
    let dir = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(open_engine(&dir.path().join("learning")));
    let mut h = Harness::with_engine(engine.clone(), Config::default(), None, dir);
    open_candidates_for_qi(&mut h);
    h.press(Key::Down, Mods::NONE);
    h.press(Key::Down, Mods::NONE);
    h.press(Key::Enter, Mods::NONE); // 選第 3 個 → 修正一次
    h.press(Key::Enter, Mods::NONE); // 送出「天器」
    let s = engine.stats();
    assert_eq!((s[0].chars, s[0].corrections), (2, 1));
}

#[test]
fn backup_keeps_latest_copies() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open_engine(&dir.path().join("learning"));
    engine.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
    engine.save().unwrap();
    let backups = dir.path().join("backup");
    for old in ["2026-01-01", "2026-01-02", "2026-01-03"] {
        std::fs::create_dir_all(backups.join(old)).unwrap();
    }
    engine.backup(&backups, 2).unwrap();
    let mut left: Vec<_> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left.len(), 2);
    assert_eq!(left[1], zct_core::engine::today());
}

#[test]
fn migrate_copies_old_learning_once() {
    let dir = tempfile::tempdir().unwrap();
    let (old, new) = (dir.path().join("old"), dir.path().join("new"));
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("user_dict.csv"), "old").unwrap();
    assert!(zct_core::Engine::migrate_from(&old, &new).unwrap());
    std::fs::write(old.join("user_dict.csv"), "newer").unwrap();
    assert!(!zct_core::Engine::migrate_from(&old, &new).unwrap(), "已有資料就不覆蓋");
    assert_eq!(std::fs::read_to_string(new.join("user_dict.csv")).unwrap(), "old");
}

#[test]
fn shift_arrows_mark_range_and_enter_adds_phrase() {
    let mut h = Harness::new();
    h.type_str("wu0 fu4"); // 天氣，游標在最後
    h.press(Key::Left, Mods::shift());
    h.press(Key::Left, Mods::shift());
    let r = h.press(Key::Enter, Mods::NONE);
    let notice = r.notice.expect("要有加詞提示");
    assert!(notice.contains("天氣"), "{notice}");
    assert!(h.s.is_composing(), "加詞後仍在組字，不會送出");
}

// ---- M5：詞庫管理 ----

#[test]
fn add_list_ban_unban_phrase() {
    use zct_core::PhraseAction;
    let dir = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(open_engine(&dir.path().join("learning")));
    engine.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
    assert!(engine.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").is_err(), "重複加入要報錯");
    assert!(engine.add_phrase("酷", "ㄎㄨˋ ㄧㄣ").is_err(), "字數與音節數不符");
    assert!(engine.add_phrase("酷音", "abc").is_err(), "注音格式錯誤");
    let list = engine.phrases();
    let e = list.iter().find(|p| p.word == "酷音").unwrap();
    assert_eq!(e.bopomofo, "ㄎㄨˋ ㄧㄣ");
    assert!(e.is_pinned(), "手動加入的詞優先");

    engine.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", PhraseAction::Ban).unwrap();
    let e = engine.phrases().into_iter().find(|p| p.word == "酷音").unwrap();
    assert_eq!(e.boost, zct_core::PhraseEntry::BANNED);
    engine.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", PhraseAction::Unban).unwrap();
    let e = engine.phrases().into_iter().find(|p| p.word == "酷音").unwrap();
    assert_eq!(e.boost, zct_core::PhraseEntry::NORMAL);
    assert!(!e.is_pinned() && !e.is_banned());
}

#[test]
fn added_phrase_shows_up_when_typing() {
    let dir = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(open_engine(&dir.path().join("learning")));
    // ㄓㄜˋ ㄙㄨㄟˋ：自創詞「這穗」
    engine.add_phrase("這穗", "ㄓㄜˋ ㄙㄨㄟˋ").unwrap();
    let mut h = Harness::with_engine(engine, Config::default(), None, dir);
    h.type_str("5k4njo4"); // ㄓㄜˋ ㄙㄨㄟˋ
    assert_eq!(h.s.preedit().text, "這穗");
}

#[test]
fn restore_copies_backup_and_freezes_saving() {
    let dir = tempfile::tempdir().unwrap();
    let learning = dir.path().join("learning");
    let backups = dir.path().join("backup");
    let engine = open_engine(&learning);
    engine.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
    engine.save().unwrap();
    engine.backup(&backups, 7).unwrap();
    let day = zct_core::Engine::backups(&backups).remove(0);

    engine.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", zct_core::PhraseAction::Ban).unwrap();
    engine.restore(&backups, &day).unwrap();
    assert!(!engine.save().unwrap(), "還原後不能再存檔，否則會蓋掉還原的檔案");
    assert!(engine.restore(&backups, "../x").is_err());

    let reloaded = open_engine(&learning);
    let e = reloaded.phrases().into_iter().find(|p| p.word == "酷音").unwrap();
    assert!(!e.is_banned(), "還原成備份當時（未封鎖）的狀態");
}

#[test]
fn no_backup_without_data() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open_engine(&dir.path().join("learning"));
    let backups = dir.path().join("backup");
    engine.backup(&backups, 7).unwrap();
    assert!(zct_core::Engine::backups(&backups).is_empty(), "沒有資料時不建立空的備份");
    engine.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
    engine.save().unwrap();
    engine.backup(&backups, 7).unwrap();
    assert_eq!(zct_core::Engine::backups(&backups).len(), 1);
}

// ---- M7：中英混打 ----

fn no_auto_space() -> Config {
    let mut c = Config::default();
    c.mixed.auto_space = false;
    c
}

fn english_commit(h: &mut Harness, keys: &str) -> Option<String> {
    let mut last = None;
    for b in keys.bytes() {
        let r = match b {
            b' ' => h.press(Key::Space, Mods::NONE),
            c => h.press(Key::Char(c), Mods::NONE),
        };
        if r.commit.is_some() {
            last = r.commit;
        }
    }
    last
}

#[test]
fn c_invalid_bopomofo_english_word_becomes_english() {
    let mut h = Harness::with_config(no_auto_space());
    assert_eq!(english_commit(&mut h, "hello ").as_deref(), Some("hello "));
    assert!(!h.s.is_composing());
}

#[test]
fn c_mixed_sentence_with_pangu_spacing() {
    // ㄨㄛˇ（我）+ hello + 空白
    let mut h = Harness::new();
    let out = english_commit(&mut h, "ji3hello ");
    assert_eq!(out.as_deref(), Some("我 hello "));
}

#[test]
fn c_does_not_trigger_on_valid_syllable_or_non_word() {
    let mut h = Harness::new();
    // fu = ㄑㄧ 合法音節 → 照常組成中文（七）
    h.type_str("fu ");
    assert!(h.s.is_composing());
    h.press(Key::Esc, Mods::NONE);
    h.s.reset();
    // hellx 不是英文單字 → 不送出英文
    assert_eq!(english_commit(&mut h, "hellx "), None);
}

#[test]
fn c_can_be_disabled() {
    let mut c = no_auto_space();
    c.mixed.auto_english = false;
    let mut h = Harness::with_config(c);
    assert_eq!(english_commit(&mut h, "hello "), None);
}

#[test]
fn b_english_option_in_candidates() {
    let mut h = Harness::with_config(no_auto_space());
    h.type_str("fu "); // ㄑㄧ 一聲（七）
    let page = h.press(Key::Space, Mods::NONE).candidates.expect("候選窗");
    assert_eq!(page.items.last().map(String::as_str), Some("fu"));
    let n = page.items.len();
    // 用數字鍵選英文選項
    let digit = b'0' + (n % 10) as u8;
    let r = h.press(Key::Char(digit), Mods::NONE);
    assert_eq!(r.commit.as_deref(), Some("fu"));
    assert!(!h.s.is_composing());
}

#[test]
fn b_english_option_keeps_previous_chinese() {
    let mut h = Harness::new();
    h.type_str("ji3fu "); // 我 + ㄑㄧ（七）
    let page = h.press(Key::Space, Mods::NONE).candidates.expect("候選窗");
    let n = page.items.len();
    for _ in 0..n - 1 {
        h.press(Key::Down, Mods::NONE);
    }
    let r = h.press(Key::Enter, Mods::NONE);
    assert_eq!(r.commit.as_deref(), Some("我 fu"));
}

#[test]
fn b_failed_syllable_does_not_leave_english_option() {
    // go = ㄕㄟ 沒有一聲，不會組成字；之後開「我」的候選窗不能出現 go，選了會把「我」刪掉
    let mut h = Harness::new();
    h.type_str("ji3go ");
    let page = h.press(Key::Space, Mods::NONE).candidates.expect("候選窗");
    assert!(!page.items.contains(&"go".to_string()), "{:?}", page.items);
}

#[test]
fn b_no_english_option_when_not_a_word() {
    let mut h = Harness::new();
    h.type_str("su3"); // ㄋㄧˇ（你）— 有聲調鍵，不是英文
    let page = h.press(Key::Space, Mods::NONE).candidates.expect("候選窗");
    assert!(page.items.iter().all(|w| w != "su3"));
}

#[test]
fn e_space_before_english_in_english_mode() {
    let mut h = Harness::new();
    h.type_str("ji3");
    h.press(Key::Enter, Mods::NONE); // 送出「我」
    h.tap_shift(); // 切到英文
    let r = h.press(Key::Char(b'o'), Mods::NONE);
    assert!(r.eaten);
    assert_eq!(r.commit.as_deref(), Some(" o"));
    // 之後的英文照常交給 App
    assert!(!h.press(Key::Char(b'k'), Mods::NONE).eaten);
}

#[test]
fn e_space_before_chinese_after_english_passthrough() {
    let mut h = Harness::new();
    h.tap_shift(); // 英文
    h.press(Key::Char(b'o'), Mods::NONE);
    h.press(Key::Char(b'k'), Mods::NONE);
    h.tap_shift(); // 中文
    h.type_str("ji3");
    let r = h.press(Key::Enter, Mods::NONE);
    assert_eq!(r.commit.as_deref(), Some(" 我"));
}

#[test]
fn e_cursor_movement_forgets_context() {
    let mut h = Harness::new();
    h.tap_shift();
    h.press(Key::Char(b'o'), Mods::NONE);
    h.press(Key::Left, Mods::NONE); // 游標移動後前文未知
    h.tap_shift();
    h.type_str("ji3");
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("我"));
}

// ---- M7：Emoji ----

fn ctrl_alt() -> Mods {
    Mods { ctrl: true, alt: true, ..Mods::NONE }
}

#[test]
fn emoji_from_composed_keyword() {
    let mut h = Harness::new();
    h.type_str("d9 vup "); // ㄎㄞ ㄒㄧㄣ → 開心
    assert_eq!(h.s.preedit().text, "開心");
    let r = h.press(Key::Char(b'e'), ctrl_alt());
    let page = r.candidates.expect("Emoji 候選");
    assert!(!page.items.is_empty());
    let first = page.items[0].clone();
    let r = h.press(Key::Char(b'1'), Mods::NONE);
    assert_eq!(r.commit.as_deref(), Some(first.as_str()), "Emoji 取代關鍵字");
    assert!(!h.s.is_composing());
}

#[test]
fn emoji_without_composition_lists_common_and_esc_closes() {
    let mut h = Harness::new();
    let page = h.press(Key::Char(b'e'), ctrl_alt()).candidates.expect("常用 Emoji");
    assert_eq!(page.items[0], zct_core::emoji::COMMON[0]);
    assert!(page.total_pages > 1);
    let r = h.press(Key::Right, Mods::NONE);
    assert_eq!(r.candidates.unwrap().page, 1);
    let r = h.press(Key::Esc, Mods::NONE);
    assert!(r.candidates.is_none());
    assert!(!h.s.is_composing());
}

#[test]
fn emoji_not_found_shows_notice() {
    let mut h = Harness::new();
    h.type_str("sk7"); // ㄋㄜ˙ → 呢（不在任何 Emoji 關鍵字中）
    let r = h.press(Key::Char(b'e'), ctrl_alt());
    assert!(r.candidates.is_none());
    assert!(r.notice.unwrap().contains("找不到"));
    assert!(h.s.is_composing(), "找不到時保留原本的組字");
}

#[test]
fn emoji_hotkey_passes_through_in_english_mode() {
    let mut h = Harness::new();
    h.tap_shift();
    assert!(!h.press(Key::Char(b'e'), ctrl_alt()).eaten);
}
