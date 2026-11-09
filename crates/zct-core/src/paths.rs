//! 資料根目錄解析（架構文件第 5 節）
//!
//! 優先順序：環境變數 `ZCT_HOME` → 指標檔 `location.toml` 的 `data_root` → 指標檔所在資料夾

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const ENV_HOME: &str = "ZCT_HOME";
pub const POINTER_FILE: &str = "location.toml";

#[derive(Debug, Deserialize)]
struct Pointer {
    data_root: PathBuf,
}

/// 指標檔所在的固定資料夾：Windows `%APPDATA%\zct`、macOS `~/Library/Application Support/zct`
pub fn pointer_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("zct"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|p| PathBuf::from(p).join("Library/Application Support/zct"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config/zct"))
    }
}

/// 純函式版本，方便測試
pub fn resolve(env_home: Option<PathBuf>, pointer_dir: &Path) -> PathBuf {
    if let Some(home) = env_home.filter(|p| !p.as_os_str().is_empty()) {
        return home;
    }
    let pointer = pointer_dir.join(POINTER_FILE);
    // 用記事本或 PowerShell 5.1 寫的檔案開頭可能有 BOM，toml 不接受
    if let Ok(text) = std::fs::read_to_string(&pointer)
        && let Ok(p) = toml::from_str::<Pointer>(text.trim_start_matches('\u{feff}'))
    {
        return if p.data_root.is_absolute() {
            p.data_root
        } else {
            pointer_dir.join(p.data_root)
        };
    }
    pointer_dir.to_path_buf()
}

/// 資料根目錄內各檔案的位置
#[derive(Debug, Clone)]
pub struct DataRoot {
    root: PathBuf,
}

impl DataRoot {
    pub fn new(root: impl Into<PathBuf>) -> DataRoot {
        DataRoot { root: root.into() }
    }

    pub fn discover() -> Option<DataRoot> {
        let dir = pointer_dir()?;
        Some(DataRoot::new(resolve(std::env::var_os(ENV_HOME).map(PathBuf::from), &dir)))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// 不同步的單機資料
    pub fn local(&self) -> PathBuf {
        self.root.join("local")
    }

    /// 使用者詞庫、學習紀錄、選字統計
    pub fn learning(&self) -> PathBuf {
        self.local().join("learning")
    }

    /// 每日備份
    pub fn backup(&self) -> PathBuf {
        self.local().join("backup")
    }

    /// M2 / M3 版本的學習資料位置（啟動時遷移）
    pub fn legacy_chewing_dir(&self) -> PathBuf {
        self.local().join("chewing").join("v4")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_with_bom() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(POINTER_FILE),
            "\u{feff}data_root = \"D:\\\\My Data\\\\學習資料\"\r\n",
        )
        .unwrap();
        assert_eq!(resolve(None, dir.path()), PathBuf::from("D:\\My Data\\學習資料"));
    }

    #[test]
    fn env_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(POINTER_FILE), "data_root = 'X:/other'").unwrap();
        assert_eq!(
            resolve(Some("D:/zct".into()), dir.path()),
            PathBuf::from("D:/zct")
        );
    }

    #[test]
    fn pointer_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        std::fs::write(
            dir.path().join(POINTER_FILE),
            format!("data_root = '{}'", target.display()),
        )
        .unwrap();
        assert_eq!(resolve(None, dir.path()), target);
    }

    #[test]
    fn pointer_relative_path_is_relative_to_pointer_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(POINTER_FILE), "data_root = 'data'").unwrap();
        assert_eq!(resolve(None, dir.path()), dir.path().join("data"));
    }

    #[test]
    fn no_pointer_falls_back_to_pointer_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve(None, dir.path()), dir.path());
    }

    #[test]
    fn empty_env_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve(Some(PathBuf::new()), dir.path()), dir.path());
    }
}
