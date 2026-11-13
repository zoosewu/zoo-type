//! 開發用：測試服務程序的管理用請求
//! cargo run -p zct-ipc --example admin

use std::time::Duration;

use zct_ipc::pipe::Client;
use zct_ipc::{PROTOCOL_VERSION, Request, Response};

fn main() {
    let mut c = Client::connect().expect("服務程序沒有在執行");
    let mut call = |r: Request| c.call(&r, Duration::from_secs(2)).expect("呼叫失敗");
    call(Request::Hello {
        version: PROTOCOL_VERSION,
        pid: std::process::id(),
        exe: "admin.exe".into(),
    });
    println!(
        "加詞：{:?}",
        call(Request::AddPhrase {
            word: "酷音".into(),
            bopomofo: "ㄎㄨˋ ㄧㄣ".into()
        })
    );
    println!(
        "重複加詞：{:?}",
        call(Request::AddPhrase {
            word: "酷音".into(),
            bopomofo: "ㄎㄨˋ ㄧㄣ".into()
        })
    );
    if let Response::Phrases(p) = call(Request::ListPhrases) {
        println!(
            "詞庫 {} 筆：{:?}",
            p.len(),
            p.iter()
                .map(|e| (&e.word, &e.bopomofo, e.boost))
                .collect::<Vec<_>>()
        );
    }
    println!("統計：{:?}", call(Request::Stats));
    let backups = match call(Request::Backups) {
        Response::Backups(b) => b,
        other => panic!("{other:?}"),
    };
    println!("備份：{backups:?}");
    if std::env::args().any(|a| a == "--restore") {
        println!(
            "還原：{:?}",
            call(Request::Restore {
                day: backups[0].clone()
            })
        );
    }
}
