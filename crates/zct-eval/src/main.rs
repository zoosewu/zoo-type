//! 選字準確率評測
//!
//! ```sh
//! sh scripts/fetch-dict.sh && sh scripts/fetch-dict.sh
//! cargo run -p zct-eval --release                      # 預設設定
//! cargo run -p zct-eval --release -- --no-sort         # 關閉依頻率排序比較
//! cargo run -p zct-eval --release -- --show 20         # 列出錯誤的片段
//! cargo run -p zct-eval --release -- --ai http://localhost:1234/v1 qwen3.5-9b   # AI 修正評測
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use zct_core::Engine;
use zct_core::config::Config;
use zct_eval::{Readings, evaluate, evaluate_ai};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut config = Config::default();
    let mut show = 10usize;
    let mut corpus_path = root.join("tests/eval/corpus.txt");
    let mut ai: Option<(String, String)> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-sort" => config.candidates.sort_by_frequency = false,
            "--show" => show = args.next().and_then(|n| n.parse().ok()).unwrap_or(show),
            "--ai" => ai = args.next().zip(args.next()),
            path => corpus_path = PathBuf::from(path),
        }
    }

    let readings = Readings::load(&root.join("assets/dict/tsi.csv"))
        .expect("需要先執行 scripts/fetch-dict.sh");
    let corpus = std::fs::read_to_string(&corpus_path).expect("讀不到語料");
    let learning = std::env::temp_dir().join(format!("zct-eval-{}", std::process::id()));
    let engine = Arc::new(
        Engine::open(&root.join("assets/dict"), &learning)
            .expect("需要先執行 scripts/fetch-dict.sh"),
    );

    if let Some((base_url, model)) = ai {
        let ep = zct_ai::Endpoint {
            base_url,
            model,
            api_key: std::env::var("ZCT_AI_KEY").ok(),
            timeout: std::time::Duration::from_secs(30),
        };
        let r = evaluate_ai(engine, config, &readings, &corpus, |text| {
            zct_ai::correct(&ep, text).map_err(|e| e.to_string())
        });
        let pct = |n: usize| n as f64 * 100.0 / r.chars.max(1) as f64;
        println!(
            "AI 修正：{:.2}% → {:.2}%（{} 字）　修好 {} 段、改壞 {} 段、失敗 {} 次　平均 {} ms",
            pct(r.correct_before),
            pct(r.correct_after),
            r.chars,
            r.fixed,
            r.broken,
            r.failed,
            r.total_ms / r.runs.max(1) as u128
        );
        for (want, got, after) in r.changes.iter().take(show) {
            println!("  原文 {want}\n  首選 {got}\n  AI   {after}");
        }
        let _ = std::fs::remove_dir_all(&learning);
        return;
    }

    let r = evaluate(engine, config, &readings, &corpus);
    println!(
        "首選正確率：{:.2}%（{}/{} 字）　整段完全正確：{}/{}　略過：{}",
        r.accuracy() * 100.0,
        r.correct,
        r.chars,
        r.exact_runs,
        r.runs,
        r.skipped
    );
    for (want, got) in r.mismatches.iter().take(show) {
        println!("  {want}\n→ {got}");
    }
    let _ = std::fs::remove_dir_all(&learning);
}
