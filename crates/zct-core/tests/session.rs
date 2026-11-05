//! Session 整合測試（需要先執行 scripts/fetch-dict.sh 下載詞庫）

use zct_core::config::{Config, InputMode};
use zct_core::{Key, KeyEvent, KeyResult, Mods, Session, SessionOptions, Side};

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
        let dir = tempfile::tempdir().unwrap();
        let opts = SessionOptions {
            sys_dict_dir: concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/dict").into(),
            user_dir: dir.path().join("user"),
            config,
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
