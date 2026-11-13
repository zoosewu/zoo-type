//! 開發用：打「開心」後開啟 Emoji 候選窗，停 3 秒供截圖
use std::time::Duration;

use zct_core::{Key, KeyEvent, Mods};
use zct_ipc::pipe::Client;
use zct_ipc::{PROTOCOL_VERSION, Rect, Request};

fn main() {
    let mut c = Client::connect().expect("服務程序沒有在執行");
    let t = Duration::from_millis(500);
    c.call(
        &Request::Hello {
            version: PROTOCOL_VERSION,
            pid: std::process::id(),
            exe: "emoji.exe".into(),
        },
        t,
    )
    .unwrap();
    let caret = Some(Rect {
        left: 400,
        top: 300,
        right: 402,
        bottom: 324,
    });
    let mut now = 0;
    let mut key = |c: &mut Client, k: Key, m: Mods| {
        now += 40;
        let r = c.call(&Request::Key(KeyEvent::down(k, m, now)), t).unwrap();
        println!("{k:?} → {r:?}");
        now += 40;
        c.call(&Request::Key(KeyEvent::up(k, m, now)), t).unwrap();
        c.call(
            &Request::Caret {
                rect: caret,
                fresh: true,
            },
            t,
        )
        .unwrap();
    };
    for b in b"d9 vup " {
        let k = if *b == b' ' {
            Key::Space
        } else {
            Key::Char(*b)
        };
        key(&mut c, k, Mods::NONE);
    }
    key(
        &mut c,
        Key::Char(b'e'),
        Mods {
            ctrl: true,
            alt: true,
            ..Mods::NONE
        },
    );
    std::thread::sleep(Duration::from_secs(3));
}
