//! T0：字 n-gram 語言模型（需求 4.13），用來替 chewing 的候選句重新打分
//!
//! - 每個 n-gram 以 64 位元雜湊當鍵，各階一張排序好的表，查詢用二分搜尋；整個模型一次讀進記憶體
//! - 打分用 stupid backoff（Brants 2007）：只比較同一組候選句誰比較通順，不需要正規化的機率
//! - 句子前後加上邊界記號，所以「句首、句尾常見的字」也算在內
//!
//! 檔案格式（little-endian）：`ZCTNG001`、階數 u32、總字數 u64，接著每一階：筆數 u64、
//! 鍵 u64 × 筆數、次數 u32 × 筆數（鍵由小到大）。

use std::io::Write;
use std::path::Path;

const MAGIC: &[u8; 8] = b"ZCTNG001";
/// 句首、句尾記號（不會出現在中文文字裡的控制字元）
pub const BOS: char = '\u{2}';
pub const EOS: char = '\u{3}';
/// 退一階時乘上的權重（論文建議 0.4）
const BACKOFF: f64 = 0.4;

/// n-gram 的鍵：FNV-1a 64 位元雜湊（以字為單位，所以階數不同的鍵不會相同）
pub fn key(chars: &[char]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &c in chars {
        for b in (c as u32).to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

#[derive(Debug, Default)]
struct Table {
    keys: Vec<u64>,
    counts: Vec<u32>,
}

impl Table {
    fn get(&self, k: u64) -> u32 {
        self.keys
            .binary_search(&k)
            .map(|i| self.counts[i])
            .unwrap_or(0)
    }
}

#[derive(Debug, Default)]
pub struct NgramModel {
    tables: Vec<Table>,
    /// 訓練語料的總字數（含邊界記號），一階的分母
    total: u64,
}

impl NgramModel {
    pub fn order(&self) -> usize {
        self.tables.len()
    }

    pub fn load(path: &Path) -> std::io::Result<NgramModel> {
        let bytes = std::fs::read(path)?;
        NgramModel::from_bytes(&bytes).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "n-gram 模型格式不對")
        })
    }

    pub fn from_bytes(mut b: &[u8]) -> Option<NgramModel> {
        fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
            let (head, rest) = b.split_at_checked(n)?;
            *b = rest;
            Some(head)
        }
        let u32_ = |b: &mut &[u8]| take(b, 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()));
        let u64_ = |b: &mut &[u8]| take(b, 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()));
        if take(&mut b, 8)? != MAGIC {
            return None;
        }
        let order = u32_(&mut b)? as usize;
        let total = u64_(&mut b)?;
        let mut tables = Vec::with_capacity(order);
        for _ in 0..order {
            let n = usize::try_from(u64_(&mut b)?).ok()?;
            let keys = take(&mut b, n.checked_mul(8)?)?
                .chunks_exact(8)
                .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
                .collect();
            let counts = take(&mut b, n.checked_mul(4)?)?
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            tables.push(Table { keys, counts });
        }
        b.is_empty().then_some(NgramModel { tables, total })
    }

    /// 寫出模型；`tables[n - 1]` 是 n 階的（鍵, 次數），不必先排序
    pub fn write(path: &Path, total: u64, tables: Vec<Vec<(u64, u32)>>) -> std::io::Result<()> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        out.write_all(MAGIC)?;
        out.write_all(&(tables.len() as u32).to_le_bytes())?;
        out.write_all(&total.to_le_bytes())?;
        for mut t in tables {
            t.sort_unstable_by_key(|&(k, _)| k);
            t.dedup_by_key(|&mut (k, _)| k);
            out.write_all(&(t.len() as u64).to_le_bytes())?;
            for &(k, _) in &t {
                out.write_all(&k.to_le_bytes())?;
            }
            for &(_, c) in &t {
                out.write_all(&c.to_le_bytes())?;
            }
        }
        out.flush()
    }

    fn count(&self, gram: &[char]) -> u32 {
        self.tables
            .get(gram.len().wrapping_sub(1))
            .map_or(0, |t| t.get(key(gram)))
    }

    /// `w` 接在 `ctx` 後面的分數（stupid backoff，0～1）
    fn prob(&self, ctx: &[char], w: char) -> f64 {
        let mut mult = 1.0;
        for k in (0..=ctx.len()).rev() {
            let c = &ctx[ctx.len() - k..];
            let mut gram = c.to_vec();
            gram.push(w);
            let hit = self.count(&gram);
            if hit > 0 {
                let denom = if k == 0 {
                    self.total
                } else {
                    self.count(c) as u64
                };
                if denom > 0 {
                    return mult * hit as f64 / denom as f64;
                }
            }
            mult *= BACKOFF;
        }
        // 語料裡沒出現過的字
        mult * 0.5 / self.total.max(1) as f64
    }

    /// 一段文字的分數（log10，越大越通順）；非中文的字當成句子邊界
    pub fn score(&self, text: &str) -> f64 {
        let order = self.order().max(1);
        let mut total = 0.0;
        for run in text.split(|c: char| !crate::mixed::is_cjk(c)) {
            if run.is_empty() {
                continue;
            }
            let seq: Vec<char> = std::iter::once(BOS)
                .chain(run.chars())
                .chain(std::iter::once(EOS))
                .collect();
            for i in 1..seq.len() {
                let ctx = &seq[i.saturating_sub(order - 1)..i];
                total += self.prob(ctx, seq[i]).log10();
            }
        }
        total
    }
}

/// 訓練用：一句（只含中文字）加上邊界記號後的所有 1～`order` 階 n-gram 鍵
pub fn sentence_keys(sentence: &[char], order: usize, mut f: impl FnMut(usize, u64)) {
    let seq: Vec<char> = std::iter::once(BOS)
        .chain(sentence.iter().copied())
        .chain(std::iter::once(EOS))
        .collect();
    for n in 1..=order {
        for w in seq.windows(n) {
            f(n, key(w));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn train(sentences: &[&str], order: usize) -> NgramModel {
        let mut maps = vec![std::collections::HashMap::<u64, u32>::new(); order];
        let mut total = 0;
        for s in sentences {
            let chars: Vec<char> = s.chars().collect();
            sentence_keys(&chars, order, |n, k| {
                *maps[n - 1].entry(k).or_default() += 1;
                if n == 1 {
                    total += 1;
                }
            });
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.bin");
        NgramModel::write(
            &path,
            total,
            maps.into_iter().map(|m| m.into_iter().collect()).collect(),
        )
        .unwrap();
        NgramModel::load(&path).unwrap()
    }

    #[test]
    fn prefers_seen_context() {
        let m = train(
            &["確認後再部署", "明天再說", "下次再來", "我在家", "他在公司"],
            3,
        );
        assert_eq!(m.order(), 3);
        assert!(m.score("下次再說") > m.score("下次在說"));
        assert!(m.score("我在公司") > m.score("我再公司"));
    }

    #[test]
    fn rejects_corrupt_files() {
        assert!(NgramModel::from_bytes(b"nope").is_none());
        let mut ok = MAGIC.to_vec();
        ok.extend(0u32.to_le_bytes());
        ok.extend(0u64.to_le_bytes());
        assert!(NgramModel::from_bytes(&ok).is_some());
        ok.push(0);
        assert!(NgramModel::from_bytes(&ok).is_none(), "多出來的位元組");
    }
}
