//! 自動更新（只有正式版）：兩個平台共用「查、下載、驗證、排程」，平台只負責安裝
//!
//! - 開機 5 分鐘後、之後每天查 GitHub Releases 的最新正式版（設定 `[update] auto_check`）；設定畫面可以立即檢查
//! - 檢查只看版號；使用者按「更新」才下載這個平台的檔案與 `.sig` 到固定位置，驗證簽章與版號（不能降版），再交給平台安裝
//! - 只接受官方 Release 的下載網址（`https://github.com/zoosewu/zoo-type/releases/download/<tag>/<檔名>`）
//!
//! 只連到 GitHub，不送出任何使用者資料。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zct_core::menu::UpdateState;
use zct_update::{PUBLIC_KEY, Version, verify};

use crate::Host;

const LATEST: &str = "https://api.github.com/repos/zoosewu/zoo-type/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/zoosewu/zoo-type/releases/download/";
const FIRST_CHECK: Duration = Duration::from_secs(5 * 60);
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_RELEASE_JSON: u64 = 2 * 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_SIG: u64 = 4096;

/// 這個平台要下載的檔案
pub struct Target {
    /// 下載後放的資料夾
    pub dir: PathBuf,
    /// 下載後的固定檔名（Windows 以 SYSTEM 執行的 updater 從固定位置取）；簽章是「檔名.sig」
    pub file: String,
    /// Release 上的檔名
    pub asset: fn(&Version) -> String,
}

impl Target {
    /// Windows：`%ProgramData%\ZooType\update\setup.exe`
    pub fn windows() -> Option<Target> {
        Some(Target {
            dir: zct_update::inbox()?,
            file: zct_update::INBOX_SETUP.into(),
            asset: |v| format!("zoo-type-{v}-windows-x64-setup.exe"),
        })
    }

    /// macOS：資料資料夾的 `local/update/update.zip`
    pub fn macos(data: &zct_core::paths::DataRoot) -> Target {
        Target {
            dir: data.local().join("update"),
            file: "update.zip".into(),
            asset: |v| format!("zoo-type-{v}-macos-{}.zip", mac_arch()),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(&self.file)
    }

    fn sig(&self) -> PathBuf {
        self.dir.join(format!("{}.sig", self.file))
    }
}

pub fn mac_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

/// 只有使用者平常用的正式版管更新：開發版、或用 `ZCT_HOME` 另開的（測試用）都不管
pub fn manages_updates() -> bool {
    zct_core::channel::CURRENT.id == zct_core::channel::OFFICIAL.id
        && zct_core::paths::instance_suffix().is_empty()
}

/// 設定畫面與輸入法選單顯示的狀態
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// 比目前新、等使用者按「更新」的版本（還沒下載，或已下載驗證過）
    pub available: Option<String>,
    pub checking: bool,
    /// 正在下載新版：已下載的百分比
    pub progress: Option<u8>,
    /// 開始安裝的時間（安裝程式在背景跑，沒有回報）
    pub installing: Option<Instant>,
    /// 上次檢查的結果：已是最新版（`Ok`）或失敗原因
    pub last: Option<Result<(), String>>,
    pub message: String,
}

/// 背景安裝超過這麼久還沒換版，當作失敗：選單恢復「更新到 vX」可以再按
const INSTALL_TIMEOUT: Duration = Duration::from_secs(3 * 60);

impl Status {
    /// 選單、圖示提示與設定畫面要顯示的步驟
    pub fn state(&self) -> UpdateState {
        let installing = self
            .installing
            .is_some_and(|t| t.elapsed() < INSTALL_TIMEOUT);
        match (&self.available, self.progress) {
            (Some(v), _) if installing => UpdateState::Installing(v.clone()),
            (_, Some(p)) => UpdateState::Downloading(p),
            _ if self.checking => UpdateState::Checking,
            (Some(v), _) => UpdateState::Ready(v.clone()),
            (None, None) => match &self.last {
                Some(Ok(())) => UpdateState::Latest,
                Some(Err(e)) => UpdateState::Failed(e.clone()),
                None => UpdateState::Idle,
            },
        }
    }
}

/// 檢查時找到的新版：按「更新」才下載
#[derive(Clone)]
struct Found {
    version: Version,
    file_url: String,
    sig_url: String,
}

pub struct Updater {
    target: Target,
    status: Mutex<Status>,
    found: Mutex<Option<Found>>,
    /// 每天的檢查與手動檢查可能同時發生：一次只查一個，免得同時寫下載資料夾
    running: Mutex<()>,
}

impl Updater {
    /// 建立時先看下載資料夾：還沒裝的新版直接提示，已經裝過（或更舊）的檔案清掉
    pub fn new(target: Target) -> Arc<Updater> {
        let u = Updater {
            target,
            status: Mutex::new(Status {
                message: "尚未檢查更新".into(),
                ..Default::default()
            }),
            found: Mutex::new(None),
            running: Mutex::new(()),
        };
        match u.pending() {
            Some(v) => u.set(|s| {
                s.available = Some(v.to_string());
                s.message = format!("可以更新到 v{v}");
            }),
            None => {
                let _ = std::fs::remove_file(u.target.path());
                let _ = std::fs::remove_file(u.target.sig());
            }
        }
        Arc::new(u)
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 平台開始（或放棄）安裝新版
    pub fn set_installing(&self, on: bool) {
        self.set(|s| s.installing = on.then(Instant::now));
    }

    fn set(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()));
    }

    /// 下載資料夾裡簽章正確、而且比目前新的版本
    fn pending(&self) -> Option<Version> {
        verified(&self.target.path(), &self.target.sig())
    }

    /// 要安裝的檔案：重新驗證過的（版號, 路徑）
    pub fn pending_file(&self) -> Option<(String, PathBuf)> {
        self.pending().map(|v| (v.to_string(), self.target.path()))
    }

    /// 立刻查一次（只看版號，不下載）；有新版回傳版號
    pub fn check(&self) -> Result<Option<String>, String> {
        let _running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        self.set(|s| {
            s.checking = true;
            s.progress = None;
            s.message = "檢查更新中…".into();
        });
        let result = self.check_inner().map(|v| v.map(|v| v.to_string()));
        self.set(|s| {
            s.checking = false;
            s.progress = None;
            s.last = Some(result.as_ref().map(|_| ()).map_err(Clone::clone));
            match &result {
                Ok(Some(v)) => {
                    s.available = Some(v.clone());
                    s.message = format!("可以更新到 v{v}");
                }
                Ok(None) => {
                    s.available = None;
                    s.message = "目前已是最新版".into();
                }
                Err(e) => s.message = format!("檢查更新失敗：{e}"),
            }
        });
        match &result {
            Ok(Some(v)) => log::info!("有新版：{v}"),
            Ok(None) => log::info!("檢查更新：已是最新版"),
            Err(e) => log::warn!("檢查更新失敗：{e}"),
        }
        result
    }

    fn check_inner(&self) -> Result<Option<Version>, String> {
        let release: Release = serde_json::from_slice(&get(LATEST, MAX_RELEASE_JSON)?)
            .map_err(|e| format!("更新資訊格式錯誤：{e}"))?;
        if release.draft || release.prerelease {
            return Ok(None);
        }
        let version = Version::parse(&release.tag_name).ok_or("版號格式不對")?;
        if !version.is_newer_than(&Version::current()) {
            return Ok(None);
        }
        let name = (self.target.asset)(&version);
        let found = Found {
            file_url: asset_url(&release, &name)?,
            sig_url: asset_url(&release, &format!("{name}.sig"))?,
            version: version.clone(),
        };
        *self.found.lock().unwrap_or_else(|e| e.into_inner()) = Some(found);
        Ok(Some(version))
    }

    /// 使用者按「更新」：下載（已下載驗證過就略過）並驗證，回傳要安裝的（版號, 檔案）。會等下載完
    pub fn prepare(&self) -> Result<(String, PathBuf), String> {
        let _running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        let found = self.found.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let result = match (self.pending(), found) {
            (Some(v), f) if f.as_ref().is_none_or(|f| f.version == v) => Ok(()),
            (_, Some(f)) => self.download(&f),
            (_, None) => Err("請先檢查更新".into()),
        };
        self.set(|s| s.progress = None);
        match result.and_then(|()| self.pending_file().ok_or("下載的檔案簽章不符".to_string()))
        {
            Ok(file) => Ok(file),
            Err(e) => {
                self.failed(&e);
                Err(e)
            }
        }
    }

    fn download(&self, f: &Found) -> Result<(), String> {
        let version = &f.version;
        std::fs::create_dir_all(&self.target.dir)
            .map_err(|e| format!("無法建立 {}：{e}", self.target.dir.display()))?;
        self.set(|s| s.progress = Some(0));
        download(&f.file_url, &self.target.path(), MAX_FILE, |done, total| {
            self.set(|s| {
                s.progress = Some(total.map_or(0, |t| percent(done, t)));
                s.message = match total {
                    Some(t) => format!("下載 v{version}：{} / {}", mb(done), mb(t)),
                    None => format!("下載 v{version}：{}", mb(done)),
                };
            })
        })?;
        download(&f.sig_url, &self.target.sig(), MAX_SIG, |_, _| {})?;
        self.set(|s| s.message = format!("驗證 v{version}…"));
        match self.pending() {
            Some(v) if v == *version => Ok(()),
            _ => Err("下載的檔案簽章不符".into()),
        }
    }

    /// 更新失敗：顯示原因，可以再按一次
    pub fn failed(&self, reason: &str) {
        log::warn!("更新失敗：{reason}");
        self.set(|s| {
            s.installing = None;
            s.progress = None;
            s.message = format!("更新失敗：{reason}");
        });
    }

    /// 在背景下載驗證，再交給平台安裝（`install` 收到版號與檔案）
    pub fn install_in_background(
        self: &Arc<Self>,
        install: impl FnOnce(&str, &Path) -> Result<(), String> + Send + 'static,
    ) {
        let me = self.clone();
        me.set(|s| s.progress = Some(0));
        let spawned = std::thread::Builder::new()
            .name("update-install".into())
            .spawn(move || {
                let Ok((version, file)) = me.prepare() else {
                    return;
                };
                // 安裝程式在背景跑、沒有回報：先顯示「正在安裝」
                me.set_installing(true);
                if let Err(e) = install(&version, &file) {
                    me.failed(&e);
                }
            });
        if let Err(e) = spawned {
            self.failed(&e.to_string());
        }
    }

    /// 背景排程：開機 5 分鐘後、之後每天，設定開著才查
    pub fn spawn_schedule(self: &Arc<Self>, host: Arc<Host>) -> std::io::Result<()> {
        let me = self.clone();
        std::thread::Builder::new()
            .name("update".into())
            .spawn(move || {
                std::thread::sleep(FIRST_CHECK);
                loop {
                    if host.settings().current().0.update.auto_check {
                        let _ = me.check();
                    }
                    std::thread::sleep(INTERVAL);
                }
            })
            .map(|_| ())
    }

    /// 在背景立刻查一次（設定畫面的「檢查更新」）；結果看 [`Updater::status`]
    pub fn check_in_background(self: &Arc<Self>) {
        let me = self.clone();
        me.set(|s| s.checking = true);
        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = me.check();
            });
        if let Err(e) = spawned {
            self.set(|s| {
                s.checking = false;
                s.message = format!("檢查更新失敗：{e}");
            });
        }
    }
}

/// 簽章正確、而且比目前新的版本
pub fn verified(file: &Path, sig: &Path) -> Option<Version> {
    let bytes = std::fs::read(file).ok()?;
    let sig = std::fs::read_to_string(sig).ok()?;
    verify(&PUBLIC_KEY, &bytes, &sig)
        .ok()
        .filter(|v| v.is_newer_than(&Version::current()))
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Release 上這個檔案的下載網址；不是官方 Release 的網址一律拒絕
fn asset_url(release: &Release, name: &str) -> Result<String, String> {
    let url = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.browser_download_url.clone())
        .ok_or(format!("Release 沒有 {name}"))?;
    official_url(&url, &release.tag_name, name)?;
    Ok(url)
}

pub fn official_url(url: &str, tag: &str, name: &str) -> Result<(), String> {
    if url == format!("{DOWNLOAD_PREFIX}{tag}/{name}") {
        Ok(())
    } else {
        Err("更新來源不符官方 Release".into())
    }
}

/// 慢的網路也能下載完：不限總時間，連線或任何一次讀取卡住太久才放棄
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .build()
        .into()
}

fn request(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    agent()
        .get(url)
        .header("User-Agent", "zoo-type")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())
}

fn get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    request(url)?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// 邊下載邊回報（已下載, 總大小）
fn download(
    url: &str,
    to: &Path,
    limit: u64,
    progress: impl Fn(u64, Option<u64>),
) -> Result<(), String> {
    use std::io::{Read, Write};
    let mut response = request(url)?;
    let total = response.body().content_length();
    let mut reader = response.body_mut().with_config().limit(limit).reader();
    let tmp = to.with_extension("download");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = vec![0; 64 * 1024];
    let mut done = 0;
    progress(0, total);
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&tmp, to).map_err(|e| e.to_string())
}

fn percent(done: u64, total: u64) -> u8 {
    (done.saturating_mul(100) / total.max(1)).min(100) as u8
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_official_release_urls_are_accepted() {
        let name = "zoo-type-0.1.5-macos-arm64.zip";
        let ok = format!("{DOWNLOAD_PREFIX}v0.1.5/{name}");
        assert!(official_url(&ok, "v0.1.5", name).is_ok());
        for bad in [
            "https://example.com/installer.zip".to_string(),
            format!("http://github.com/zoosewu/zoo-type/releases/download/v0.1.5/{name}"),
            format!("{DOWNLOAD_PREFIX}v0.1.4/{name}"),
            format!("{ok}?x=1"),
            format!(
                "https://github.com.evil.example/zoosewu/zoo-type/releases/download/v0.1.5/{name}"
            ),
        ] {
            assert!(official_url(&bad, "v0.1.5", name).is_err(), "{bad}");
        }
    }

    #[test]
    fn unsigned_or_old_downloads_are_not_offered() {
        let dir = tempfile::tempdir().unwrap();
        let target = Target {
            dir: dir.path().into(),
            file: "update.zip".into(),
            asset: |v| format!("zoo-type-{v}.zip"),
        };
        std::fs::write(target.path(), b"fake").unwrap();
        std::fs::write(target.sig(), "zoo-type-update-v1\n99.0.0\n00").unwrap();
        let u = Updater::new(target);
        assert_eq!(u.status().available, None);
        assert!(u.pending_file().is_none());
        assert!(
            !dir.path().join("update.zip").exists(),
            "驗證不過的檔案清掉"
        );
    }

    #[test]
    fn install_downloads_only_after_a_check_found_a_version() {
        let dir = tempfile::tempdir().unwrap();
        let u = Updater::new(Target {
            dir: dir.path().join("update"),
            file: "update.zip".into(),
            asset: |v| format!("zoo-type-{v}.zip"),
        });
        assert_eq!(u.prepare().unwrap_err(), "請先檢查更新");
        assert!(
            !dir.path().join("update").exists(),
            "沒按更新不建立下載資料夾"
        );
        let s = u.status();
        assert_eq!((s.progress, s.installing), (None, None), "失敗後可以再按");
        assert!(s.message.starts_with("更新失敗"));
    }

    #[test]
    fn status_reads_as_one_update_step() {
        let mut s = Status::default();
        assert_eq!(s.state(), UpdateState::Idle);
        s.last = Some(Ok(()));
        assert_eq!(s.state(), UpdateState::Latest);
        s.last = Some(Err("離線".into()));
        assert_eq!(s.state(), UpdateState::Failed("離線".into()));
        s.checking = true;
        assert_eq!(s.state(), UpdateState::Checking);
        s.progress = Some(30);
        assert_eq!(s.state(), UpdateState::Downloading(30));
        s = Status {
            available: Some("0.1.6".into()),
            ..Default::default()
        };
        assert_eq!(s.state(), UpdateState::Ready("0.1.6".into()));
        s.installing = Some(Instant::now());
        assert_eq!(s.state(), UpdateState::Installing("0.1.6".into()));
        // 安裝程式沒換版就停了：可以再按一次
        s.installing = Instant::now().checked_sub(INSTALL_TIMEOUT);
        assert_eq!(s.state(), UpdateState::Ready("0.1.6".into()));
    }

    #[test]
    fn download_progress_reads_as_percent_and_megabytes() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(13, 26), 50);
        assert_eq!(percent(30, 26), 100);
        assert_eq!(mb(26 * 1_048_576), "26.0 MB");
    }

    #[test]
    fn versions_never_go_backwards() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.1.5").is_newer_than(&v("0.1.4")));
        assert!(!v("0.1.3").is_newer_than(&v("0.1.4")));
        assert!(!v("0.1.5-rc.1").is_newer_than(&v("0.1.5")));
    }
}
