//! 開發用：模擬 TSF DLL 驅動服務程序，確認組字、候選窗與提示窗
//! cargo run -p zct-ipc --example drive

use std::time::Duration;

use zct_core::{Key, KeyEvent, Mods, Side};
use zct_ipc::pipe::Client;
use zct_ipc::{PROTOCOL_VERSION, Rect, Request, Response};

fn main() {
    let mut c = Client::connect().expect("服務程序沒有在執行");
    let t = Duration::from_millis(500);
    let call = |c: &mut Client, r: Request| c.call(&r, t).expect("呼叫失敗");
    println!("{:?}", call(&mut c, Request::Hello { version: PROTOCOL_VERSION, pid: std::process::id(), exe: "drive.exe".into() }));
    let caret = Some(Rect { left: 400, top: 300, right: 402, bottom: 324 });
    let mut now = 0u64;
    let mut key = |c: &mut Client, k: Key, mods: Mods| {
        now += 40;
        let r = call(c, Request::Key(KeyEvent::down(k, mods, now)));
        now += 40;
        let up_mods = if matches!(k, Key::Shift(_)) { Mods::NONE } else { mods };
        let r2 = call(c, Request::Key(KeyEvent::up(k, up_mods, now)));
        call(c, Request::Caret(caret));
        if let (Response::Key(a), Response::Key(b)) = (r, r2) {
            println!("{k:?} eaten={} preedit={:?} commit={:?} cand={:?} mode={:?}",
                a.eaten, b.preedit.text, a.commit.or(b.commit), b.candidates.map(|p| p.items), a.mode_changed.or(b.mode_changed));
        }
    };
    for b in b"wu0 fu4" {
        let k = if *b == b' ' { Key::Space } else { Key::Char(*b) };
        key(&mut c, k, Mods::NONE);
    }
    key(&mut c, Key::Left, Mods::NONE);
    key(&mut c, Key::Space, Mods::NONE);
    key(&mut c, Key::Down, Mods::NONE);
    key(&mut c, Key::Down, Mods::NONE);
    println!("候選窗應出現在 (400, 324) 附近，選擇框在第 3 個，3 秒後按 Enter 選它");
    std::thread::sleep(Duration::from_secs(3));
    key(&mut c, Key::Enter, Mods::NONE);
    key(&mut c, Key::Enter, Mods::NONE);
    key(&mut c, Key::Shift(Side::Left), Mods::shift());
    println!("提示窗「A」應短暫出現");
    std::thread::sleep(Duration::from_secs(1));
}
