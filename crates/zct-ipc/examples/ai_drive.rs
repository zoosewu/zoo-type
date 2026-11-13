//! 開發用：打「可以在」→ Ctrl+Alt+R → 等 AI → Enter，印出送出的文字
use std::time::Duration;

use zct_core::{Key, KeyEvent, Mods};
use zct_ipc::pipe::Client;
use zct_ipc::{PROTOCOL_VERSION, Rect, Request, Response};

fn main() {
    let mut c = Client::connect().expect("服務程序沒有在執行");
    let t = Duration::from_secs(2);
    c.call(
        &Request::Hello {
            version: PROTOCOL_VERSION,
            pid: std::process::id(),
            exe: "ai.exe".into(),
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
    let mut key = |c: &mut Client, k: Key, m: Mods| -> Response {
        now += 40;
        let r = c.call(&Request::Key(KeyEvent::down(k, m, now)), t).unwrap();
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
        r
    };
    for b in b"dk3u3y94" {
        key(&mut c, Key::Char(*b), Mods::NONE);
    }
    if let Response::Key(r) = key(
        &mut c,
        Key::Char(b'r'),
        Mods {
            ctrl: true,
            alt: true,
            ..Mods::NONE
        },
    ) {
        println!("請求：{:?} 提示：{:?}", r.ai_request, r.notice);
    }
    std::thread::sleep(Duration::from_millis(800));
    // 下一次按鍵前服務程序會把 AI 結果交給 session
    if let Response::Eat(eat) = c
        .call(
            &Request::TestKeyDown(KeyEvent::down(Key::Enter, Mods::NONE, 9000)),
            t,
        )
        .unwrap()
    {
        println!("Enter 會被吃掉：{eat}");
    }
    if let Response::Key(r) = key(&mut c, Key::Enter, Mods::NONE) {
        println!("送出：{:?}", r.commit);
    }
}
