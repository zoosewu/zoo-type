//! 「關於」頁的「作者的其他專案」：讀 apex-code 的專案清單 API（兩個平台共用）
//!
//! 規範：https://github.com/zoosewu/apex-code/blob/main/docs/API.md 。不認得的欄位或值一律略過；
//! 清單與封面圖快取一天（GitHub Pages 本身約 10 分鐘）。讀取失敗時整個區塊不顯示。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const API: &str = "https://zoosewu.github.io/apex-code/api/v1/projects.json";
/// 自己在清單裡的 id（排除）
const SELF_ID: &str = "zoo-type";
const CACHE_FOR: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_LIST: u64 = 2 * 1024 * 1024;
const MAX_IMAGE: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Project {
    pub name: String,
    /// 一句話介紹（繁體中文，沒有時用英文）
    pub tagline: String,
    /// 點了開啟的網址（repo）
    pub url: String,
    /// 封面圖（下載到快取資料夾）；沒有截圖或下載失敗時是 None
    pub cover: Option<PathBuf>,
}

/// 讀清單與封面（會連網，請在背景執行緒呼叫）。`cache`：資料資料夾的 `local/cache/projects`
pub fn fetch(cache: &Path) -> Result<Vec<Project>, String> {
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let list = cache.join("projects.json");
    let json = match fresh(&list) {
        Some(text) => text,
        None => {
            let bytes = crate::net::get(API, MAX_LIST)?;
            let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            let _ = std::fs::write(&list, &text);
            text
        }
    };
    let mut projects = parse(&json)?;
    for (project, cover_url) in &mut projects {
        project.cover = cover_url
            .as_deref()
            .and_then(|url| cover(cache, &project.name, url));
    }
    Ok(projects.into_iter().map(|(p, _)| p).collect())
}

/// 快取還在一天內就用快取
fn fresh(path: &Path) -> Option<String> {
    let age = std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())?;
    (age < CACHE_FOR)
        .then(|| std::fs::read_to_string(path).ok())
        .flatten()
}

/// 解析清單：（專案, 封面圖網址）；排除自己，順序照清單
fn parse(json: &str) -> Result<Vec<(Project, Option<String>)>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let projects = value
        .get("projects")
        .and_then(|p| p.as_array())
        .ok_or("清單格式不對")?;
    let text = |v: &serde_json::Value| -> Option<String> {
        v.get("zh-TW")
            .or_else(|| v.get("en"))
            .and_then(|t| t.as_str())
            .map(str::to_owned)
    };
    Ok(projects
        .iter()
        .filter(|p| p.get("id").and_then(|i| i.as_str()) != Some(SELF_ID))
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_owned();
            let url = p.get("links")?.get("repo")?.as_str()?.to_owned();
            let tagline = p.get("tagline").and_then(text).unwrap_or_default();
            let cover = p
                .get("screenshots")
                .and_then(|s| s.as_array())
                .and_then(|s| s.first())
                .and_then(|s| s.get("src"))
                .and_then(text)
                .filter(|u| u.starts_with("https://"));
            Some((
                Project {
                    name,
                    tagline,
                    url,
                    cover: None,
                },
                cover,
            ))
        })
        .collect())
}

/// 封面圖：快取一天；檔名用專案名稱，副檔名照網址
fn cover(cache: &Path, name: &str, url: &str) -> Option<PathBuf> {
    let ext = url
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.'))
        .map(|(_, e)| e.split(['?', '#']).next().unwrap_or(e).to_ascii_lowercase())
        .filter(|e| matches!(e.as_str(), "png" | "jpg" | "jpeg"))?;
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = cache.join(format!("{safe}.{ext}"));
    if fresh_file(&path) {
        return Some(path);
    }
    let bytes = crate::net::get(url, MAX_IMAGE).ok()?;
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

fn fresh_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < CACHE_FOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_skips_self_and_unknown_fields() {
        let json = r#"{
          "apiVersion": "1.3.0", "somethingNew": true,
          "projects": [
            {"id": "zoo-type", "name": "動物園輸入法", "links": {"repo": "https://github.com/zoosewu/zoo-type"}},
            {"id": "zing-explore", "name": "zing-explore", "status": "brand-new-status",
             "tagline": {"zh-TW": "檔案總管", "en": "File explorer"},
             "links": {"repo": "https://github.com/zoosewu/zing-explore"},
             "screenshots": [{"src": {"zh-TW": "https://example.com/a.png", "en": "https://example.com/a.png"}}]},
            {"id": "no-repo", "name": "x", "links": {}},
            {"id": "en-only", "name": "en-only", "tagline": {"en": "English"},
             "links": {"repo": "https://github.com/zoosewu/en-only"}, "screenshots": []}
          ]
        }"#;
        let list = parse(json).unwrap();
        let names: Vec<_> = list.iter().map(|(p, _)| p.name.as_str()).collect();
        assert_eq!(names, ["zing-explore", "en-only"]);
        assert_eq!(list[0].0.tagline, "檔案總管");
        assert_eq!(list[0].1.as_deref(), Some("https://example.com/a.png"));
        assert_eq!(list[1].0.tagline, "English");
        assert_eq!(list[1].1, None);
        assert!(parse("{}").is_err());
    }
}
