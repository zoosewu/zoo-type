//! AI 同音字修正（需求 4.13）：呼叫 OpenAI 相容 API（Ollama、LM Studio、vLLM、雲端服務皆可）
//!
//! - 只在使用者按下快捷鍵時呼叫，結果當成一個候選，按 Enter 才採用
//! - 不改寫、不加字：回應字數與原文不同時視為無效
//! - 同步 HTTP（ureq）：服務程序在背景執行緒呼叫，不會卡住打字

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const SYSTEM_PROMPT: &str = "你是繁體中文注音輸入法的同音字校正器。使用者以注音輸入，下面的句子可能有同音或近音的錯字\
（例如 在／再、的／得／地、他／她、做／作、已／以）。\
只修正錯字，不要改寫、不要增減字、不要加標點、不要解釋。只輸出修正後的句子；沒有錯字就原樣輸出。";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// 例如 `http://localhost:11434/v1`（Ollama）、`https://api.openai.com/v1`
    pub base_url: String,
    pub model: String,
    /// 沒有金鑰的本機服務可以留空
    pub api_key: Option<String>,
    pub timeout: Duration,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: RespMessage,
}

#[derive(Deserialize)]
struct RespMessage {
    content: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AiError {
    /// 連線、逾時、HTTP 錯誤
    Request(String),
    /// 回應格式不對
    Response(String),
    /// 回應字數與原文不同（模型改寫了句子）
    Rewritten(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::Request(e) => write!(f, "無法連線到 AI：{e}"),
            AiError::Response(e) => write!(f, "AI 回應格式錯誤：{e}"),
            AiError::Rewritten(s) => write!(f, "AI 改寫了句子，已忽略：{s}"),
        }
    }
}

impl std::error::Error for AiError {}

/// 去掉推理模型的 `<think>…</think>`、引號與前後空白
pub fn clean(reply: &str) -> String {
    let mut s = reply.to_owned();
    while let (Some(a), Some(b)) = (s.find("<think>"), s.find("</think>")) {
        if b < a {
            break;
        }
        s.replace_range(a..b + "</think>".len(), "");
    }
    s.trim()
        .trim_matches(|c| matches!(c, '"' | '「' | '」' | '“' | '”'))
        .trim()
        .to_owned()
}

/// 檢查修正結果：字數必須與原文相同（只換字，不改寫）
pub fn validate(original: &str, reply: &str) -> Result<String, AiError> {
    let fixed = clean(reply);
    if fixed.chars().count() != original.chars().count() {
        return Err(AiError::Rewritten(fixed));
    }
    Ok(fixed)
}

fn chat(ep: &Endpoint, system: &str, user: &str) -> Result<String, AiError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(ep.timeout))
        .build()
        .into();
    let url = format!("{}/chat/completions", ep.base_url.trim_end_matches('/'));
    let body = ChatRequest {
        model: &ep.model,
        messages: vec![
            Message {
                role: "system",
                content: system,
            },
            Message {
                role: "user",
                content: user,
            },
        ],
        temperature: 0.0,
        stream: false,
    };
    let mut req = agent.post(&url);
    if let Some(key) = ep.api_key.as_deref().filter(|k| !k.is_empty()) {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }
    let resp: ChatResponse = req
        .send_json(&body)
        .map_err(|e| AiError::Request(e.to_string()))?
        .body_mut()
        .read_json()
        .map_err(|e| AiError::Response(e.to_string()))?;
    resp.choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| AiError::Response("沒有 choices".into()))
}

/// 修正同音錯字；回傳修正後的句子（可能與原文相同）
pub fn correct(ep: &Endpoint, text: &str) -> Result<String, AiError> {
    validate(text, &chat(ep, SYSTEM_PROMPT, text)?)
}

/// 設定程式的「測試」：能連上並取得回應即可
pub fn ping(ep: &Endpoint) -> Result<String, AiError> {
    chat(ep, "只回覆 OK。", "測試").map(|r| clean(&r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    #[test]
    fn clean_strips_think_and_quotes() {
        assert_eq!(clean("<think>嗯…</think>\n「可以再簡單一點」\n"), "可以再簡單一點");
        assert_eq!(clean("  今天天氣很好 "), "今天天氣很好");
    }

    #[test]
    fn validate_rejects_rewrites() {
        assert_eq!(validate("可以在簡單一點", "可以再簡單一點").unwrap(), "可以再簡單一點");
        assert!(matches!(
            validate("可以在簡單一點", "可以再簡單一點。"),
            Err(AiError::Rewritten(_))
        ));
    }

    /// 本機假伺服器：檢查請求內容並回傳固定的 OpenAI 格式回應
    fn fake_server(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            // ureq 傳 JSON 時可能用 chunked 編碼
            if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
                loop {
                    let mut size = String::new();
                    reader.read_line(&mut size).unwrap();
                    let n = usize::from_str_radix(size.trim(), 16).unwrap();
                    let mut chunk = vec![0; n + 2];
                    reader.read_exact(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..n]);
                }
            }
            let json = format!(
                r#"{{"choices":[{{"message":{{"role":"assistant","content":{}}}}}]}}"#,
                serde_json::to_string(reply).unwrap()
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                json.len()
            )
            .unwrap();
            format!("{head}{}", String::from_utf8(body).unwrap())
        });
        (format!("http://{addr}/v1"), handle)
    }

    #[test]
    fn correct_calls_openai_compatible_api() {
        let (url, server) = fake_server("可以再簡單一點");
        let ep = Endpoint {
            base_url: url,
            model: "test-model".into(),
            api_key: Some("sk-test".into()),
            timeout: Duration::from_secs(5),
        };
        assert_eq!(correct(&ep, "可以在簡單一點").unwrap(), "可以再簡單一點");
        let request = server.join().unwrap();
        let (head, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("POST /v1/chat/completions"));
        assert!(head.contains("Bearer sk-test"));
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["model"], "test-model");
        assert_eq!(json["temperature"], 0.0);
        assert_eq!(json["messages"][1]["content"], "可以在簡單一點");
    }

    #[test]
    fn unreachable_endpoint_is_a_request_error() {
        let ep = Endpoint {
            base_url: "http://127.0.0.1:9/v1".into(),
            model: "m".into(),
            api_key: None,
            timeout: Duration::from_secs(2),
        };
        assert!(matches!(correct(&ep, "測試"), Err(AiError::Request(_))));
    }
}
