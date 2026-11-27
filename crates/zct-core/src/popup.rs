//! 候選窗、組字提示列、提示泡泡的排版（兩個平台共用）
//!
//! 平台只量文字（[`Measure`]）、照結果畫（[`Popup`]）：文字畫在哪、選擇框與游標的位置、視窗大小、滑鼠點得到的範圍。
//! 彈出位置也在這裡（[`place`]）：預設在游標下方，放不下就改到上方，左右上下不超出螢幕。
//! 座標以視窗左上角為原點、往下為正，單位和平台量文字的單位相同。

use serde::Serialize;

use crate::session::{CandidatePage, Preedit};

/// 數字字級相對於候選字字級
pub const NUM_RATIO: f32 = 0.75;
/// 視窗內距
pub const PAD_X: f32 = 6.0;
pub const PAD_Y: f32 = 4.0;
/// 選擇框左右內距
pub const ROW_PAD_X: f32 = 6.0;
/// 每列上下內距
pub const ROW_PAD_Y: f32 = 2.0;
/// 數字與候選字之間的距離
pub const GAP_NUM: f32 = 8.0;
/// 選擇框圓角
pub const ROW_RADIUS: f32 = 4.0;
/// 視窗與游標之間的距離
pub const OFFSET: f32 = 4.0;
/// 提示泡泡（中／A、全形…）顯示多久
pub const HINT_MS: u64 = 800;
/// 組字提示列的游標寬度
pub const CARET_WIDTH: f32 = 1.5;

/// 文字用哪種字型：候選字（主題字型與字級）或數字（字級 × [`NUM_RATIO`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Font {
    Text,
    Number,
}

/// 文字顏色：主題的文字色或數字色（數字、頁碼、不是目前這一欄的候選）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Ink {
    Text,
    Dim,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// 一段要畫的文字（左上角）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Run {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub font: Font,
    pub ink: Ink,
}

/// 滑鼠點這裡選第 `page` 頁第 `row` 個候選
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Hit {
    pub rect: Rect,
    pub page: usize,
    pub row: usize,
}

/// 排好的視窗內容
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Popup {
    pub width: f32,
    pub height: f32,
    /// 用主題的選擇框顏色畫的圓角框（圓角 [`ROW_RADIUS`]）：選擇框、組字提示列的選字目標
    pub highlights: Vec<Rect>,
    pub runs: Vec<Run>,
    /// 組字提示列的游標（主題文字色）
    pub caret: Option<Rect>,
    pub hits: Vec<Hit>,
}

impl Popup {
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// 滑鼠點在 (x, y)：選哪一頁哪一個候選
    pub fn hit(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        self.hits
            .iter()
            .find(|h| h.rect.contains(x, y))
            .map(|h| (h.page, h.row))
    }

    fn shifted(mut self, dy: f32) -> Popup {
        for r in &mut self.runs {
            r.y += dy;
        }
        for h in &mut self.highlights {
            h.y += dy;
        }
        for h in &mut self.hits {
            h.rect.y += dy;
        }
        if let Some(c) = &mut self.caret {
            c.y += dy;
        }
        self
    }
}

/// 平台量文字：回傳（寬, 高）
pub trait Measure {
    fn measure(&mut self, text: &str, font: Font) -> (f32, f32);
}

impl<F: FnMut(&str, Font) -> (f32, f32)> Measure for F {
    fn measure(&mut self, text: &str, font: Font) -> (f32, f32) {
        self(text, font)
    }
}

fn run(text: impl Into<String>, x: f32, y: f32, font: Font, ink: Ink) -> Run {
    Run {
        text: text.into(),
        x,
        y,
        font,
        ink,
    }
}

fn page_label(page: &CandidatePage) -> String {
    format!("{}/{}", page.page + 1, page.total_pages)
}

/// 候選窗：只顯示的注音小視窗是一列文字；展開多欄、直式、橫式
pub fn candidates(page: &CandidatePage, horizontal: bool, m: &mut impl Measure) -> Popup {
    if page.items.is_empty() && page.columns.is_empty() {
        return Popup::default();
    }
    if page.is_display_only() {
        plain(&page.items.join("  "), m)
    } else if !page.columns.is_empty() {
        columns(page, m)
    } else if horizontal {
        row(page, m)
    } else {
        vertical(page, m)
    }
}

/// 一列文字，沒有數字與選擇框（打字時的注音小視窗）
fn plain(text: &str, m: &mut impl Measure) -> Popup {
    let (w, h) = m.measure(text, Font::Text);
    let row_h = h + ROW_PAD_Y * 2.0;
    Popup {
        width: PAD_X * 2.0 + ROW_PAD_X * 2.0 + w,
        height: row_h + PAD_Y * 2.0,
        runs: vec![run(
            text,
            PAD_X + ROW_PAD_X,
            PAD_Y + ROW_PAD_Y,
            Font::Text,
            Ink::Text,
        )],
        ..Popup::default()
    }
}

/// 橫式：「1 天 2 添 …」排成一列，選擇框框住目前項目，最後是頁碼
fn row(page: &CandidatePage, m: &mut impl Measure) -> Popup {
    let mut cells = Vec::new();
    let mut row_h: f32 = 0.0;
    for (i, item) in page.items.iter().enumerate() {
        let label = page.row_label(i);
        let num = m.measure(&label, Font::Number);
        let text = m.measure(item, Font::Text);
        row_h = row_h.max(text.1).max(num.1);
        cells.push((label, num, text));
    }
    let row_h = row_h + ROW_PAD_Y * 2.0;
    let mut p = Popup::default();
    let mut x = PAD_X;
    for (i, (label, (nw, nh), (tw, th))) in cells.into_iter().enumerate() {
        let w = ROW_PAD_X + nw + GAP_NUM / 2.0 + tw + ROW_PAD_X;
        let cell = Rect {
            x,
            y: PAD_Y,
            w,
            h: row_h,
        };
        if page.selected_row() == Some(i) {
            p.highlights.push(cell);
        }
        p.hits.push(Hit {
            rect: cell,
            page: page.page,
            row: i,
        });
        p.runs.push(run(
            label,
            x + ROW_PAD_X,
            PAD_Y + (row_h - nh) / 2.0,
            Font::Number,
            Ink::Dim,
        ));
        p.runs.push(run(
            &page.items[i],
            x + ROW_PAD_X + nw + GAP_NUM / 2.0,
            PAD_Y + (row_h - th) / 2.0,
            Font::Text,
            Ink::Text,
        ));
        x += w;
    }
    if page.total_pages > 1 {
        let label = page_label(page);
        let (w, h) = m.measure(&label, Font::Number);
        p.runs.push(run(
            label,
            x + ROW_PAD_X,
            PAD_Y + (row_h - h) / 2.0,
            Font::Number,
            Ink::Dim,
        ));
        x += w + ROW_PAD_X * 2.0;
    }
    p.width = x + PAD_X;
    p.height = row_h + PAD_Y * 2.0;
    p
}

/// 直式：每列「數字 候選字」，選擇框標示目前項目，多頁時底部顯示頁碼
fn vertical(page: &CandidatePage, m: &mut impl Measure) -> Popup {
    // 數字欄寬固定，候選字才會對齊
    let (num_w, _) = m.measure("0", Font::Number);
    let mut rows = Vec::new();
    let mut row_h: f32 = 0.0;
    let mut text_w: f32 = 0.0;
    for (i, item) in page.items.iter().enumerate() {
        let label = page.row_label(i);
        let num = m.measure(&label, Font::Number);
        let text = m.measure(item, Font::Text);
        row_h = row_h.max(text.1).max(num.1);
        text_w = text_w.max(text.0);
        rows.push((label, num, text));
    }
    let row_h = row_h + ROW_PAD_Y * 2.0;
    let row_w = ROW_PAD_X + num_w + GAP_NUM + text_w + ROW_PAD_X;

    let mut p = Popup::default();
    let mut y = PAD_Y;
    for (i, (label, (nw, nh), (_, th))) in rows.into_iter().enumerate() {
        let cell = Rect {
            x: PAD_X,
            y,
            w: row_w,
            h: row_h,
        };
        if page.selected_row() == Some(i) {
            p.highlights.push(cell);
        }
        p.hits.push(Hit {
            rect: cell,
            page: page.page,
            row: i,
        });
        let x0 = PAD_X + ROW_PAD_X;
        // 數字靠右對齊數字欄
        p.runs.push(run(
            label,
            x0 + num_w - nw,
            y + (row_h - nh) / 2.0,
            Font::Number,
            Ink::Dim,
        ));
        p.runs.push(run(
            &page.items[i],
            x0 + num_w + GAP_NUM,
            y + (row_h - th) / 2.0,
            Font::Text,
            Ink::Text,
        ));
        y += row_h;
    }
    let mut width = row_w;
    if page.total_pages > 1 {
        let label = page_label(page);
        let (w, h) = m.measure(&label, Font::Number);
        width = width.max(w + ROW_PAD_X * 2.0);
        p.runs.push(run(
            label,
            PAD_X + width - ROW_PAD_X - w,
            y + ROW_PAD_Y,
            Font::Number,
            Ink::Dim,
        ));
        y += h + ROW_PAD_Y * 2.0;
    }
    // 選擇框與點擊範圍跟整個視窗一樣寬
    for r in p
        .highlights
        .iter_mut()
        .chain(p.hits.iter_mut().map(|h| &mut h.rect))
    {
        r.w = width;
    }
    p.width = PAD_X * 2.0 + width;
    p.height = y + PAD_Y;
    p
}

/// 展開多欄：每一頁一欄並排，目前這一欄有數字與選擇框，其他欄淡色；底部顯示頁碼
fn columns(page: &CandidatePage, m: &mut impl Measure) -> Popup {
    let active = page.page.saturating_sub(page.first_column);
    let (num_w, _) = m.measure("0★", Font::Number);
    let mut cols = Vec::new();
    let mut row_h: f32 = 0.0;
    for (c, items) in page.visible_columns().enumerate() {
        let mut rows = Vec::new();
        let mut text_w: f32 = 0.0;
        for (i, item) in items.iter().enumerate() {
            let num = (c == active).then(|| {
                let label = page.row_label(i);
                let size = m.measure(&label, Font::Number);
                (label, size)
            });
            let text = m.measure(item, Font::Text);
            row_h = row_h.max(text.1);
            text_w = text_w.max(text.0);
            rows.push((num, item.clone(), text));
        }
        cols.push((rows, text_w));
    }
    let row_h = row_h + ROW_PAD_Y * 2.0;

    let mut p = Popup::default();
    let mut x = PAD_X;
    let mut bottom = PAD_Y;
    for (c, (rows, text_w)) in cols.into_iter().enumerate() {
        let col_w = ROW_PAD_X + num_w + GAP_NUM + text_w + ROW_PAD_X;
        let mut y = PAD_Y;
        for (i, (num, item, (_, th))) in rows.into_iter().enumerate() {
            let cell = Rect {
                x,
                y,
                w: col_w,
                h: row_h,
            };
            if c == active && page.selected_row() == Some(i) {
                p.highlights.push(cell);
            }
            p.hits.push(Hit {
                rect: cell,
                page: page.first_column + c,
                row: i,
            });
            if let Some((label, (nw, nh))) = num {
                p.runs.push(run(
                    label,
                    x + ROW_PAD_X + num_w - nw,
                    y + (row_h - nh) / 2.0,
                    Font::Number,
                    Ink::Dim,
                ));
            }
            p.runs.push(run(
                item,
                x + ROW_PAD_X + num_w + GAP_NUM,
                y + (row_h - th) / 2.0,
                Font::Text,
                if c == active { Ink::Text } else { Ink::Dim },
            ));
            y += row_h;
        }
        bottom = bottom.max(y);
        x += col_w;
    }
    let width = x - PAD_X;
    let label = page_label(page);
    let (w, h) = m.measure(&label, Font::Number);
    p.runs.push(run(
        label,
        PAD_X + width - ROW_PAD_X - w,
        bottom + ROW_PAD_Y,
        Font::Number,
        Ink::Dim,
    ));
    p.width = PAD_X * 2.0 + width;
    p.height = bottom + h + ROW_PAD_Y * 2.0 + PAD_Y;
    p
}

/// 組字提示列：「組字內容（選字目標反白）　N 字」，有游標
pub fn composition(preedit: &Preedit, m: &mut impl Measure) -> Popup {
    if preedit.text.is_empty() {
        return Popup::default();
    }
    let display = preedit.display();
    let pieces = [display.before_target, display.target, display.after_target];
    let mut p = Popup::default();
    let mut x = PAD_X + ROW_PAD_X;
    let mut h: f32 = 0.0;
    let mut target = None;
    for (i, piece) in pieces.into_iter().enumerate() {
        if piece.is_empty() {
            continue;
        }
        let (w, lh) = m.measure(piece, Font::Text);
        h = h.max(lh);
        if i == 1 {
            target = Some((x, w));
        }
        p.runs
            .push(run(piece, x, PAD_Y + ROW_PAD_Y, Font::Text, Ink::Text));
        x += w;
    }
    let row_h = h + ROW_PAD_Y * 2.0;
    if let Some((tx, tw)) = target {
        p.highlights.push(Rect {
            x: tx - 1.0,
            y: PAD_Y,
            w: tw + 2.0,
            h: row_h,
        });
    }
    let count = format!("{} 字", preedit.len);
    let (cw, ch) = m.measure(&count, Font::Number);
    p.runs.push(run(
        count,
        x + GAP_NUM * 2.0,
        PAD_Y + (row_h - ch) / 2.0,
        Font::Number,
        Ink::Dim,
    ));
    // 游標：游標前文字的寬度
    let x0 = PAD_X + ROW_PAD_X;
    let caret_x = if display.before_cursor.is_empty() {
        x0
    } else {
        x0 + m.measure(display.before_cursor, Font::Text).0
    };
    p.caret = Some(Rect {
        x: caret_x - CARET_WIDTH / 2.0,
        y: PAD_Y + ROW_PAD_Y,
        w: CARET_WIDTH,
        h: row_h - ROW_PAD_Y * 2.0,
    });
    p.width = x + GAP_NUM * 2.0 + cw + ROW_PAD_X + PAD_X;
    p.height = row_h + PAD_Y * 2.0;
    p
}

/// 提示泡泡：一段文字
pub fn hint(text: &str, m: &mut impl Measure) -> Popup {
    if text.is_empty() {
        return Popup::default();
    }
    let (w, h) = m.measure(text, Font::Text);
    Popup {
        width: w + PAD_X * 2.0,
        height: h + PAD_Y * 2.0,
        runs: vec![run(text, PAD_X, PAD_Y, Font::Text, Ink::Text)],
        ..Popup::default()
    }
}

/// 幾個排好的內容由上往下疊在同一個視窗（macOS 把組字提示列、候選窗、提示放在同一個面板）：
/// 回傳各部分（已往下移到疊好的位置，空的不佔高度）與整個視窗的（寬, 高）
pub fn arrange<const N: usize>(parts: [Popup; N]) -> ([Popup; N], (f32, f32)) {
    let (mut width, mut height) = (0.0f32, 0.0f32);
    let parts = parts.map(|part| {
        if part.is_empty() {
            return part;
        }
        let h = part.height;
        let part = part.shifted(height);
        width = width.max(part.width);
        height += h;
        part
    });
    (parts, (width, height))
}

/// 疊成一個視窗（見 [`arrange`]）
pub fn stack<const N: usize>(parts: [Popup; N]) -> Popup {
    let (parts, (width, height)) = arrange(parts);
    let mut out = Popup {
        width,
        height,
        ..Popup::default()
    };
    for part in parts {
        out.runs.extend(part.runs);
        out.highlights.extend(part.highlights);
        out.hits.extend(part.hits);
        if part.caret.is_some() {
            out.caret = part.caret;
        }
    }
    out
}

/// 組字提示列的位置：組字開始時的游標位置，組字結束前不跟著游標移動。
/// App 剛輸出完常回報不出位置（VS Code 等 Chromium App）：先用沿用的舊位置，App 回報後再移過去
#[derive(Debug, Clone, Copy)]
pub struct BarAnchor<T> {
    anchor: Option<T>,
    /// 固定的位置是 App 回報的（不是沿用的舊位置）
    fresh: bool,
}

impl<T> Default for BarAnchor<T> {
    fn default() -> Self {
        BarAnchor {
            anchor: None,
            fresh: false,
        }
    }
}

impl<T: Copy> BarAnchor<T> {
    /// `composing`：組字區有內容；`caret`：這次的游標位置；`fresh`：`caret` 是 App 回報的。
    /// 回傳組字提示列要放的位置
    pub fn update(&mut self, composing: bool, caret: Option<T>, fresh: bool) -> Option<T> {
        if !composing {
            self.reset();
            return caret;
        }
        if caret.is_some() && (self.anchor.is_none() || (fresh && !self.fresh)) {
            self.anchor = caret;
            self.fresh = fresh;
        }
        self.anchor.or(caret)
    }

    /// 失焦、取消組字
    pub fn reset(&mut self) {
        self.anchor = None;
        self.fresh = false;
    }
}

/// 三個彈出視窗（兩平台都是各自獨立的視窗，不互相推擠）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// 組字提示列：固定在組字開始的位置
    Bar,
    /// 候選窗（含打字時的注音小視窗）
    Candidates,
    /// 提示文字（例如切換中英）：自己計時消失
    Hint,
}

impl PopupKind {
    /// 優先放在位置的上方（組字提示列，避開下方的候選窗）；其他放下方
    pub const fn above(self) -> bool {
        matches!(self, PopupKind::Bar)
    }
}

/// 一個視窗這次要怎麼處理
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction<T> {
    /// 顯示在這個位置（None：App 回報不出位置，平台沿用上次的位置）
    Show(Option<T>),
    Hide,
    /// 不動（提示照自己的時間消失，之後的按鍵不會提早收掉）
    Keep,
}

/// 這次按鍵結果的內容
#[derive(Debug, Clone, Copy, Default)]
pub struct PopupContent {
    /// 組字區有內容
    pub composing: bool,
    /// 設定要顯示組字提示列
    pub composition_bar: bool,
    /// 有候選字（或注音小視窗）
    pub candidates: bool,
    /// 有提示文字
    pub notice: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupPlan<T> {
    pub bar: PopupAction<T>,
    pub candidates: PopupAction<T>,
    pub hint: PopupAction<T>,
}

/// 每次按鍵後三個視窗要怎麼顯示（兩平台共用，平台只負責畫和量游標）
#[derive(Debug, Clone, Copy)]
pub struct Popups<T> {
    bar: BarAnchor<T>,
}

impl<T> Default for Popups<T> {
    fn default() -> Self {
        Popups {
            bar: BarAnchor::default(),
        }
    }
}

impl<T: Copy> Popups<T> {
    /// `caret`：這次的游標位置；`fresh`：`caret` 是 App 回報的（不是沿用的舊位置）
    pub fn plan(&mut self, content: PopupContent, caret: Option<T>, fresh: bool) -> PopupPlan<T> {
        let bar_anchor = self.bar.update(content.composing, caret, fresh);
        PopupPlan {
            bar: if content.composing && content.composition_bar {
                PopupAction::Show(bar_anchor)
            } else {
                PopupAction::Hide
            },
            candidates: if content.candidates {
                PopupAction::Show(caret)
            } else {
                PopupAction::Hide
            },
            hint: if content.notice {
                PopupAction::Show(caret)
            } else {
                PopupAction::Keep
            },
        }
    }

    /// 失焦、取消組字
    pub fn reset(&mut self) {
        self.bar.reset();
    }
}

/// [`place`] 用在座標往上為正的平台（macOS）：`anchor`、`work` 與回傳的位置都是左下角
pub fn place_y_up(anchor: Rect, size: (f32, f32), work: Rect, gap: f32, above: bool) -> (f32, f32) {
    let flip = |r: Rect| Rect {
        y: -(r.y + r.h),
        ..r
    };
    let (x, top) = place(flip(anchor), size, flip(work), gap, above);
    (x, -(top + size.1))
}

/// 彈出位置（左上角）：`anchor` 是游標，`work` 是螢幕可用範圍，座標往下為正。
/// 預設在游標下方，超出螢幕下緣就改放上方；`above` 優先放上方（組字提示列，避開下方的候選窗），
/// 上方放不下再改下方。左右上下都不超出螢幕
pub fn place(anchor: Rect, size: (f32, f32), work: Rect, gap: f32, above: bool) -> (f32, f32) {
    let (w, h) = size;
    let bottom = anchor.y + anchor.h.max(1.0);
    let mut y = if above {
        anchor.y - gap - h
    } else {
        bottom + gap
    };
    if above && y < work.y {
        y = bottom + gap;
    } else if !above && y + h > work.bottom() {
        y = anchor.y - gap - h;
    }
    let x = anchor.x.clamp(work.x, (work.right() - w).max(work.x));
    let y = y.clamp(work.y, (work.bottom() - h).max(work.y));
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 測試用量法：每個字寬 10、數字字型寬 7，高 20 / 15
    fn measure(text: &str, font: Font) -> (f32, f32) {
        let n = text.chars().count() as f32;
        match font {
            Font::Text => (n * 10.0, 20.0),
            Font::Number => (n * 7.0, 15.0),
        }
    }

    fn page(items: &[&str]) -> CandidatePage {
        CandidatePage {
            items: items.iter().map(|s| s.to_string()).collect(),
            page: 0,
            total_pages: 1,
            highlight: 1,
            ..Default::default()
        }
    }

    #[test]
    fn vertical_rows_align_and_highlight_the_selected_row() {
        let p = candidates(&page(&["天氣", "添"]), false, &mut measure);
        // 列高 20 + 4；數字欄寬 7；候選字從 6 + 6 + 7 + 8 = 27 開始
        let texts: Vec<_> = p.runs.iter().filter(|r| r.font == Font::Text).collect();
        assert_eq!((texts[0].x, texts[0].y), (27.0, 6.0));
        assert_eq!((texts[1].x, texts[1].y), (27.0, 30.0));
        let row_w = 6.0 + 7.0 + 8.0 + 20.0 + 6.0;
        assert_eq!(
            p.highlights,
            [Rect {
                x: 6.0,
                y: 28.0,
                w: row_w,
                h: 24.0
            }]
        );
        assert_eq!((p.width, p.height), (row_w + 12.0, 4.0 + 48.0 + 4.0));
        assert_eq!(p.hit(10.0, 30.0), Some((0, 1)));
        assert_eq!(p.hit(10.0, 5.0), Some((0, 0)));
        assert_eq!(p.hit(10.0, 3.0), None);
    }

    #[test]
    fn page_number_shows_only_with_several_pages() {
        let mut pg = page(&["天"]);
        let one = candidates(&pg, false, &mut measure);
        assert!(one.runs.iter().all(|r| r.text != "1/1"));
        pg.total_pages = 3;
        pg.page = 1;
        let many = candidates(&pg, false, &mut measure);
        assert!(
            many.runs
                .iter()
                .any(|r| r.text == "2/3" && r.ink == Ink::Dim)
        );
        let row = candidates(&pg, true, &mut measure);
        assert!(row.runs.iter().any(|r| r.text == "2/3"));
    }

    #[test]
    fn horizontal_cells_follow_each_other() {
        let p = candidates(&page(&["天", "添"]), true, &mut measure);
        let cell = 6.0 + 7.0 + 4.0 + 10.0 + 6.0;
        assert_eq!(p.hits[0].rect.x, 6.0);
        assert_eq!(p.hits[1].rect.x, 6.0 + cell);
        assert_eq!(p.highlights, [p.hits[1].rect]);
        assert_eq!(p.width, 6.0 + cell * 2.0 + 6.0);
    }

    #[test]
    fn columns_dim_inactive_columns_and_hit_their_pages() {
        let mut pg = page(&["天", "添"]);
        pg.columns = vec![vec!["田".into()], vec![], vec!["甜".into()]];
        pg.first_column = 2;
        pg.page = 3;
        pg.total_pages = 5;
        let p = candidates(&pg, false, &mut measure);
        let tian = p.runs.iter().find(|r| r.text == "田").unwrap();
        assert_eq!(tian.ink, Ink::Dim, "不是目前這一欄");
        assert!(p.runs.iter().any(|r| r.text == "天" && r.ink == Ink::Text));
        let hit = p.hits.iter().find(|h| h.page == 2).unwrap();
        assert_eq!(hit.row, 0);
        assert!(p.runs.iter().any(|r| r.text == "4/5"));
        assert_eq!(p.highlights.len(), 1);
    }

    #[test]
    fn display_only_popup_is_one_plain_line() {
        let p = candidates(
            &CandidatePage::display_only(vec!["ㄋㄧ".into(), "你".into()]),
            false,
            &mut measure,
        );
        assert_eq!(p.runs.len(), 1);
        assert_eq!(p.runs[0].text, "ㄋㄧ  你");
        assert!(p.highlights.is_empty() && p.hits.is_empty(), "不能選");
    }

    #[test]
    fn composition_marks_target_caret_and_count() {
        let preedit = Preedit {
            text: "天氣很好".into(),
            cursor: 2,
            target: Some((2, 2)),
            len: 4,
        };
        let p = composition(&preedit, &mut measure);
        assert_eq!(p.highlights[0].x, 12.0 + 20.0 - 1.0);
        assert_eq!(p.highlights[0].w, 22.0);
        assert_eq!(p.caret.unwrap().x, 12.0 + 20.0 - CARET_WIDTH / 2.0);
        assert!(p.runs.iter().any(|r| r.text == "4 字"));
    }

    #[test]
    fn stacked_parts_keep_their_hits() {
        let comp = composition(
            &Preedit {
                text: "天".into(),
                cursor: 1,
                target: None,
                len: 1,
            },
            &mut measure,
        );
        let cands = candidates(&page(&["天"]), false, &mut measure);
        let comp_h = comp.height;
        let p = stack([
            comp,
            Popup::default(),
            cands.clone(),
            hint("中", &mut measure),
        ]);
        assert_eq!(p.hits[0].rect.y, cands.hits[0].rect.y + comp_h);
        assert_eq!(p.height, comp_h + cands.height + 28.0);
        assert!(p.caret.is_some());
    }

    #[test]
    fn placement_flips_above_and_stays_on_screen() {
        let work = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 800.0,
        };
        let caret = Rect {
            x: 100.0,
            y: 300.0,
            w: 1.0,
            h: 20.0,
        };
        assert_eq!(
            place(caret, (200.0, 100.0), work, 4.0, false),
            (100.0, 324.0)
        );
        let low = Rect { y: 750.0, ..caret };
        assert_eq!(
            place(low, (200.0, 100.0), work, 4.0, false),
            (100.0, 646.0),
            "下面放不下改上面"
        );
        assert_eq!(
            place(caret, (200.0, 100.0), work, 4.0, true),
            (100.0, 196.0),
            "組字提示列放上面"
        );
        let top = Rect { y: 10.0, ..caret };
        assert_eq!(
            place(top, (200.0, 100.0), work, 4.0, true),
            (100.0, 34.0),
            "上面放不下改下面"
        );
        let right = Rect { x: 950.0, ..caret };
        assert_eq!(
            place(right, (200.0, 100.0), work, 4.0, false).0,
            800.0,
            "不超出右邊"
        );
    }

    #[test]
    fn composition_bar_stays_put_until_composition_ends() {
        let mut bar = BarAnchor::default();
        assert_eq!(bar.update(true, Some(1), true), Some(1), "組字開始時的位置");
        assert_eq!(bar.update(true, Some(2), true), Some(1), "組字中不跟著游標");
        assert_eq!(bar.update(false, Some(3), true), Some(3), "組字結束放開");
        // 先拿到沿用的舊位置：App 回報後改用新位置，之後就固定
        assert_eq!(bar.update(true, Some(4), false), Some(4));
        assert_eq!(bar.update(true, Some(5), true), Some(5));
        assert_eq!(bar.update(true, Some(6), true), Some(5));
        bar.reset();
        assert_eq!(bar.update(true, None, true), None);
        assert_eq!(bar.update(true, Some(7), false), Some(7));
    }

    #[test]
    fn popup_plan_keeps_windows_independent() {
        use PopupAction::*;
        let mut popups = Popups::default();
        let typing = PopupContent {
            composing: true,
            composition_bar: true,
            candidates: true,
            notice: false,
        };
        let plan = popups.plan(typing, Some(1), true);
        assert_eq!(plan.bar, Show(Some(1)), "組字提示列在組字開始的位置");
        assert_eq!(plan.candidates, Show(Some(1)), "候選窗在游標");
        assert_eq!(plan.hint, Keep, "沒有新提示時舊提示照自己的時間消失");
        let plan = popups.plan(typing, Some(2), true);
        assert_eq!(plan.bar, Show(Some(1)), "組字中提示列不動");
        assert_eq!(plan.candidates, Show(Some(2)), "候選窗跟著游標");
        // 英文優先打字時只有注音小視窗、關掉組字提示列：提示列收起，候選窗照樣顯示
        let plan = popups.plan(
            PopupContent {
                composition_bar: false,
                ..typing
            },
            Some(3),
            true,
        );
        assert_eq!((plan.bar, plan.candidates), (Hide, Show(Some(3))));
        // 切換中英：組字結束，只顯示提示
        let plan = popups.plan(
            PopupContent {
                notice: true,
                ..PopupContent::default()
            },
            Some(4),
            true,
        );
        assert_eq!(
            plan,
            PopupPlan {
                bar: Hide,
                candidates: Hide,
                hint: Show(Some(4))
            }
        );
        assert!(
            PopupKind::Bar.above() && !PopupKind::Candidates.above() && !PopupKind::Hint.above()
        );
    }

    #[test]
    fn y_up_placement_mirrors_y_down() {
        // macOS：螢幕高 800，游標在下緣附近（y 往上為正）
        let work = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 800.0,
        };
        let caret = Rect {
            x: 100.0,
            y: 480.0,
            w: 1.0,
            h: 20.0,
        };
        assert_eq!(
            place_y_up(caret, (200.0, 100.0), work, 4.0, false),
            (100.0, 376.0),
            "放在游標下方"
        );
        let low = Rect { y: 30.0, ..caret };
        assert_eq!(
            place_y_up(low, (200.0, 100.0), work, 4.0, false),
            (100.0, 54.0),
            "下面放不下改上面"
        );
    }
}
