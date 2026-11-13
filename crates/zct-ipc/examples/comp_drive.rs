//! 開發用：打「我 love 你」→ 左移兩格 → 停 2 秒（組字提示列）→ ↓ 開候選窗 → 停 2 秒
use std::time::Duration;

use zct_core::{Key, KeyEvent, Mods};
use zct_ipc::pipe::Client;
use zct_ipc::{PROTOCOL_VERSION, Rect, Request};

fn main() {
    let mut c = Client::connect().expect("服務程序沒有在執行");
    let t = Duration::from_secs(2);
    c.call(&Request::Hello { version: PROTOCOL_VERSION, pid: std::process::id(), exe: "comp.exe".into() }, t).unwrap();
    let caret = Some(Rect { left: 400, top: 300, right: 402, bottom: 324 });
    let mut now = 0;
    let mut key = |c: &mut Client, k: Key| {
        now += 40;
        c.call(&Request::Key(KeyEvent::down(k, Mods::NONE, now)), t).unwrap();
        now += 40;
        c.call(&Request::Key(KeyEvent::up(k, Mods::NONE, now)), t).unwrap();
        c.call(&Request::Caret { rect: caret, fresh: true }, t).unwrap();
    };
    for b in b"ji3 love su3" {
        key(&mut c, if *b == b' ' { Key::Space } else { Key::Char(*b) });
    }
    key(&mut c, Key::Left);
    key(&mut c, Key::Left);
    key(&mut c, Key::Left);
    println!("組字提示列（目標：love）");
    std::thread::sleep(Duration::from_secs(2));
    key(&mut c, Key::Home);
    key(&mut c, Key::Down);
    println!("候選窗（我）");
    std::thread::sleep(Duration::from_secs(2));
}
