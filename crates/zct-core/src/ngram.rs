//! T0：字 n-gram 語言模型（需求 4.13），用來替 chewing 的候選句重新打分
//!
//! - 打分用 stupid backoff（Brants 2007）：只比較同一組候選句誰比較通順，不需要正規化的機率
//! - 句子前後加上邊界記號，所以「句首、句尾常見的字」也算在內
//! - 為了讓模型小到能隨安裝程式發布：每個 n-gram 只存 32 位元指紋與量化後的分數（共 5 bytes）。
//!   指紋撞到的機率約為「筆數 / 2³²」，查不到的 n-gram 偶爾被當成有，對排序的影響可以忽略
//!
//! 檔案格式（little-endian）：`ZCTNG002`、階數 u32、沒見過的字的分數 f32（log10），接著每一階：
//! 筆數 u64、指紋 u32 × 筆數（由小到大）、分數 u8 × 筆數（`-q / 32` = log10 條件機率）。

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAGIC: &[u8; 8] = b"ZCTNG002";
/// 句首、句尾記號（不會出現在中文文字裡的控制字元）
pub const BOS: char = '\u{2}';
pub const EOS: char = '\u{3}';
/// 退一階時乘上的權重（論文建議 0.4），log10
const BACKOFF_LOG: f64 = -0.397_940_008_672_037_6;
/// 分數量化：log10 每 1/32 一格，最低 -7.97
const QUANT: f64 = 32.0;

/// 異體字當成同一個字：寫法（臺／台、賬／帳）交給 chewing 決定，T0 只判斷是哪個字
///
/// 訓練語料（維基轉台灣正體）偏好「臺、佈、賬」這類寫法，一般打字多用「台、布、帳」；不合併的話，
/// 模型會把使用者慣用的寫法改掉
pub fn fold(c: char) -> char {
    match c {
        '臺' => '台',
        '賬' => '帳',
        '佈' => '布',
        '唸' => '念',
        '裏' => '裡',
        '着' => '著',
        '爲' => '為',
        '綫' => '線',
        '麪' => '麵',
        '衆' => '眾',
        '啓' => '啟',
        '峯' => '峰',
        '羣' => '群',
        '祕' => '秘',
        '纔' => '才',
        _ => c,
    }
}

/// n-gram 的鍵：FNV-1a 64 位元雜湊（以字為單位，所以階數不同的鍵不會相同；異體字先合併）
pub fn key(chars: &[char]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &c in chars {
        for b in (fold(c) as u32).to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// 存在模型裡的指紋：雜湊的高 32 位元
fn fingerprint(chars: &[char]) -> u32 {
    (key(chars) >> 32) as u32
}

#[derive(Debug, Default)]
struct Table {
    fps: Vec<u32>,
    scores: Vec<u8>,
}

impl Table {
    fn get(&self, fp: u32) -> Option<u8> {
        self.fps.binary_search(&fp).ok().map(|i| self.scores[i])
    }
}

#[derive(Debug, Default)]
pub struct NgramModel {
    tables: Vec<Table>,
    /// 語料裡沒出現過的字（log10）
    floor: f64,
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
        if take(&mut b, 8)? != MAGIC {
            return None;
        }
        let order = u32::from_le_bytes(take(&mut b, 4)?.try_into().ok()?) as usize;
        let floor = f32::from_le_bytes(take(&mut b, 4)?.try_into().ok()?) as f64;
        let mut tables = Vec::with_capacity(order.min(16));
        for _ in 0..order {
            let n = usize::try_from(u64::from_le_bytes(take(&mut b, 8)?.try_into().ok()?)).ok()?;
            let fps = take(&mut b, n.checked_mul(4)?)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect();
            let scores = take(&mut b, n)?.to_vec();
            tables.push(Table { fps, scores });
        }
        b.is_empty().then_some(NgramModel { tables, floor })
    }

    /// `w` 接在 `ctx` 後面的分數（log10，stupid backoff）
    fn prob_log(&self, ctx: &[char], w: char) -> f64 {
        let mut penalty = 0.0;
        let mut gram: Vec<char> = Vec::with_capacity(ctx.len() + 1);
        for k in (0..=ctx.len()).rev() {
            gram.clear();
            gram.extend_from_slice(&ctx[ctx.len() - k..]);
            gram.push(w);
            if let Some(q) = self.tables.get(k).and_then(|t| t.get(fingerprint(&gram))) {
                return penalty - q as f64 / QUANT;
            }
            penalty += BACKOFF_LOG;
        }
        penalty + self.floor
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
                total += self.prob_log(ctx, seq[i]);
            }
        }
        total
    }
}

/// 訓練：累計每個 n-gram 的次數與它的前文（算條件機率用）
pub struct Counter {
    /// `maps[n - 1]`：n 階的鍵 → （次數, 前 n-1 個字的鍵）
    maps: Vec<HashMap<u64, (u32, u64)>>,
    /// 總字數（含邊界記號），一階的分母
    total: u64,
}

impl Counter {
    pub fn new(order: usize) -> Counter {
        Counter {
            maps: vec![HashMap::new(); order.max(1)],
            total: 0,
        }
    }

    /// 各階目前的筆數（顯示進度用）
    pub fn sizes(&self) -> Vec<usize> {
        self.maps.iter().map(HashMap::len).collect()
    }

    /// 加入一句（只含中文字）；`weight` 是這句算幾次（少量的口語語料要加重）
    pub fn add(&mut self, sentence: &[char], weight: u32) {
        let seq: Vec<char> = std::iter::once(BOS)
            .chain(sentence.iter().copied())
            .chain(std::iter::once(EOS))
            .collect();
        for (i, map) in self.maps.iter_mut().enumerate() {
            let n = i + 1;
            for w in seq.windows(n) {
                let ctx = if n == 1 { 0 } else { key(&w[..n - 1]) };
                let e = map.entry(key(w)).or_insert((0, ctx));
                e.0 = e.0.saturating_add(weight);
            }
        }
        self.total += seq.len() as u64 * weight as u64;
    }

    /// 修剪（`min[n - 1]`：n 階至少要出現幾次）後寫出模型；回傳各階留下的筆數
    pub fn write(&self, path: &Path, min: &[u32]) -> std::io::Result<Vec<usize>> {
        let total = self.total.max(1) as f64;
        let quant = |p: f64| (-p.log10() * QUANT).round().clamp(0.0, 255.0) as u8;
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        out.write_all(MAGIC)?;
        out.write_all(&(self.maps.len() as u32).to_le_bytes())?;
        out.write_all(&((0.5 / total).log10() as f32).to_le_bytes())?;
        let mut sizes = Vec::new();
        for (i, map) in self.maps.iter().enumerate() {
            let floor = min.get(i).copied().unwrap_or(1);
            let lower = i.checked_sub(1).map(|j| &self.maps[j]);
            let mut rows: Vec<(u32, u8)> = map
                .iter()
                .filter(|(_, (c, _))| *c >= floor)
                .map(|(&k, &(c, ctx))| {
                    let denom = match lower {
                        None => total,
                        Some(m) => m.get(&ctx).map_or(c, |&(d, _)| d.max(c)) as f64,
                    };
                    ((k >> 32) as u32, quant(c as f64 / denom))
                })
                .collect();
            rows.sort_unstable_by_key(|&(fp, _)| fp);
            rows.dedup_by_key(|&mut (fp, _)| fp);
            out.write_all(&(rows.len() as u64).to_le_bytes())?;
            for &(fp, _) in &rows {
                out.write_all(&fp.to_le_bytes())?;
            }
            out.write_all(&rows.iter().map(|&(_, q)| q).collect::<Vec<_>>())?;
            sizes.push(rows.len());
        }
        out.flush()?;
        Ok(sizes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn train(sentences: &[&str], order: usize) -> NgramModel {
        let mut c = Counter::new(order);
        for s in sentences {
            c.add(&s.chars().collect::<Vec<_>>(), 1);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.bin");
        c.write(&path, &[1, 1, 1]).unwrap();
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
    fn variants_score_the_same() {
        let m = train(&["臺灣很好", "臺北下雨"], 3);
        assert_eq!(m.score("台灣很好"), m.score("臺灣很好"));
    }

    #[test]
    fn rejects_corrupt_files() {
        assert!(NgramModel::from_bytes(b"nope").is_none());
        let mut ok = MAGIC.to_vec();
        ok.extend(0u32.to_le_bytes());
        ok.extend(0f32.to_le_bytes());
        assert!(NgramModel::from_bytes(&ok).is_some());
        ok.push(0);
        assert!(NgramModel::from_bytes(&ok).is_none(), "多出來的位元組");
    }
}
