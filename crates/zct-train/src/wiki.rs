//! 維基百科 dump：從 XML 串流取出條目內文，去掉維基語法，留下一般文字

use std::io::BufRead;

/// 逐頁讀出主要命名空間（條目）的內文；重新導向頁略過
pub fn articles(reader: impl BufRead, mut f: impl FnMut(&str)) -> std::io::Result<()> {
    let mut ns0 = false;
    let mut in_text = false;
    let mut text = String::new();
    for line in reader.lines() {
        let line = line?;
        let t = line.trim_start();
        if t.starts_with("<page>") {
            ns0 = false;
        } else if t.starts_with("<ns>") {
            ns0 = t == "<ns>0</ns>";
        }
        if !in_text {
            let Some(start) = line.find("<text") else {
                continue;
            };
            let Some(gt) = line[start..].find('>') else {
                continue;
            };
            let body = &line[start + gt + 1..];
            // <text ... /> 是空的
            if line[start..start + gt].ends_with('/') {
                continue;
            }
            text.clear();
            in_text = true;
            match body.find("</text>") {
                Some(end) => {
                    text.push_str(&body[..end]);
                    in_text = false;
                }
                None => {
                    text.push_str(body);
                    text.push('\n');
                }
            }
        } else if let Some(end) = line.find("</text>") {
            text.push_str(&line[..end]);
            in_text = false;
        } else {
            text.push_str(&line);
            text.push('\n');
        }
        if !in_text && ns0 && !text.is_empty() {
            let body = unescape(&text);
            let lower = body.trim_start().to_lowercase();
            if !lower.starts_with("#redirect") && !lower.starts_with("#重定向") {
                f(&clean(&body));
            }
            text.clear();
        }
    }
    Ok(())
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// 刪掉成對的區塊（可巢狀），例如 `{{` … `}}`
fn strip_nested(s: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut rest = s;
    while !rest.is_empty() {
        if rest.starts_with(open) {
            depth += 1;
            rest = &rest[open.len()..];
        } else if depth > 0 && rest.starts_with(close) {
            depth -= 1;
            rest = &rest[close.len()..];
        } else {
            let c = rest.chars().next().unwrap();
            if depth == 0 {
                out.push(c);
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// 刪掉 `<tag …>…</tag>` 整段
fn strip_element(s: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(&open) {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(gt) = after.find('>') else {
            rest = "";
            break;
        };
        // 自己結束的 <ref name="x" />
        if after[..gt].ends_with('/') {
            rest = &after[gt + 1..];
            continue;
        }
        rest = match after.find(&close) {
            Some(j) => &after[j + close.len()..],
            None => &after[gt + 1..],
        };
    }
    out.push_str(rest);
    out
}

/// 刪掉剩下的 HTML 標籤（留內容）
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// `-{zh-hans:简;zh-hant:繁;}-` 取台灣 / 繁體的寫法；`-{字}-` 留字
fn language_variants(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("-{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find("}-") else {
            rest = after;
            continue;
        };
        let inner = &after[..j];
        if inner.contains(':') {
            let pick = ["zh-tw:", "zh-hant:", "zh-hk:", "zh:"]
                .iter()
                .find_map(|p| {
                    inner
                        .split(';')
                        .find_map(|part| part.trim().strip_prefix(p).map(str::to_owned))
                })
                .unwrap_or_default();
            out.push_str(&pick);
        } else {
            out.push_str(inner);
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    out
}

/// `[[目標|顯示]]` → 顯示；`[[檔案:…]]`、`[[Category:…]]` 整個刪掉
fn links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("[[") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find("]]") else {
            rest = after;
            continue;
        };
        let inner = &after[..j];
        let lower = inner.to_lowercase();
        let skip = [
            "file:",
            "image:",
            "category:",
            "檔案:",
            "文件:",
            "图像:",
            "圖像:",
            "分类:",
            "分類:",
        ]
        .iter()
        .any(|p| lower.starts_with(p));
        if !skip {
            out.push_str(inner.rsplit('|').next().unwrap_or(inner));
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    out
}

/// 維基語法 → 一般文字；表格、清單符號、標題行都拿掉
pub fn clean(text: &str) -> String {
    let s = strip_nested(text, "<!--", "-->");
    let s = strip_element(&s, "ref");
    let s = strip_element(&s, "math");
    let s = strip_element(&s, "gallery");
    let s = strip_element(&s, "syntaxhighlight");
    let s = strip_element(&s, "code");
    let s = strip_nested(&s, "{{", "}}");
    let s = strip_nested(&s, "{|", "|}");
    let s = language_variants(&s);
    let s = links(&s);
    let s = strip_tags(&s);
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('=')
            || line.starts_with('|')
            || line.starts_with('!')
        {
            continue;
        }
        let line = line.trim_start_matches(['*', '#', ':', ';', ' ']);
        out.push_str(&line.replace("'''", "").replace("''", ""));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_removes_markup() {
        let src = "'''臺北市'''是[[中華民國]]的[[首都|首都城市]]<ref name=\"a\">來源</ref>。{{Infobox|x={{y}}}}\n\
                   == 歷史 ==\n* 清朝時[[File:a.jpg|thumb|圖]]設府<!-- 註解 -->。\n{| class=\"wikitable\"\n|格\n|}\n\
                   -{zh-hans:软件;zh-tw:軟體;}-很好用<ref name=\"b\" />。";
        assert_eq!(
            clean(src),
            "臺北市是中華民國的首都城市。\n清朝時設府。\n軟體很好用。\n"
        );
    }

    #[test]
    fn articles_skip_other_namespaces_and_redirects() {
        let xml = "<page>\n<ns>0</ns>\n<text bytes=\"9\">第一篇&lt;br&gt;內容</text>\n</page>\n\
                   <page>\n<ns>4</ns>\n<text>維基計畫頁</text>\n</page>\n\
                   <page>\n<ns>0</ns>\n<text>#REDIRECT [[別處]]</text>\n</page>\n\
                   <page>\n<ns>0</ns>\n<text xml:space=\"preserve\">多行\n第二行</text>\n</page>\n";
        let mut got = Vec::new();
        articles(xml.as_bytes(), |t| got.push(t.to_owned())).unwrap();
        assert_eq!(got, vec!["第一篇內容\n", "多行\n第二行\n"]);
    }
}
