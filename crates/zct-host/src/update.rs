//! 自動更新（只有正式版）：依 zoo-release 的 docs/UPDATE.md，兩個平台共用「檢查、下載、驗證、排程」，平台只負責安裝
//!
//! - 檢查：讀 `releases/latest/download/latest.json`（不經 GitHub API，沒有次數限制），只送出一個 GET、不帶使用者資料
//! - 時機：啟動 1 分鐘後（距上次成功檢查不到 24 小時就沿用上次的結果）、之後每 24 小時；手動檢查不受限。
//!   自動檢查只在設定 `[update] auto_check` 開著時進行，失敗不顯示
//! - 提示：主版或次版較新 → 輸入法選單提示；只有修補版較新 → 只在「關於」頁顯示
//! - 使用者按「更新」才下載：只接受本 repo 的 Release 網址，驗證簽章（zoo-release 的共用金鑰）與 sha256，不能降版
//!
//! 狀態（上次檢查時間、看到的最新版本）存在資料資料夾的 `local/update-state.json`。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zct_core::menu::UpdateState;
use zoo_update::{Availability, Manifest, Version, current_platform, keys, manifest_url};

use crate::Host;
use crate::net::{get, request};

/// GitHub 的 `擁有者/repo`
pub const REPO: &str = "zoosewu/zoo-type";
const FIRST_CHECK: Duration = Duration::from_secs(60);
/// 多久看一次「該檢查了沒」（間隔 24 小時由 `zoo_update::UpdateState::check_due` 判斷）
const TICK: Duration = Duration::from_secs(60 * 60);
const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024 * 1024;
/// 背景安裝超過這麼久還沒換版，當作失敗：可以再按一次
const INSTALL_TIMEOUT: Duration = Duration::from_secs(3 * 60);

/// 下載的更新檔放在哪裡
pub struct Target {
    pub dir: PathBuf,
    /// 固定檔名；旁邊的「檔名.sig」是 latest.json 裡的簽章（Windows 以 SYSTEM 執行的 updater 再驗證一次）
    pub file: String,
}

impl Target {
    /// Windows：`%ProgramData%\ZooType\update\setup.exe`
    pub fn windows() -> Option<Target> {
        Some(Target {
            dir: zct_update::inbox()?,
            file: zct_update::INBOX_SETUP.into(),
        })
    }

    /// macOS：資料資料夾的 `local/update/update.zip`
    pub fn macos(data: &zct_core::paths::DataRoot) -> Target {
        Target {
            dir: data.local().join("update"),
            file: "update.zip".into(),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(&self.file)
    }

    fn sig(&self) -> PathBuf {
        self.dir.join(format!("{}.sig", self.file))
    }

    fn clear(&self) {
        let _ = std::fs::remove_file(self.path());
        let _ = std::fs::remove_file(self.sig());
    }
}

/// 只有使用者平常用的正式版管更新：開發版、或用 `ZCT_HOME` 另開的（測試用）都不管
pub fn manages_updates() -> bool {
    zct_core::channel::CURRENT.id == zct_core::channel::OFFICIAL.id
        && zct_core::paths::instance_suffix().is_empty()
}

/// 這個程式的版號；預先發行版（`-rc.1`）不參與更新
fn current() -> Option<Version> {
    Version::parse(env!("CARGO_PKG_VERSION"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// 正在進行的事（只在這次執行期間）
#[derive(Debug, Clone, Default)]
struct Busy {
    checking: bool,
    /// 手動檢查失敗：「關於」頁顯示「無法檢查更新」
    check_failed: bool,
    /// 下載中：（版號, 百分比）
    download: Option<(String, u8)>,
    /// 開始安裝的時間（安裝程式在背景跑，沒有回報）
    installing: Option<(String, Instant)>,
    /// 下載或驗證失敗的原因
    failed: Option<String>,
}

pub struct Updater {
    target: Target,
    state_file: PathBuf,
    saved: Mutex<zoo_update::UpdateState>,
    busy: Mutex<Busy>,
    /// 最近一次讀到的 latest.json（按「更新」時沒有就再讀一次）
    manifest: Mutex<Option<Manifest>>,
    /// 檢查與下載一次一個，免得同時寫下載資料夾
    running: Mutex<()>,
    cancel: AtomicBool,
}

impl Updater {
    /// `state_file`：資料資料夾的 `local/update-state.json`。上次下載到一半或沒裝的檔案清掉
    pub fn new(target: Target, state_file: PathBuf) -> Arc<Updater> {
        target.clear();
        let saved = std::fs::read(&state_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Arc::new(Updater {
            target,
            state_file,
            saved: Mutex::new(saved),
            busy: Mutex::new(Busy::default()),
            manifest: Mutex::new(None),
            running: Mutex::new(()),
            cancel: AtomicBool::new(false),
        })
    }

    fn busy(&self, f: impl FnOnce(&mut Busy)) {
        f(&mut self.busy.lock().unwrap_or_else(|e| e.into_inner()));
    }

    /// 選單、圖示提示與「關於」頁要顯示的步驟
    pub fn state(&self) -> UpdateState {
        let busy = self.busy.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let saved = self.saved.lock().unwrap_or_else(|e| e.into_inner()).clone();
        state_of(&busy, &saved, current(), Instant::now(), now_secs())
    }

    /// 立刻檢查一次（只讀 latest.json，不下載）；有新版回傳版號。
    /// `manual`：「關於」頁的「檢查更新」，失敗時顯示「無法檢查更新」；自動檢查失敗不顯示
    pub fn check(&self, manual: bool) -> Result<Option<String>, String> {
        let _running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        self.busy(|b| {
            b.checking = true;
            b.check_failed = false;
            b.failed = None;
        });
        let result = fetch_manifest();
        let found = match &result {
            Ok(manifest) => {
                let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
                saved.record_check(now_secs(), manifest);
                self.save(&saved);
                current().and_then(|c| saved.availability(c).version())
            }
            Err(_) => None,
        };
        let ok = result.is_ok();
        match result {
            Ok(m) => {
                *self.manifest.lock().unwrap_or_else(|e| e.into_inner()) = Some(m);
                match &found {
                    Some(v) => log::info!("有新版：{v}"),
                    None => log::info!("檢查更新：已是最新版"),
                }
            }
            Err(ref e) => log::warn!("檢查更新失敗：{e}"),
        }
        self.busy(|b| {
            b.checking = false;
            b.check_failed = manual && !ok;
        });
        if ok {
            Ok(found.map(|v| v.to_string()))
        } else {
            Err("無法檢查更新".into())
        }
    }

    fn save(&self, saved: &zoo_update::UpdateState) {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.state_file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = self.state_file.with_extension("tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(saved)?)?;
            std::fs::rename(&tmp, &self.state_file)
        };
        if let Err(e) = write() {
            log::warn!("無法儲存更新狀態：{e}");
        }
    }

    /// 使用者按「更新」：下載並驗證，回傳要安裝的（版號, 檔案）。會等下載完
    pub fn prepare(&self) -> Result<(String, PathBuf), String> {
        let _running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        self.cancel.store(false, Ordering::Relaxed);
        self.busy(|b| b.failed = None);
        let result = self.download();
        self.busy(|b| b.download = None);
        match result {
            Ok(done) => Ok(done),
            Err(Stop::Cancelled) => {
                self.target.clear();
                Err("已取消".into())
            }
            Err(Stop::Failed(reason)) => {
                self.target.clear();
                self.failed(&reason);
                Err(reason)
            }
        }
    }

    fn download(&self) -> Result<(String, PathBuf), Stop> {
        let cached = self
            .manifest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let manifest = match cached {
            Some(m) => m,
            None => fetch_manifest().map_err(|e| {
                log::warn!("讀取更新資訊失敗：{e}");
                Stop::Failed("無法連線".into())
            })?,
        };
        let version = manifest
            .version()
            .filter(|v| current().is_some_and(|c| *v > c))
            .ok_or_else(|| Stop::Failed("目前已是最新版".into()))?;
        let asset = manifest
            .asset(current_platform())
            .ok_or_else(|| Stop::Failed("這個平台沒有自動更新檔".into()))?;
        asset.check_url(REPO).map_err(|e| {
            log::warn!("{e}");
            Stop::Failed("更新來源不符官方 Release".into())
        })?;
        let v = version.to_string();
        std::fs::create_dir_all(&self.target.dir)
            .map_err(|e| Stop::Failed(format!("無法建立 {}：{e}", self.target.dir.display())))?;
        self.busy(|b| b.download = Some((v.clone(), 0)));
        let path = self.target.path();
        download(&asset.url, &path, MAX_FILE, &self.cancel, |done, total| {
            let percent = total.map_or(0, |t| percent(done, t));
            self.busy(|b| b.download = Some((v.clone(), percent)));
        })?;
        asset.verify_file(&path, keys::ALL).map_err(|e| {
            log::warn!("更新檔驗證失敗：{e}");
            Stop::Failed(match e {
                zoo_update::Error::Sha256 => "檔案不完整".into(),
                _ => "簽章驗證失敗".into(),
            })
        })?;
        // Windows 以 SYSTEM 執行的 updater 從固定位置取檔案與簽章，再驗證一次
        std::fs::write(self.target.sig(), &asset.signature)
            .map_err(|e| Stop::Failed(e.to_string()))?;
        Ok((v, path))
    }

    /// 取消下載
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// 平台開始（或放棄）安裝新版
    pub fn set_installing(&self, version: Option<&str>) {
        self.busy(|b| b.installing = version.map(|v| (v.to_owned(), Instant::now())));
    }

    /// 更新失敗：顯示原因與「手動下載」，可以再按一次
    pub fn failed(&self, reason: &str) {
        log::warn!("更新失敗：{reason}");
        self.busy(|b| {
            b.installing = None;
            b.download = None;
            b.failed = Some(reason.to_owned());
        });
    }

    /// 在背景下載驗證，再交給平台安裝（`install` 收到版號與檔案）
    pub fn install_in_background(
        self: &Arc<Self>,
        install: impl FnOnce(&str, &Path) -> Result<(), String> + Send + 'static,
    ) {
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("update-install".into())
            .spawn(move || {
                let Ok((version, file)) = me.prepare() else {
                    return;
                };
                // 安裝程式在背景跑、沒有回報：先顯示「正在安裝」
                me.set_installing(Some(&version));
                if let Err(e) = install(&version, &file) {
                    me.failed(&e);
                }
            });
        if let Err(e) = spawned {
            self.failed(&e.to_string());
        }
    }

    /// 背景排程：啟動 1 分鐘後、之後每小時看一次，距上次成功檢查滿 24 小時而且設定開著才查
    pub fn spawn_schedule(self: &Arc<Self>, host: Arc<Host>) -> std::io::Result<()> {
        let me = self.clone();
        std::thread::Builder::new()
            .name("update".into())
            .spawn(move || {
                std::thread::sleep(FIRST_CHECK);
                loop {
                    let due = {
                        let mut saved = me.saved.lock().unwrap_or_else(|e| e.into_inner());
                        saved.auto_check = host.settings().current().0.update.auto_check;
                        saved.check_due(now_secs())
                    };
                    if due {
                        let _ = me.check(false);
                    }
                    std::thread::sleep(TICK);
                }
            })
            .map(|_| ())
    }

    /// 在背景立刻手動檢查一次（「關於」頁、輸入法選單的「檢查更新」）；結果看 [`Updater::state`]
    pub fn check_in_background(self: &Arc<Self>) {
        let me = self.clone();
        me.busy(|b| b.checking = true);
        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = me.check(true);
            });
        if spawned.is_err() {
            self.busy(|b| {
                b.checking = false;
                b.check_failed = true;
            });
        }
    }
}

/// 顯示的步驟：進行中的事優先，其次是上次檢查的結果
fn state_of(
    busy: &Busy,
    saved: &zoo_update::UpdateState,
    current: Option<Version>,
    now: Instant,
    now_secs: u64,
) -> UpdateState {
    if let Some((v, since)) = &busy.installing
        && now.duration_since(*since) < INSTALL_TIMEOUT
    {
        return UpdateState::Installing(v.clone());
    }
    if let Some((version, percent)) = &busy.download {
        return UpdateState::Downloading {
            version: version.clone(),
            percent: *percent,
        };
    }
    if busy.checking {
        return UpdateState::Checking;
    }
    if let Some(reason) = &busy.failed {
        return UpdateState::Failed(reason.clone());
    }
    if busy.check_failed {
        return UpdateState::CheckFailed;
    }
    let Some(current) = current else {
        return UpdateState::Off;
    };
    match saved.availability(current) {
        Availability::Feature(v) | Availability::Patch(v) => UpdateState::Available {
            version: v.to_string(),
            feature: saved.should_show_banner(current),
        },
        Availability::UpToDate => match saved.last_check {
            Some(t) => UpdateState::Latest {
                checked: Some(checked_label(t, now_secs)),
            },
            None => UpdateState::Idle,
        },
    }
}

/// 「今天 14:31」「昨天 09:05」，更早的寫日期（當地時間、24 小時制）
fn checked_label(then: u64, now: u64) -> String {
    use chrono::{Local, TimeZone};
    let (Some(then), Some(now)) = (
        Local.timestamp_opt(then as i64, 0).single(),
        Local.timestamp_opt(now as i64, 0).single(),
    ) else {
        return String::new();
    };
    let days = (now.date_naive() - then.date_naive()).num_days();
    let time = then.format("%H:%M");
    match days {
        0 => format!("今天 {time}"),
        1 => format!("昨天 {time}"),
        _ => format!("{} {time}", then.format("%Y-%m-%d")),
    }
}

enum Stop {
    Cancelled,
    Failed(String),
}

fn fetch_manifest() -> Result<Manifest, String> {
    let bytes = get(&manifest_url(REPO), MAX_MANIFEST)?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    Manifest::parse(&text).map_err(|e| e.to_string())
}

/// 邊下載邊回報（已下載, 總大小）；`cancel` 設起來就停
fn download(
    url: &str,
    to: &Path,
    limit: u64,
    cancel: &AtomicBool,
    progress: impl Fn(u64, Option<u64>),
) -> Result<(), Stop> {
    use std::io::{Read, Write};
    let network = |e: String| {
        log::warn!("下載更新失敗：{e}");
        Stop::Failed("無法連線".into())
    };
    let local = |e: std::io::Error| Stop::Failed(e.to_string());
    let mut response = request(url).map_err(network)?;
    let total = response.body().content_length();
    let mut reader = response.body_mut().with_config().limit(limit).reader();
    let tmp = to.with_extension("download");
    let mut file = std::fs::File::create(&tmp).map_err(local)?;
    let mut buf = vec![0; 64 * 1024];
    let mut done = 0;
    progress(0, total);
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err(Stop::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| network(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(local)?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all().map_err(local)?;
    drop(file);
    std::fs::rename(&tmp, to).map_err(local)
}

fn percent(done: u64, total: u64) -> u8 {
    (done.saturating_mul(100) / total.max(1)).min(100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: Version = Version::new(0, 1, 10);

    fn saved(latest: Option<&str>, last_check: Option<u64>) -> zoo_update::UpdateState {
        zoo_update::UpdateState {
            latest: latest.map(Into::into),
            last_check,
            ..Default::default()
        }
    }

    fn at(busy: &Busy, saved: &zoo_update::UpdateState) -> UpdateState {
        state_of(busy, saved, Some(V), Instant::now(), 0)
    }

    #[test]
    fn feature_versions_reach_the_menu_patches_only_the_about_page() {
        let busy = Busy::default();
        assert_eq!(at(&busy, &saved(None, None)), UpdateState::Idle);
        assert_eq!(
            at(&busy, &saved(Some("0.2.0"), Some(1))),
            UpdateState::Available {
                version: "0.2.0".into(),
                feature: true
            }
        );
        assert_eq!(
            at(&busy, &saved(Some("0.1.11"), Some(1))),
            UpdateState::Available {
                version: "0.1.11".into(),
                feature: false
            }
        );
        let mut skipped = saved(Some("0.2.3"), Some(1));
        skipped.skip(Version::new(0, 2, 0));
        assert_eq!(
            at(&busy, &skipped),
            UpdateState::Available {
                version: "0.2.3".into(),
                feature: false
            },
            "略過的版本只在「關於」頁顯示"
        );
        assert!(matches!(
            at(&busy, &saved(Some("0.1.10"), Some(1))),
            UpdateState::Latest { checked: Some(_) }
        ));
        assert_eq!(
            state_of(
                &busy,
                &saved(Some("0.2.0"), Some(1)),
                None,
                Instant::now(),
                0
            ),
            UpdateState::Off,
            "預先發行版不參與更新"
        );
    }

    #[test]
    fn work_in_progress_comes_before_the_last_result() {
        let s = saved(Some("0.2.0"), Some(1));
        let mut busy = Busy {
            check_failed: true,
            ..Default::default()
        };
        assert_eq!(at(&busy, &s), UpdateState::CheckFailed);
        busy.failed = Some("無法連線".into());
        assert_eq!(at(&busy, &s), UpdateState::Failed("無法連線".into()));
        busy.checking = true;
        assert_eq!(at(&busy, &s), UpdateState::Checking);
        busy.download = Some(("0.2.0".into(), 68));
        assert_eq!(
            at(&busy, &s),
            UpdateState::Downloading {
                version: "0.2.0".into(),
                percent: 68
            }
        );
        busy.installing = Some(("0.2.0".into(), Instant::now()));
        assert_eq!(at(&busy, &s), UpdateState::Installing("0.2.0".into()));
        // 安裝程式沒換版就停了：可以再按一次
        busy.installing = Instant::now()
            .checked_sub(INSTALL_TIMEOUT)
            .map(|t| ("0.2.0".into(), t));
        assert!(matches!(at(&busy, &s), UpdateState::Downloading { .. }));
    }

    #[test]
    fn last_check_reads_as_today_or_yesterday() {
        use chrono::{Local, TimeZone};
        let noon = Local
            .with_ymd_and_hms(2026, 11, 28, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp() as u64;
        let morning = noon - 3 * 3600 + 31 * 60; // 09:31
        assert_eq!(checked_label(morning, noon), "今天 09:31");
        assert_eq!(checked_label(morning - 86_400, noon), "昨天 09:31");
        assert_eq!(
            checked_label(morning - 3 * 86_400, noon),
            "2026-11-25 09:31"
        );
    }

    #[test]
    fn state_is_kept_between_runs_and_leftovers_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let target = || Target {
            dir: dir.path().join("update"),
            file: "update.zip".into(),
        };
        std::fs::create_dir_all(dir.path().join("update")).unwrap();
        std::fs::write(dir.path().join("update/update.zip"), b"half").unwrap();
        let state_file = dir.path().join("update-state.json");
        std::fs::write(&state_file, r#"{"latest":"99.0.0","last_check":1}"#).unwrap();
        let u = Updater::new(target(), state_file);
        assert!(
            !dir.path().join("update/update.zip").exists(),
            "上次沒裝的檔案清掉"
        );
        // 預先發行版（測試 Release 的 0.0.0-test.1）不參與更新，不會提示
        if current().is_some() {
            assert!(matches!(
                u.state(),
                UpdateState::Available { version, feature: true } if version == "99.0.0"
            ));
        } else {
            assert!(!matches!(u.state(), UpdateState::Available { .. }));
        }
    }

    #[test]
    fn download_progress_reads_as_percent() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(13, 26), 50);
        assert_eq!(percent(30, 26), 100);
    }
}
