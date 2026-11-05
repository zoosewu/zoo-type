//! 選字準確率回歸測試：準確率低於基準就失敗（需要 fetch-dict.sh 與 fetch-eval.sh 的資料，沒有時略過）

use std::path::PathBuf;
use std::sync::Arc;

use zct_core::Engine;
use zct_core::config::Config;
use zct_eval::{Readings, evaluate};

/// 2026-11-05 基準：99.35%（1380/1389）。留一點餘裕給詞庫更新造成的小幅波動。
const BASELINE: f64 = 0.99;

#[test]
fn first_choice_accuracy_does_not_regress() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (tsi, dict) = (root.join("assets/eval/tsi.csv"), root.join("assets/dict"));
    if !tsi.exists() || !dict.join("static_dict.bin").exists() {
        eprintln!("略過：沒有評測資料（執行 scripts/fetch-dict.sh 與 scripts/fetch-eval.sh）");
        return;
    }
    let readings = Readings::load(&tsi).unwrap();
    let corpus = std::fs::read_to_string(root.join("tests/eval/corpus.txt")).unwrap();
    let learning = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::open(&dict, learning.path()).unwrap());
    let r = evaluate(engine, Config::default(), &readings, &corpus);
    assert_eq!(r.skipped, 0, "語料中有查不到讀音的字");
    assert!(
        r.accuracy() >= BASELINE,
        "首選正確率 {:.2}% 低於基準 {:.0}%：{:?}",
        r.accuracy() * 100.0,
        BASELINE * 100.0,
        r.mismatches
    );
}
