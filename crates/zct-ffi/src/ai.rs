//! 測試用的假 OpenAI 相容 API
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
pub fn server(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        let mut length = 0;
        let mut chunked = false;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            chunked |= line.to_lowercase().contains("transfer-encoding: chunked");
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        if chunked {
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let size = usize::from_str_radix(line.trim(), 16).unwrap();
                let mut part = vec![0; size + 2];
                reader.read_exact(&mut part).unwrap();
                if size == 0 {
                    break;
                }
                body.extend_from_slice(&part[..size]);
            }
        }
        request.push_str(&String::from_utf8(body).unwrap());
        let json = serde_json::json!({"choices":[{"message":{"content":reply}}]}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}", json.len()).unwrap();
        request
    });
    (format!("http://{address}/v1"), worker)
}
