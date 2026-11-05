//! 選字準確率評測
//!
//! ```sh
//! sh scripts/fetch-dict.sh && sh scripts/fetch-eval.sh
//! cargo run -p zct-eval --release                      # 預設設定
//! cargo run -p zct-eval --release -- --no-sort         # 關閉依頻率排序比較
//! cargo run -p zct-eval --release -- --show 20         # 列出錯誤的片段
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use zct_core::Engine;
use zct_core::config::Config;
use zct_eval::{Readings, evaluate};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut config = Config::default();
    let mut show = 10usize;
    let mut corpus_path = root.join("tests/eval/corpus.txt");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-sort" => config.candidates.sort_by_frequency = false,
            "--show" => show = args.next().and_then(|n| n.parse().ok()).unwrap_or(show),
            path => corpus_path = PathBuf::from(path),
        }
    }

    let readings = Readings::load(&root.join("assets/eval/tsi.csv"))
        .expect("需要先執行 scripts/fetch-eval.sh");
    let corpus = std::fs::read_to_string(&corpus_path).expect("讀不到語料");
    let learning = std::env::temp_dir().join(format!("zct-eval-{}", std::process::id()));
    let engine = Arc::new(
        Engine::open(&root.join("assets/dict"), &learning).expect("需要先執行 scripts/fetch-dict.sh"),
    );

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
