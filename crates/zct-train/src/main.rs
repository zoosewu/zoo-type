//! T0 語言模型訓練工具（開發用）
//!
//! ```sh
//! # 維基 dump → 台灣正體、一行一句；每 50 篇留一篇做評測（不參與訓練）
//! cargo run -p zct-train --release -- extract .corpus/text .corpus/wiki/*.bz2
//! # 統計字 n-gram、修剪、寫出模型
//! cargo run -p zct-train --release -- train .corpus/text/train.txt .corpus/ngram.bin --order 4 --min 1,2,3,3
//! # 用本機 LLM 生成口語句子補強（LM Studio：lms server start）
//! cargo run -p zct-train --release -- generate .corpus/text/llm.txt http://localhost:1234/v1 qwen/qwen3.5-4b
//! ```

mod generate;
mod wiki;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;
use zct_core::mixed::is_cjk;
use zct_core::ngram::{NgramModel, sentence_keys};

/// 每幾篇條目留一篇當評測語料
const HELDOUT_EVERY: usize = 50;
/// 一句至少幾個中文字才留下
const MIN_SENTENCE: usize = 4;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("extract") if args.len() >= 3 => extract(
            Path::new(&args[1]),
            &args[2..].iter().map(PathBuf::from).collect::<Vec<_>>(),
        ),
        Some("train") if args.len() >= 3 => train(&args[1..]),
        Some("generate") if args.len() >= 4 => {
            generate::run(Path::new(&args[1]), &args[2], &args[3])
        }
        _ => {
            eprintln!(
                "用法：\n  zct-train extract <輸出資料夾> <dump.bz2>…\n  zct-train train <語料.txt> <模型.bin> [--order 4] [--min 1,2,3,3]\n  zct-train generate <輸出.txt> <端點> <模型>"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("錯誤：{e}");
        std::process::exit(1);
    }
}

/// 文章 → 句子（以 。！？；和換行切開），只留夠長的
fn sentences(text: &str, out: &mut String) {
    for s in text.split(['。', '！', '？', '；', '!', '?', '\n']) {
        let s = s.trim();
        if s.chars().filter(|&c| is_cjk(c)).count() >= MIN_SENTENCE {
            out.push_str(s);
            out.push('\n');
        }
    }
}

fn extract(out_dir: &Path, dumps: &[PathBuf]) -> std::io::Result<()> {
    std::fs::create_dir_all(out_dir)?;
    let mut train = BufWriter::new(std::fs::File::create(out_dir.join("train.txt"))?);
    let mut heldout = BufWriter::new(std::fs::File::create(out_dir.join("heldout.txt"))?);
    let cc = Arc::new(
        OpenCC::from_config(BuiltinConfig::S2twp)
            .map_err(|e| std::io::Error::other(e.to_string()))?,
    );
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let (job_tx, job_rx) = mpsc::sync_channel::<(bool, String)>(threads * 4);
    let job_rx = Arc::new(std::sync::Mutex::new(job_rx));
    let (out_tx, out_rx) = mpsc::sync_channel::<(bool, String)>(threads * 4);
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let (rx, tx, cc) = (job_rx.clone(), out_tx.clone(), cc.clone());
            std::thread::spawn(move || {
                loop {
                    let job = rx.lock().unwrap().recv();
                    let Ok((held, text)) = job else { break };
                    let mut s = String::new();
                    sentences(&cc.convert(&text), &mut s);
                    if tx.send((held, s)).is_err() {
                        break;
                    }
                }
            })
        })
        .collect();
    drop(out_tx);
    let writer = std::thread::spawn(move || -> std::io::Result<(usize, usize)> {
        let (mut chars, mut lines) = (0usize, 0usize);
        for (held, s) in out_rx {
            chars += s.chars().count();
            lines += s.lines().count();
            if held {
                heldout.write_all(s.as_bytes())?;
            } else {
                train.write_all(s.as_bytes())?;
            }
        }
        train.flush()?;
        heldout.flush()?;
        Ok((chars, lines))
    });
    let mut n = 0usize;
    for dump in dumps {
        eprintln!("讀取 {}", dump.display());
        let file = std::fs::File::open(dump)?;
        let reader = BufReader::with_capacity(1 << 20, bzip2::read::MultiBzDecoder::new(file));
        wiki::articles(reader, |text| {
            n += 1;
            if n % 10_000 == 0 {
                eprintln!("  {n} 篇");
            }
            let _ = job_tx.send((n % HELDOUT_EVERY == 0, text.to_owned()));
        })?;
    }
    drop(job_tx);
    for w in workers {
        let _ = w.join();
    }
    let (chars, lines) = writer.join().expect("寫檔執行緒")?;
    eprintln!(
        "完成：{n} 篇、{lines} 句、{chars} 字 → {}",
        out_dir.display()
    );
    Ok(())
}

fn train(args: &[String]) -> std::io::Result<()> {
    let corpus = Path::new(&args[0]);
    let out = Path::new(&args[1]);
    let mut order = 4usize;
    let mut min: Vec<u32> = vec![1, 2, 3, 3];
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--order" => order = it.next().and_then(|v| v.parse().ok()).unwrap_or(order),
            "--min" => {
                min = it
                    .next()
                    .map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect())
                    .unwrap_or(min)
            }
            other => eprintln!("略過不認得的參數 {other}"),
        }
    }
    let mut maps: Vec<HashMap<u64, u32>> = vec![HashMap::new(); order];
    let mut total = 0u64;
    let mut lines = 0usize;
    let reader = BufReader::with_capacity(1 << 20, std::fs::File::open(corpus)?);
    for line in reader.lines() {
        let line = line?;
        for run in line.split(|c: char| !is_cjk(c)).filter(|r| !r.is_empty()) {
            let chars: Vec<char> = run.chars().collect();
            sentence_keys(&chars, order, |n, k| {
                let c = maps[n - 1].entry(k).or_default();
                *c = c.saturating_add(1);
                if n == 1 {
                    total += 1;
                }
            });
        }
        lines += 1;
        if lines % 1_000_000 == 0 {
            let sizes: Vec<usize> = maps.iter().map(HashMap::len).collect();
            eprintln!("  {lines} 句，各階筆數 {sizes:?}");
        }
    }
    let tables: Vec<Vec<(u64, u32)>> = maps
        .into_iter()
        .enumerate()
        .map(|(i, m)| {
            let floor = min.get(i).copied().unwrap_or(1);
            m.into_iter().filter(|&(_, c)| c >= floor).collect()
        })
        .collect();
    let sizes: Vec<usize> = tables.iter().map(Vec::len).collect();
    NgramModel::write(out, total, tables)?;
    let bytes = std::fs::metadata(out)?.len();
    eprintln!(
        "完成：{lines} 句、{total} 字，修剪後各階 {sizes:?}，模型 {:.1} MB → {}",
        bytes as f64 / 1e6,
        out.display()
    );
    Ok(())
}
