//! 在終端機模擬按鍵序列（大千排列的美式鍵盤字元）
//!
//! cargo run -p zct-core --example sim -- "wu0 fu4" "<enter>"
//! 特殊鍵：<enter> <space> <esc> <bs> <left> <right> <up> <down> <shift> <ctrl+X>

use std::time::Instant;

use zct_core::config::Config;
use zct_core::{Key, KeyEvent, Mods, Session, SessionOptions, Side};

fn main() {
    let dict = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/dict");
    // ZCT_SIM_LEARNING：指定學習資料夾（例如使用者資料的複本），重現個人化後的選字
    let user = std::env::var_os("ZCT_SIM_LEARNING")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("zct-sim"));
    let t0 = Instant::now();
    let engine = zct_core::Engine::open(std::path::Path::new(dict), &user)
        .expect("需要先執行 scripts/fetch-dict.sh");
    let engine = std::sync::Arc::new(engine);
    let mut s = Session::new(&SessionOptions {
        engine: engine.clone(),
        config: Config::default(),
        rule: None,
    })
    .expect("需要先執行 scripts/fetch-dict.sh");
    println!("建立 session：{:?}", t0.elapsed());

    let mut t = 0u64;
    let mut send = |s: &mut Session, key: Key, mods: Mods, label: String| {
        t += 50;
        let start = Instant::now();
        let r = s.process_key(&KeyEvent::down(key, mods, t));
        let took = start.elapsed();
        t += 50;
        let up_mods = if matches!(key, Key::Shift(_)) { Mods::NONE } else { mods };
        let r_up = s.process_key(&KeyEvent::up(key, up_mods, t));
        let commit = r.commit.or(r_up.commit);
        let mode = r.mode_changed.or(r_up.mode_changed);
        println!(
            "{label:<10} 吃掉={:<5} 組字={:<12} 送出={:<8} 候選={:<40} 模式={:<8} ({took:?})",
            r.eaten,
            format!("{:?}", r_up.preedit.text),
            format!("{:?}", commit.unwrap_or_default()),
            format!("{:?}", r_up.candidates.map(|c| c.items).unwrap_or_default()),
            mode.map(|m| format!("{m:?}")).unwrap_or_default(),
        );
    };

    for arg in std::env::args().skip(1) {
        let mut rest = arg.as_str();
        while !rest.is_empty() {
            if let Some(end) = rest.strip_prefix('<').and_then(|r| r.find('>')) {
                let name = &rest[1..=end];
                rest = &rest[end + 2..];
                let (key, mods) = match name {
                    "enter" => (Key::Enter, Mods::NONE),
                    "space" => (Key::Space, Mods::NONE),
                    "esc" => (Key::Esc, Mods::NONE),
                    "bs" => (Key::Backspace, Mods::NONE),
                    "left" => (Key::Left, Mods::NONE),
                    "right" => (Key::Right, Mods::NONE),
                    "up" => (Key::Up, Mods::NONE),
                    "down" => (Key::Down, Mods::NONE),
                    "shift" => (Key::Shift(Side::Left), Mods::shift()),
                    n if n.starts_with("ctrl+") => (Key::Char(n.as_bytes()[5]), Mods::ctrl()),
                    n if n.starts_with("shift+") => (Key::Char(n.as_bytes()[6]), Mods::shift()),
                    n => panic!("未知按鍵 <{n}>"),
                };
                send(&mut s, key, mods, format!("<{name}>"));
            } else {
                let c = rest.as_bytes()[0];
                rest = &rest[1..];
                let key = if c == b' ' { Key::Space } else { Key::Char(c) };
                send(&mut s, key, Mods::NONE, format!("{:?}", c as char));
            }
        }
    }
    // 學習紀錄平常由服務程序定期存檔；這裡結束前存一次，下次執行才看得到
    if let Err(e) = engine.save() {
        eprintln!("存檔失敗：{e}");
    }
}
