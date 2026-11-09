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

/// 資料夾名稱（改名前是 `zct`）
const DIR_NAME: &str = "ZooType";
const LEGACY_DIR_NAME: &str = "zct";

/// 使用者設定的上層資料夾：Windows `%APPDATA%`、macOS `~/Library/Application Support`
fn app_data() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config"))
    }
}

/// 指標檔所在的固定資料夾：Windows `%APPDATA%\ZooType`、macOS `~/Library/Application Support/ZooType`
pub fn pointer_dir() -> Option<PathBuf> {
    app_data().map(|p| p.join(DIR_NAME))
}

/// 改名前的資料夾（`%APPDATA%\zct`）搬到新名稱；回傳做了什麼（記在 log）
///
/// 服務程序與設定程式啟動時、找資料根目錄之前呼叫。新資料夾已存在就不動。
pub fn migrate_legacy() -> Option<String> {
    let base = app_data()?;
    match migrate_dir(&base.join(LEGACY_DIR_NAME), &base.join(DIR_NAME)) {
        Ok(done) => done,
        Err(e) => Some(format!("搬移舊資料夾失敗：{e}")),
    }
}

/// 先試改名（同一顆磁碟、一瞬間完成）；舊資料夾還被舊版程式使用而改不了名時，改為複製一份
pub fn migrate_dir(old: &Path, new: &Path) -> std::io::Result<Option<String>> {
    if new.exists() || !old.exists() {
        return Ok(None);
    }
    if std::fs::rename(old, new).is_ok() {
        return Ok(Some(format!("資料夾改名：{} → {}", old.display(), new.display())));
    }
    copy_dir(old, new)?;
    let _ = std::fs::write(
        old.join("已搬移到 ZooType.txt"),
        format!("這個資料夾的內容已複製到 {}，舊版程式都關閉後可以刪除這個資料夾。\r\n", new.display()),
    );
    Ok(Some(format!("資料夾複製：{} → {}（舊資料夾使用中）", old.display(), new.display())))
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
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
    fn migrate_renames_legacy_dir() {
        let base = tempfile::tempdir().unwrap();
        let (old, new) = (base.path().join("zct"), base.path().join("ZooType"));
        std::fs::create_dir_all(old.join("local").join("learning")).unwrap();
        std::fs::write(old.join("config.toml"), "x").unwrap();
        std::fs::write(old.join("local").join("learning").join("user_dict.csv"), "y").unwrap();
        assert!(migrate_dir(&old, &new).unwrap().is_some());
        assert!(!old.exists());
        assert_eq!(std::fs::read_to_string(new.join("local/learning/user_dict.csv")).unwrap(), "y");
        // 新資料夾已存在：不動
        std::fs::create_dir_all(&old).unwrap();
        assert!(migrate_dir(&old, &new).unwrap().is_none());
        assert!(old.exists());
    }

    #[test]
    fn copy_keeps_everything() {
        let base = tempfile::tempdir().unwrap();
        let (src, dst) = (base.path().join("a"), base.path().join("b"));
        std::fs::create_dir_all(src.join(".git").join("objects")).unwrap();
        std::fs::write(src.join(".git").join("HEAD"), "ref").unwrap();
        std::fs::write(src.join("location.toml"), "data_root = 'D:/x'").unwrap();
        copy_dir(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join(".git/HEAD")).unwrap(), "ref");
        assert!(dst.join("location.toml").exists());
    }

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
