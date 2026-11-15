//! LLM 補強語料：請本機模型依「生活主題 × 易混淆的字詞」寫台灣日常口語句子
//!
//! 維基是百科文體，口語、祈使句（「拖鞋換好再進來」「房租要繳」）很少；這些正是同音字最常選錯的地方。
//! 輸出一行一句；檔案已有的「主題 × 字詞」組合會略過（中斷後可以接著跑）。

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;
use zct_core::mixed::is_cjk;

const TOPICS: &[&str] = &[
    "上班與開會",
    "學校與作業",
    "租屋與繳費",
    "網購與退貨",
    "餐廳與點餐",
    "搭捷運高鐵與交通",
    "看醫生與吃藥",
    "天氣與出門",
    "家人之間的對話",
    "朋友約吃飯",
    "寫程式與部署",
    "手機與電腦問題",
    "旅行與訂房",
    "運動與健身",
    "養寵物",
    "煮飯與做家事",
    "銀行與信用卡",
    "社群貼文與留言",
    "打電動與直播",
    "買房與裝潢",
    "節日與祝福",
    "求職與面試",
    "看電影與追劇",
    "通勤與塞車",
];

/// 注音輸入最常混淆的字詞（每一組都要出現在句子裡，模型才學得到正確的上下文）
const WORDS: &[&str] = &[
    "再", "在", "交", "教", "訂", "定", "繳", "祝", "著", "哪裡", "那裡", "得", "地", "做", "作",
    "已經", "以後", "需要", "必須", "向", "像", "藥", "花", "帳單", "畫", "劃", "她", "他", "裡",
    "部署",
];

const SYSTEM: &str = "你是台灣人，用台灣繁體中文寫日常口語句子。用詞要是台灣的說法（例如：軟體、網路、捷運、影片），\
不要用中國大陸用語。每句獨立、自然、口語，像是聊天訊息或生活中會說的話。只輸出句子，一行一句，不要編號、不要解釋。";

const PER_REQUEST: usize = 15;

fn key(topic: &str, word: &str) -> String {
    format!("#{topic}|{word}")
}

/// 去掉編號、項目符號、引號；太短或含英數字太多的不要
fn tidy(line: &str) -> Option<String> {
    let s = line
        .trim()
        .trim_start_matches(|c: char| {
            c.is_ascii_digit() || matches!(c, '.' | '、' | ')' | '）' | '-' | '*' | '•' | ' ')
        })
        .trim_matches(|c| matches!(c, '"' | '「' | '」' | '“' | '”'))
        .trim();
    let cjk = s.chars().filter(|&c| is_cjk(c)).count();
    (cjk >= 4 && cjk * 2 >= s.chars().count()).then(|| s.to_owned())
}

pub fn run(out: &Path, endpoint: &str, model: &str) -> std::io::Result<()> {
    let done: HashSet<String> = std::fs::read_to_string(out)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with('#'))
        .map(str::to_owned)
        .collect();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)?;
    let cc = OpenCC::from_config(BuiltinConfig::S2twp)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let ep = zct_ai::Endpoint {
        base_url: endpoint.to_owned(),
        model: model.to_owned(),
        api_key: None,
        timeout: Duration::from_secs(120),
    };
    let total = TOPICS.len() * WORDS.len();
    let mut n = done.len();
    let mut sentences = 0usize;
    for topic in TOPICS {
        for word in WORDS {
            let k = key(topic, word);
            if done.contains(&k) {
                continue;
            }
            let user = format!(
                "主題：{topic}。請寫 {PER_REQUEST} 句不同的句子，每句都要自然地用到「{word}」，用法正確、情境多樣。"
            );
            let reply = match zct_ai::generate(&ep, SYSTEM, &user, 0.9) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("  {topic} × {word} 失敗：{e}");
                    continue;
                }
            };
            let text = cc.convert(&zct_ai::clean(&reply));
            let lines: Vec<String> = text.lines().filter_map(tidy).collect();
            writeln!(file, "{k}")?;
            for l in &lines {
                writeln!(file, "{l}")?;
            }
            file.flush()?;
            n += 1;
            sentences += lines.len();
            eprintln!("  [{n}/{total}] {topic} × {word}：{} 句", lines.len());
        }
    }
    eprintln!("完成：這次新增 {sentences} 句 → {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_strips_numbering_and_rejects_noise() {
        assert_eq!(tidy("1. 「明天再說吧」").as_deref(), Some("明天再說吧"));
        assert_eq!(tidy("- 房租這週要繳").as_deref(), Some("房租這週要繳"));
        assert_eq!(tidy("OK"), None);
        assert_eq!(tidy("abc def ghi 好"), None);
    }
}
