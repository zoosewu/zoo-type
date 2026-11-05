//! 開發用：假的 OpenAI 相容伺服器，把收到的句子中的「在」換成「再」後回傳
//! cargo run -p zct-ai --example fake_ai -- 18080
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

fn main() {
    let port = std::env::args().nth(1).unwrap_or_else(|| "18080".into());
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let (mut len, mut chunked) = (0usize, false);
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let l = line.to_ascii_lowercase();
            if let Some(v) = l.strip_prefix("content-length:") { len = v.trim().parse().unwrap(); }
            if l.contains("transfer-encoding: chunked") { chunked = true; }
            if line == "\r\n" { break; }
        }
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        if chunked {
            loop {
                let mut size = String::new();
                reader.read_line(&mut size).unwrap();
                let n = usize::from_str_radix(size.trim(), 16).unwrap();
                let mut chunk = vec![0; n + 2];
                reader.read_exact(&mut chunk).unwrap();
                if n == 0 { break; }
                body.extend_from_slice(&chunk[..n]);
            }
        }
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let text = json["messages"][1]["content"].as_str().unwrap_or("").replace('在', "再");
        let reply = serde_json::json!({"choices":[{"message":{"role":"assistant","content":text}}]}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
    }
}
