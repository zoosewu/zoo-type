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
fn shift_tap_keeps_composition_and_types_latin_into_it() {
    // 第 Shift 6 Shift 章：組字中切英文不送出，數字與英文放進組字區
    let mut h = Harness::new();
    h.type_str("2u4"); // 第
    let first = h.s.preedit().text;
    let r = h.tap_shift();
    assert_eq!(r.mode_changed, Some(InputMode::English));
    assert!(r.commit.is_none());
    assert!(h.press(Key::Char(b'6'), Mods::NONE).eaten);
    assert_eq!(h.s.preedit().text, format!("{first}6"));
    h.tap_shift();
    h.type_str("5; "); // ㄓㄤ
    let text = h.press(Key::Enter, Mods::NONE).commit.unwrap();
    assert!(text.starts_with(&format!("{first}6")) && text.chars().count() == 3, "{text}");
}

#[test]
fn shift_tap_when_empty_still_toggles_without_composition() {
    let mut h = Harness::new();
    assert_eq!(h.tap_shift().mode_changed, Some(InputMode::English));
    assert!(!h.press(Key::Char(b'a'), Mods::NONE).eaten, "沒在組字時英文直接交給 App");
}

#[test]
fn english_mode_in_composition_uppercase_space_and_enter() {
    let mut h = Harness::new();
    h.type_str("ji3"); // 我
    h.tap_shift();
    h.press(Key::Space, Mods::NONE);
    h.press(Key::Char(b'o'), Mods::shift());
    h.press(Key::Char(b'k'), Mods::NONE);
    assert_eq!(h.s.preedit().text, "我 Ok");
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("我 Ok"));
}

#[test]
fn english_only_at_start_or_after_space() {
    // 我love：中文後面沒有空白 → 不判斷英文
    let mut h = Harness::new();
    h.type_str("ji3love ");
    assert!(!h.s.preedit().text.contains("love"), "{}", h.s.preedit().text);
    // 開頭
    let mut h = Harness::new();
    h.type_str("hello ");
    assert_eq!(h.s.preedit().text, "hello ");
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
    let r = h.press(Key::Down, Mods::NONE);
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
    h.press(Key::Down, Mods::NONE).candidates.expect("候選窗")
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
    h.press(Key::Down, Mods::NONE).candidates.expect("候選窗");
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
fn half_width_symbol_joins_composition() {
    let mut h = Harness::new();
    half(&mut h);
    h.type_str("wu0 fu4");
    let r = h.press(Key::Char(b'/'), Mods::shift());
    assert_eq!(r.commit, None);
    assert_eq!(h.s.preedit().text, "天氣?");
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("天氣?"));
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
fn simplified_output_shows_traditional_and_converts_on_commit() {
    // 簡體模式：組字區與候選窗一律顯示繁體，送出時整句轉成簡體
    let mut c = Config::default();
    c.input.output = Output::Simplified;
    let mut h = Harness::with_config(c);
    h.type_str("wu0 fu4");
    assert_eq!(h.s.preedit().text, "天氣");
    let page = {
        h.press(Key::Left, Mods::NONE);
        h.press(Key::Down, Mods::NONE).candidates.unwrap()
    };
    assert!(page.items.contains(&"氣".to_string()), "{:?}", page.items);
    assert!(!page.items.contains(&"气".to_string()), "簡體字不出現：{:?}", page.items);
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
fn numpad_digits_pass_through_when_idle_and_join_composition() {
    let mut h = Harness::with_config(no_auto_space());
    assert!(!h.press(Key::Numpad(b'5'), Mods::NONE).eaten);
    h.type_str("wu0 "); // 天（空白 = 一聲）
    let r = h.press(Key::Numpad(b'5'), Mods::NONE);
    assert!(r.eaten && r.commit.is_none());
    assert_eq!(h.s.preedit().text, "天5");
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
fn c_english_word_stays_in_composition_until_enter() {
    // 句首的英文也要留在組字區
    let mut h = Harness::with_config(no_auto_space());
    assert_eq!(english_commit(&mut h, "hello "), None);
    assert_eq!(h.s.preedit().text, "hello ");
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("hello "));
}

#[test]
fn c_mixed_sentence_in_composition() {
    // 我 + 空白 + love + 空白 + 你（ㄋㄧˇ）
    let mut h = Harness::new();
    assert_eq!(english_commit(&mut h, "ji3 love su3"), None);
    assert_eq!(h.s.preedit().text, "我 love 你");
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("我 love 你"));
}

#[test]
fn c_garbage_keys_never_become_english() {
    // 不在單字表的亂打按鍵：空白或 Enter 都不會變英文
    let mut h = Harness::new();
    h.type_str("detault ");
    assert!(!h.s.preedit().text.contains("detault"));
    let mut h = Harness::new();
    h.type_str("hello");
    let r = h.press(Key::Enter, Mods::NONE);
    assert_ne!(r.commit.as_deref(), Some("hello"), "英文只在空白鍵判斷");
}

#[test]
fn invalid_syllable_stays_with_bell() {
    // ㄍㄧˇ 組不成字：保留注音並提示，不要消失
    let mut h = Harness::new();
    h.type_str("eu");
    let r = h.press(Key::Char(b'3'), Mods::NONE);
    assert!(r.bell);
    assert_eq!(h.s.preedit().text, "ㄍㄧ");
    // Backspace 清掉整個注音後照常打字
    h.press(Key::Backspace, Mods::NONE);
    assert_eq!(h.s.preedit().text, "");
    let r = h.press(Key::Char(b'j'), Mods::NONE); // ㄨ
    assert!(!r.bell);
    h.type_str("i3"); // ㄛˇ → 我
    assert_eq!(h.s.preedit().text, "我");
}

#[test]
fn invalid_first_tone_with_space_stays_with_bell() {
    let mut h = Harness::new();
    h.type_str("eu");
    let r = h.press(Key::Space, Mods::NONE);
    assert!(r.bell);
    assert_eq!(h.s.preedit().text, "ㄍㄧ");
}

#[test]
fn backspace_cancels_half_typed_syllable() {
    // 相ㄑㄧㄣ + Backspace → 相
    let mut h = Harness::new();
    h.type_str("vu; ");
    let first = h.s.preedit().text;
    h.type_str("fup");
    assert_eq!(h.s.preedit().text, format!("{first}ㄑㄧㄣ"));
    let r = h.press(Key::Backspace, Mods::NONE);
    assert!(r.eaten);
    assert_eq!(h.s.preedit().text, first);
    // 沒有注音時 Backspace 照常刪前一個字
    h.press(Key::Backspace, Mods::NONE);
    assert_eq!(h.s.preedit().text, "");
}

#[test]
fn numpad_digit_goes_into_composition() {
    // 第 6 章：數字鍵盤的 6 放進組字區，整句一起送出
    let mut h = Harness::new();
    h.type_str("2u4 "); // 第 + 空白
    let r = h.press(Key::Numpad(b'6'), Mods::NONE);
    assert!(r.commit.is_none());
    h.type_str(" 5; "); // 空白 + ㄓㄤ
    let r = h.press(Key::Enter, Mods::NONE);
    let text = r.commit.unwrap();
    assert!(text.chars().skip(1).take(3).eq(" 6 ".chars()) && text.chars().count() == 5, "{text}");
}

#[test]
fn space_after_complete_syllables_is_a_space_not_menu() {
    let mut h = Harness::new();
    h.type_str("ji3"); // 我
    let r = h.press(Key::Space, Mods::NONE);
    assert!(r.candidates.is_none(), "空白不開候選窗");
    assert_eq!(h.s.preedit().text, "我 ");
}

#[test]
fn left_skips_whole_english_word() {
    // 我 love 你：從最後往左 5 次到「我」前面
    let mut h = Harness::new();
    english_commit(&mut h, "ji3 love su3");
    for _ in 0..5 {
        h.press(Key::Left, Mods::NONE);
    }
    assert_eq!(h.s.preedit().cursor, 0);
}

#[test]
fn backspace_deletes_space_then_whole_word() {
    let mut h = Harness::new();
    english_commit(&mut h, "ji3 love ");
    assert_eq!(h.s.preedit().text, "我 love ");
    h.press(Key::Backspace, Mods::NONE);
    assert_eq!(h.s.preedit().text, "我 love");
    h.press(Key::Backspace, Mods::NONE);
    assert_eq!(h.s.preedit().text, "我 ");
}

#[test]
fn backspace_on_unfinished_word_deletes_one_letter() {
    let mut h = Harness::new();
    english_commit(&mut h, "ji3 love ");
    h.press(Key::Backspace, Mods::NONE); // 刪空白
    h.press(Key::Left, Mods::NONE); // 移動後就不是「剛刪完空白」
    h.press(Key::End, Mods::NONE);
    h.press(Key::Backspace, Mods::NONE);
    assert_eq!(h.s.preedit().text, "我 lov");
}

#[test]
fn delete_removes_whole_word_before_cursor() {
    let mut h = Harness::new();
    english_commit(&mut h, "ji3 love su3");
    h.press(Key::Home, Mods::NONE);
    h.press(Key::Right, Mods::NONE); // 我|
    h.press(Key::Right, Mods::NONE); // 空白|
    h.press(Key::Delete, Mods::NONE);
    assert_eq!(h.s.preedit().text, "我  你");
}

#[test]
fn preedit_target_marks_char_after_cursor_but_not_at_end() {
    let mut h = Harness::new();
    h.type_str("wu0 fu4"); // 天氣
    assert_eq!(h.s.preedit().target, None, "游標在最後不標示");
    assert_eq!(h.s.preedit().len, 2);
    h.press(Key::Left, Mods::NONE);
    assert_eq!(h.s.preedit().target, Some((1, 1)));
}

#[test]
fn tab_switches_to_extras_page_of_emoji_and_bopomofo() {
    let mut h = Harness::new();
    h.type_str("d9 vup "); // 開心
    h.press(Key::Home, Mods::NONE);
    let first = h.press(Key::Down, Mods::NONE).candidates.unwrap();
    // Tab → 額外選項頁；再按 Tab 回到候選字
    let extras = h.press(Key::Tab, Mods::NONE).candidates.unwrap();
    assert_eq!(extras.items.last().map(String::as_str), Some("ㄎㄞ ㄒㄧㄣ"));
    let back = h.press(Key::Tab, Mods::NONE).candidates.unwrap();
    assert_eq!(back.items, first.items);
    let extras = h.press(Key::Tab, Mods::NONE).candidates.unwrap();
    assert!(extras.items.len() > 1, "有 Emoji：{:?}", extras.items);
    // 選注音
    let n = extras.items.len();
    let digit = b'0' + (n % 10) as u8;
    assert_eq!(h.press(Key::Char(digit), Mods::NONE).commit.as_deref(), Some("ㄎㄞ ㄒㄧㄣ"));
}

#[test]
fn pinned_candidates_are_marked() {
    let mut h = Harness::new();
    highlight_and(&mut h, "汽", Key::Enter, Mods::shift());
    let page = candidates_for_qi(&mut h);
    let _ = page;
    let p = {
        h.press(Key::Esc, Mods::NONE);
        h.s.reset();
        open_candidates_for_qi(&mut h)
    };
    assert_eq!(p.items[0], "汽");
    assert!(p.pinned[0]);
    assert!(!p.pinned[1]);
}

#[test]
fn pinning_twice_does_not_overflow() {
    let mut h = Harness::new();
    highlight_and(&mut h, "汽", Key::Enter, Mods::shift());
    highlight_and(&mut h, "汽", Key::Enter, Mods::shift());
    assert_eq!(candidates_for_qi(&mut h)[0], "汽");
}

#[test]
fn c_does_not_trigger_on_valid_syllable_or_non_word() {
    let mut h = Harness::new();
    // fu = ㄑㄧ 合法音節 → 照常組成中文（七）
    h.type_str("fu ");
    assert_eq!(h.s.preedit().text, "七");
    h.s.reset();
    // xx：兩個字母、不在單字表 → 不當英文
    h.type_str("xx ");
    assert!(!h.s.preedit().text.contains("xx"));
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
    let page = h.press(Key::Down, Mods::NONE).candidates.expect("候選窗");
    assert_eq!(page.items.last().map(String::as_str), Some("fu"));
    let n = page.items.len();
    // 用數字鍵選英文選項：最後一個音節換成英文，仍在組字區
    let digit = b'0' + (n % 10) as u8;
    h.press(Key::Char(digit), Mods::NONE);
    assert_eq!(h.s.preedit().text, "fu");
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("fu"));
}

#[test]
fn b_english_option_keeps_previous_chinese() {
    let mut h = Harness::new();
    h.type_str("ji3 fu "); // 我 + 空白 + ㄑㄧ（七）
    let page = h.press(Key::Down, Mods::NONE).candidates.expect("候選窗");
    let n = page.items.len();
    for _ in 0..n - 1 {
        h.press(Key::Down, Mods::NONE);
    }
    h.press(Key::Enter, Mods::NONE);
    assert_eq!(h.s.preedit().text, "我 fu");
}

#[test]
fn b_failed_syllable_does_not_leave_english_option() {
    // go = ㄕㄟ 沒有一聲，不會組成字；之後開「我」的候選窗不能出現 go，選了會把「我」刪掉
    let mut h = Harness::new();
    h.type_str("ji3go");
    assert!(h.press(Key::Space, Mods::NONE).bell, "組不成字要提示");
    assert_eq!(h.s.preedit().text, "我ㄕㄟ");
    if let Some(page) = h.press(Key::Down, Mods::NONE).candidates {
        assert!(!page.items.contains(&"go".to_string()), "{:?}", page.items);
    }
}

#[test]
fn b_no_english_option_when_not_a_word() {
    let mut h = Harness::new();
    h.type_str("su3"); // ㄋㄧˇ（你）— 有聲調鍵，不是英文
    let page = h.press(Key::Down, Mods::NONE).candidates.expect("候選窗");
    assert!(page.items.iter().all(|w| w != "su3"));
}

fn pangu() -> Config {
    let mut c = Config::default();
    c.mixed.auto_space = true;
    c
}

#[test]
fn e_space_before_english_in_english_mode() {
    let mut h = Harness::with_config(pangu());
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
    let mut h = Harness::with_config(pangu());
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

fn ctrl_shift() -> Mods {
    Mods { ctrl: true, shift: true, ..Mods::NONE }
}

#[test]
fn emoji_from_composed_keyword() {
    let mut h = Harness::new();
    h.type_str("d9 vup "); // ㄎㄞ ㄒㄧㄣ → 開心
    assert_eq!(h.s.preedit().text, "開心");
    let r = h.press(Key::Char(b';'), ctrl_shift());
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
    let page = h.press(Key::Char(b';'), ctrl_shift()).candidates.expect("常用 Emoji");
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
    let r = h.press(Key::Char(b';'), ctrl_shift());
    assert!(r.candidates.is_none());
    assert!(r.notice.unwrap().contains("找不到"));
    assert!(h.s.is_composing(), "找不到時保留原本的組字");
}

#[test]
fn emoji_hotkey_passes_through_in_english_mode() {
    let mut h = Harness::new();
    h.tap_shift();
    assert!(!h.press(Key::Char(b';'), ctrl_shift()).eaten);
}

// ---- M7：同步 ----

#[test]
fn sync_export_keeps_timestamps_of_unchanged_entries() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open_engine(&dir.path().join("learning"));
    engine.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
    let first = zct_core::sync::export(&engine, "", 100);
    assert_eq!(first, "酷音\tㄎㄨˋ ㄧㄣ\tpinned\t100\n");
    // 沒變動 → 沿用舊時間
    assert_eq!(zct_core::sync::export(&engine, &first, 200), first);
    // 變動 → 新時間
    engine
        .phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", zct_core::PhraseAction::Ban)
        .unwrap();
    assert_eq!(
        zct_core::sync::export(&engine, &first, 300),
        "酷音\tㄎㄨˋ ㄧㄣ\tbanned\t300\n"
    );
}

#[test]
fn sync_between_two_machines_newest_wins() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = open_engine(da.path());
    let b = open_engine(db.path());
    // A 加詞並同步
    a.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
    let file = zct_core::sync::export(&a, "", 100);
    assert_eq!(zct_core::sync::import(&b, &file), 1);
    assert!(b.phrases().iter().any(|p| p.word == "酷音" && p.is_pinned()));
    // B 之後封鎖，A 的舊紀錄與 B 的新紀錄被 union 合併
    b.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", zct_core::PhraseAction::Ban).unwrap();
    let from_b = zct_core::sync::export(&b, &file, 200);
    let merged = format!("{file}{from_b}");
    assert_eq!(zct_core::sync::import(&a, &merged), 1);
    assert!(a.phrases().iter().any(|p| p.word == "酷音" && p.is_banned()));
    // 再匯入一次沒有變更
    assert_eq!(zct_core::sync::import(&a, &merged), 0);
}

// ---- M8：AI 修正 ----

fn ai_on() -> Config {
    let mut c = no_auto_space();
    c.ai.enabled = true;
    c
}

fn type_zai(h: &mut Harness) -> String {
    // ㄎㄜˇ ㄧˇ ㄗㄞˋ（可以在）
    h.type_str("dk3u3y94");
    h.s.preedit().text
}

#[test]
fn ai_request_then_accept_suggestion() {
    let mut h = Harness::with_config(ai_on());
    let original = type_zai(&mut h);
    let r = h.press(Key::Char(b'\''), ctrl_shift());
    assert_eq!(r.ai_request.as_deref(), Some(original.as_str()));
    assert_eq!(r.notice.as_deref(), Some("AI 修正中…"));
    assert!(h.s.set_ai_suggestion(&original, "可以再".into()));
    assert_eq!(h.s.candidates().unwrap().items, vec!["可以再"]);
    let r = h.press(Key::Enter, Mods::NONE);
    assert_eq!(r.commit.as_deref(), Some("可以再"));
    assert!(!h.s.is_composing());
}

#[test]
fn ai_suggestion_can_be_dismissed() {
    let mut h = Harness::with_config(ai_on());
    let original = type_zai(&mut h);
    h.press(Key::Char(b'\''), ctrl_shift());
    h.s.set_ai_suggestion(&original, "可以再".into());
    let r = h.press(Key::Esc, Mods::NONE);
    assert!(r.candidates.is_none());
    assert_eq!(h.s.preedit().text, original, "放棄建議後保留原本的組字");
}

#[test]
fn stale_ai_result_is_ignored() {
    let mut h = Harness::with_config(ai_on());
    let original = type_zai(&mut h);
    h.press(Key::Char(b'\''), ctrl_shift());
    h.type_str("1j4"); // 結果回來前繼續打字
    assert!(!h.s.set_ai_suggestion(&original, "可以再".into()));
    assert!(h.s.candidates().is_none());
}

#[test]
fn ai_same_as_original_is_not_shown() {
    let mut h = Harness::with_config(ai_on());
    let original = type_zai(&mut h);
    h.press(Key::Char(b'\''), ctrl_shift());
    assert!(!h.s.set_ai_suggestion(&original, original.clone()));
}

#[test]
fn ai_disabled_shows_notice() {
    let mut h = Harness::new();
    type_zai(&mut h);
    let r = h.press(Key::Char(b'\''), ctrl_shift());
    assert!(r.ai_request.is_none());
    assert!(r.notice.unwrap().contains("未啟用"));
}

#[test]
fn phrase_state_cycle_normal_pinned_banned() {
    use zct_core::PhraseAction;
    let dir = tempfile::tempdir().unwrap();
    let e = open_engine(dir.path());
    let state = |e: &zct_core::Engine| {
        let p = e.phrases().into_iter().find(|p| p.word == "酷音").unwrap();
        (p.is_pinned(), p.is_banned())
    };
    e.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
    assert_eq!(state(&e), (true, false), "加入即優先");
    e.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", PhraseAction::Unpin).unwrap();
    assert_eq!(state(&e), (false, false), "取消優先");
    e.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", PhraseAction::Ban).unwrap();
    assert_eq!(state(&e), (false, true));
    e.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", PhraseAction::Pin).unwrap();
    assert_eq!(state(&e), (true, false), "封鎖後可直接設回優先");
    e.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", PhraseAction::Pin).unwrap();
    assert_eq!(state(&e), (true, false), "重複設定優先不會溢位");
}

#[test]
fn left_right_expand_candidates_into_columns_and_keep_row() {
    // ㄕˋ 的候選很多：→ 換到下一欄並展開，選擇框停在同一行
    let mut h = Harness::new();
    h.type_str("g4"); // ㄕˋ
    let first = h.press(Key::Down, Mods::NONE).candidates.unwrap();
    assert!(first.columns.is_empty(), "還沒按 ← → 時不展開");
    assert!(first.total_pages > 2);
    h.press(Key::Down, Mods::NONE);
    h.press(Key::Down, Mods::NONE); // 第 3 行
    let page = h.press(Key::Right, Mods::NONE).candidates.unwrap();
    assert_eq!(page.page, 1);
    assert_eq!(page.highlight, 2, "同一行");
    assert!(page.columns.len() > 1);
    assert_eq!(page.first_column, 0);
    assert_eq!(page.columns[0], first.items);
    assert_eq!(page.columns[1], page.items);
    // Enter 選的是第二欄第 3 個
    let want = page.items[2].clone();
    h.press(Key::Enter, Mods::NONE);
    assert_eq!(h.s.preedit().text, want);
}

#[test]
fn expanded_state_resets_after_selection() {
    let mut h = Harness::new();
    h.type_str("g4");
    h.press(Key::Down, Mods::NONE);
    h.press(Key::Right, Mods::NONE);
    h.press(Key::Enter, Mods::NONE);
    let page = h.press(Key::Down, Mods::NONE).candidates.unwrap();
    assert!(page.columns.is_empty());
}

#[test]
fn enter_drops_stuck_syllable_and_commits_rest() {
    // 我ㄍㄟ（組不成字卡住）+ Enter → 送出「我」
    let mut h = Harness::new();
    h.type_str("ji3eo");
    assert!(h.press(Key::Space, Mods::NONE).bell);
    let r = h.press(Key::Enter, Mods::NONE);
    assert!(r.eaten);
    assert_eq!(r.commit.as_deref(), Some("我"));
    assert!(!h.s.is_composing());
}

#[test]
fn enter_with_only_stuck_syllable_clears_without_newline() {
    let mut h = Harness::new();
    h.type_str("eu3"); // ㄍㄧˇ 卡住
    let r = h.press(Key::Enter, Mods::NONE);
    assert!(r.eaten, "不要把 Enter 交給 App 換行");
    assert!(r.commit.is_none());
    assert!(!h.s.is_composing());
}

#[test]
fn delete_keeps_original_behavior_while_typing_syllable() {
    // Delete 不取消注音（維持 chewing 原本行為）
    let mut h = Harness::new();
    h.type_str("eo"); // ㄍㄟ
    h.press(Key::Delete, Mods::NONE);
    assert_eq!(h.s.preedit().text, "ㄍㄟ");
}

#[test]
fn tech_abbreviations_are_english() {
    // 一般字典沒有的縮寫：repo、config、api
    for w in ["repo", "config", "api"] {
        let mut h = Harness::new();
        h.type_str(&format!("{w} "));
        assert_eq!(h.s.preedit().text, format!("{w} "), "{w}");
    }
}

#[test]
fn user_english_words_file() {
    let dir = tempfile::tempdir().unwrap();
    let words = dir.path().join("dict").join("english.txt");
    let mut engine = open_engine(&dir.path().join("learning"));
    engine.load_user_words(&words);
    assert!(words.exists(), "不存在時建立範本");
    std::fs::write(&words, "# 註解\nZctx\n").unwrap();
    let mut engine = open_engine(&dir.path().join("learning"));
    engine.load_user_words(&words);
    let mut h = Harness::with_engine(std::sync::Arc::new(engine), Config::default(), None, dir);
    h.type_str("zctx ");
    assert_eq!(h.s.preedit().text, "zctx ");
}

/// 共用同一個 Engine 的第二個 Harness（模擬下一次打字）
fn again(h: &Harness) -> Harness {
    Harness::with_engine(h.s.engine(), Config::default(), None, tempfile::tempdir().unwrap())
}

#[test]
fn shift_typed_english_is_learned_after_commit() {
    let mut h = Harness::new();
    h.type_str("ji3"); // 我
    h.tap_shift();
    h.type_str(" zctq");
    h.tap_shift();
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("我 zctq"));
    // 下次直接打 zctq + 空白就是英文
    let mut h2 = again(&h);
    h2.type_str("zctq ");
    assert_eq!(h2.s.preedit().text, "zctq ");
}

#[test]
fn shift_english_deleted_before_commit_is_not_learned() {
    let mut h = Harness::new();
    h.type_str("ji3");
    h.tap_shift();
    h.type_str(" zctq");
    h.s.reset(); // 取消，沒有送出
    let mut h2 = again(&h);
    h2.type_str("zctq ");
    assert_ne!(h2.s.preedit().text, "zctq ");
}

#[test]
fn forget_hotkey_on_english_word_stops_auto_english() {
    let mut h = Harness::new();
    h.type_str("repo ");
    assert_eq!(h.s.preedit().text, "repo ");
    h.press(Key::Left, Mods::NONE); // 游標移到 repo 後面
    let r = h.press(Key::Delete, Mods::shift());
    assert!(r.notice.is_some_and(|n| n.contains("repo")));
    assert_eq!(h.s.preedit().text, "repo ", "文字不變，只是之後不再當英文");
    let mut h2 = again(&h);
    h2.type_str("repo ");
    assert_ne!(h2.s.preedit().text, "repo ");
}

#[test]
fn halfwidth_symbol_stays_in_composition() {
    // 半形模式下，組字中打的半形符號留在組字區，Enter 才一起送出
    let mut h = Harness::new();
    h.press(Key::Space, Mods::shift()); // 切半形
    h.type_str("ji3"); // 我
    let r = h.press(Key::Char(b'1'), Mods::shift()); // !
    assert!(r.commit.is_none());
    assert_eq!(h.s.preedit().text, "我!");
    assert_eq!(h.press(Key::Enter, Mods::NONE).commit.as_deref(), Some("我!"));
}

#[test]
fn width_is_remembered_across_sessions() {
    // 切到別的輸入法再切回來（新的 session）全半形維持
    let mut h = Harness::new();
    let r = h.press(Key::Space, Mods::shift());
    let w = r.width_changed.expect("切換全半形");
    let h2 = again(&h);
    assert_eq!(h2.s.width(), w);
}

/// 打 ㄔㄨㄥˊ ㄎㄞ ㄐㄧ，把第一個字改成「重」後送出
fn correct_chong(h: &mut Harness) -> Option<String> {
    h.type_str("tj/6d9 ru ");
    h.press(Key::Home, Mods::NONE);
    let mut page = h.press(Key::Down, Mods::NONE).candidates.expect("候選窗");
    for _ in 0..page.total_pages {
        if let Some(i) = page.items.iter().position(|w| w == "重") {
            h.press(Key::Char(b'0' + ((i + 1) % 10) as u8), Mods::NONE);
            return h.press(Key::Enter, Mods::NONE).commit;
        }
        page = h.press(Key::Right, Mods::NONE).candidates.expect("候選窗");
    }
    None
}

#[test]
fn correction_is_learned_on_second_time() {
    let mut h = Harness::new();
    h.type_str("tj/6d9 ru ");
    assert_eq!(h.s.preedit().text, "蟲開機", "詞庫預設（若詞庫更新後已正確，這個測試要換例子）");
    h.press(Key::Esc, Mods::NONE);
    h.s.reset();

    assert_eq!(correct_chong(&mut h).as_deref(), Some("重開機"));
    let mut h2 = again(&h);
    h2.type_str("tj/6d9 ru ");
    assert_eq!(h2.s.preedit().text, "蟲開機", "改一次還不學（保守）");
    h2.s.reset();

    assert_eq!(correct_chong(&mut h2).as_deref(), Some("重開機"));
    let mut h3 = again(&h2);
    h3.type_str("tj/6d9 ru ");
    assert_eq!(h3.s.preedit().text, "重開機", "改兩次學起來");
    // 學的是「重開機」這個詞，單獨的 ㄔㄨㄥˊ 不受影響
    h3.s.reset();
    h3.type_str("tj/6");
    assert_eq!(h3.s.preedit().text, "蟲");
}

#[test]
fn correction_undone_before_commit_is_not_counted() {
    let mut h = Harness::new();
    for _ in 0..2 {
        h.type_str("tj/6d9 ru ");
        h.press(Key::Home, Mods::NONE);
        let page = h.press(Key::Down, Mods::NONE).candidates.unwrap();
        let _ = page;
        h.press(Key::Right, Mods::NONE);
        h.press(Key::Char(b'2'), Mods::NONE); // 改成第二頁某個字
        h.s.reset(); // 沒送出
    }
    let mut h2 = again(&h);
    h2.type_str("tj/6d9 ru ");
    assert_eq!(h2.s.preedit().text, "蟲開機");
}

#[test]
fn single_char_candidates_rank_common_reading_and_hide_simplified() {
    // ㄔㄨㄥˊ：「重」（重新、重複…）排第一；簡體字（烛、蛊）不出現
    let mut h = Harness::new();
    h.type_str("tj/6");
    let page = h.press(Key::Down, Mods::NONE).candidates.unwrap();
    assert_eq!(page.items[0], "重", "{:?}", page.items);
    let mut all = page.items.clone();
    for _ in 1..page.total_pages {
        all.extend(h.press(Key::Right, Mods::NONE).candidates.unwrap().items);
    }
    for c in ["烛", "蛊"] {
        assert!(!all.contains(&c.to_string()), "{c}：{all:?}");
    }
    assert!(all.contains(&"种".to_string()), "繁體也用的字保留：{all:?}");
}

#[test]
fn phrase_and_single_candidates_are_one_list() {
    // 天氣：詞（天氣、天啟…）在前，單字（天、添…）接在後面，同一份清單
    let mut h = Harness::new();
    h.type_str("wu0 fu4");
    h.press(Key::Home, Mods::NONE);
    let first = h.press(Key::Down, Mods::NONE).candidates.unwrap();
    assert_eq!(first.items[0].chars().count(), 2, "{:?}", first.items);
    let mut all = first.items.clone();
    for _ in 1..first.total_pages {
        all.extend(h.press(Key::Right, Mods::NONE).candidates.unwrap().items);
    }
    let single = all.iter().position(|w| w.chars().count() == 1).expect("有單字");
    assert!(all[single..].iter().all(|w| w.chars().count() == 1), "單字在詞之後：{all:?}");
    // 選單字「添」：換算回 chewing 的單字那一組
    let i = all.iter().position(|w| w == "添").expect("添");
    h.press(Key::Esc, Mods::NONE);
    h.press(Key::Down, Mods::NONE);
    for _ in 0..i {
        h.press(Key::Down, Mods::NONE);
    }
    h.press(Key::Enter, Mods::NONE);
    assert!(h.s.preedit().text.starts_with('添'), "{}", h.s.preedit().text);
}

#[test]
fn simplified_only_chars_use_taiwan_usage() {
    let dir = tempfile::tempdir().unwrap();
    let e = open_engine(dir.path());
    assert_eq!(e.simplified_only('机'), Some('機'));
    assert_eq!(e.simplified_only('烛'), Some('燭'));
    // 台灣也常用的寫法保留
    for c in ['秘', '群', '后', '种', '機'] {
        assert_eq!(e.simplified_only(c), None, "{c}");
    }
    assert_eq!(e.to_traditional("我的手机"), "我的手機");
}

#[test]
fn composition_bar_hotkey_is_reported_in_any_mode() {
    let mut h = Harness::new();
    let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::NONE };
    let r = h.press(Key::F(11), ctrl_shift);
    assert!(r.eaten && r.toggle_composition_bar);
    h.tap_shift(); // 英文模式也能切換
    assert!(h.press(Key::F(11), ctrl_shift).toggle_composition_bar);
}

#[test]
fn auto_commit_of_long_composition_leaves_no_bopomofo() {
    // chewing 組字超過 39 字時會自動送出前面的字：長度不變，不能誤判成「注音組不成字」而把注音補回去
    let mut h = Harness::new();
    let mut committed = String::new();
    for _ in 0..45 {
        for r in h.type_str("ji3") {
            if let Some(c) = r.commit {
                committed.push_str(&c);
            }
        }
        let text = h.s.preedit().text;
        assert!(!text.contains('ㄨ') && !text.contains('ㄛ'), "注音殘留：{text}");
    }
    assert!(!committed.is_empty(), "超過長度要自動送出");
    let r = h.press(Key::Enter, Mods::NONE);
    committed.push_str(&r.commit.unwrap_or_default());
    assert_eq!(committed.chars().count(), 45, "{committed}");
    assert!(committed.chars().all(|c| c == '我'));
}
