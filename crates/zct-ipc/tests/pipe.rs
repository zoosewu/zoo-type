#![cfg(windows)]

use std::time::{Duration, Instant};

use zct_ipc::pipe::{Client, Listener, pipe_name_with};
use zct_ipc::{Error, Request, Response};

fn hello() -> Request {
    Request::Hello {
        version: zct_ipc::PROTOCOL_VERSION,
        pid: 1,
        exe: "test.exe".into(),
    }
}

#[test]
fn request_response_roundtrip() {
    let name = pipe_name_with(&format!("-test-{}", std::process::id())).unwrap();
    let mut listener = Listener::with_name(name.clone()).unwrap();
    let server = std::thread::spawn(move || {
        let mut conn = listener.accept().unwrap();
        while let Some(req) = conn.recv().unwrap() {
            let resp = match req {
                Request::Hello { .. } => Response::Ok,
                Request::Reset => Response::Eat(true),
                _ => Response::Error("unexpected".into()),
            };
            conn.send(&resp).unwrap();
        }
    });
    // 等伺服器建立管道
    let mut client = loop {
        if let Ok(c) = Client::connect_to(&name) {
            break c;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        client.call(&hello(), Duration::from_secs(1)).unwrap(),
        Response::Ok
    );

    let start = Instant::now();
    let n = 1000;
    for _ in 0..n {
        assert_eq!(
            client
                .call(&Request::Reset, Duration::from_secs(1))
                .unwrap(),
            Response::Eat(true)
        );
    }
    println!("往返平均 {:?}", start.elapsed() / n);
    drop(client);
    server.join().unwrap();
}

#[test]
fn slow_server_times_out() {
    let name = pipe_name_with(&format!("-slow-{}", std::process::id())).unwrap();
    let mut listener = Listener::with_name(name.clone()).unwrap();
    let server = std::thread::spawn(move || {
        let mut conn = listener.accept().unwrap();
        let _ = conn.recv();
        std::thread::sleep(Duration::from_millis(300));
    });
    let mut client = loop {
        if let Ok(c) = Client::connect_to(&name) {
            break c;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let start = Instant::now();
    let r = client.call(&hello(), Duration::from_millis(50));
    assert!(matches!(r, Err(Error::Timeout)), "{r:?}");
    assert!(start.elapsed() < Duration::from_millis(200));
    server.join().unwrap();
}

#[test]
fn large_response_is_read_completely() {
    use zct_core::PhraseEntry;
    let name = pipe_name_with(&format!("-large-{}", std::process::id())).unwrap();
    let mut listener = Listener::with_name(name.clone()).unwrap();
    let big: Vec<PhraseEntry> = (0..20_000)
        .map(|i| PhraseEntry {
            word: format!("詞{i}"),
            bopomofo: "ㄘˊ ㄏㄠˋ".into(),
            boost: 10,
        })
        .collect();
    let expected = big.clone();
    let server = std::thread::spawn(move || {
        let mut conn = listener.accept().unwrap();
        while let Some(req) = conn.recv().unwrap() {
            let resp = match req {
                Request::ListPhrases => Response::Phrases(big.clone()),
                _ => Response::Ok,
            };
            conn.send(&resp).unwrap();
        }
    });
    let mut client = loop {
        if let Ok(c) = Client::connect_to(&name) {
            break c;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let size = zct_ipc::encode(&Response::Phrases(expected.clone()))
        .unwrap()
        .len();
    assert!(
        size > 4 * zct_ipc::INITIAL_BUFFER,
        "測試資料要大於緩衝區：{size}"
    );
    match client
        .call(&Request::ListPhrases, Duration::from_secs(5))
        .unwrap()
    {
        Response::Phrases(p) => assert_eq!(p, expected),
        other => panic!("{other:?}"),
    }
    // 大型回應之後，一般的小訊息照常運作
    assert_eq!(
        client
            .call(&Request::Reset, Duration::from_secs(1))
            .unwrap(),
        Response::Ok
    );
    drop(client);
    server.join().unwrap();
}
