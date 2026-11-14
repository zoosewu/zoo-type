//! 自動更新（只有正式版，設定 `[update] auto_check`）
//!
//! - 開機後幾分鐘、之後每天查 GitHub Releases 的最新正式版（不含 pre-release）
//! - 比目前新就下載安裝檔與 `.sig` 到 `%ProgramData%\ZooType\update`，先在這裡驗證簽章（只是避免提示壞掉的檔案；
//!   真正的把關在以 SYSTEM 執行的 zoo-type-updater）
//! - 輸入法圖示的選單出現「更新到 vX」；按下後啟動排程工作「ZooType Updater」，不跳 UAC
//!
//! 只連到 GitHub，不送出任何使用者資料。

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zct_update::{INBOX_SETUP, INBOX_SIG, PUBLIC_KEY, TASK_NAME, Version, verify};

use crate::settings::Settings;

const LATEST: &str = "https://api.github.com/repos/zoosewu/zoo-type/releases/latest";
const FIRST_CHECK: Duration = Duration::from_secs(5 * 60);
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// 安裝檔大小上限（目前約 16 MB）
const MAX_DOWNLOAD: u64 = 128 * 1024 * 1024;

/// 已下載、驗證過，等使用者按「更新」的版本
pub type Available = Arc<Mutex<Option<String>>>;

pub fn spawn(settings: Arc<Settings>, available: Available) -> std::io::Result<()> {
    if zct_core::channel::CURRENT.id != zct_core::channel::OFFICIAL.id {
        return Ok(());
    }
    std::thread::Builder::new()
        .name("update".into())
        .spawn(move || {
            clean_inbox(&available);
            std::thread::sleep(FIRST_CHECK);
            loop {
                if settings.current().0.update.auto_check {
                    match check() {
                        Ok(Some(v)) => {
                            log::info!("有新版：{v}");
                            *available.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(v.to_string());
                        }
                        Ok(None) => log::info!("檢查更新：已是最新版"),
                        Err(e) => log::warn!("檢查更新失敗：{e}"),
                    }
                }
                std::thread::sleep(INTERVAL);
            }
        })
        .map(|_| ())
}

/// 上次下載、已經裝過（或更舊）的檔案清掉；還沒裝的新版直接提示，不必等下次檢查
fn clean_inbox(available: &Available) {
    let Some(inbox) = zct_update::inbox() else {
        return;
    };
    match pending(&inbox) {
        Some(v) => *available.lock().unwrap_or_else(|e| e.into_inner()) = Some(v.to_string()),
        None => {
            for name in [INBOX_SETUP, INBOX_SIG] {
                let _ = std::fs::remove_file(inbox.join(name));
            }
        }
    }
}

/// inbox 裡簽章正確、而且比目前新的版本
fn pending(inbox: &Path) -> Option<Version> {
    let bytes = std::fs::read(inbox.join(INBOX_SETUP)).ok()?;
    let sig = std::fs::read_to_string(inbox.join(INBOX_SIG)).ok()?;
    verify(&PUBLIC_KEY, &bytes, &sig)
        .ok()
        .filter(|v| v.is_newer_than(&Version::current()))
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into()
}

/// 查最新版；比目前新就下載並驗證，回傳版號
fn check() -> Result<Option<Version>, String> {
    let release: Release = agent()
        .get(LATEST)
        .header("User-Agent", "zoo-type")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;
    let version = Version::parse(&release.tag_name).ok_or("版號格式不對")?;
    if !version.is_newer_than(&Version::current()) {
        return Ok(None);
    }
    let inbox = zct_update::inbox().ok_or("找不到 ProgramData")?;
    if pending(&inbox).as_ref() == Some(&version) {
        return Ok(Some(version));
    }
    let setup_name = format!("zoo-type-{version}-windows-x64-setup.exe");
    let url = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
            .ok_or(format!("Release 沒有 {name}"))
    };
    let (setup_url, sig_url) = (url(&setup_name)?, url(&format!("{setup_name}.sig"))?);
    std::fs::create_dir_all(&inbox).map_err(|e| format!("無法建立 {}：{e}", inbox.display()))?;
    download(&setup_url, &inbox.join(INBOX_SETUP))?;
    download(&sig_url, &inbox.join(INBOX_SIG))?;
    match pending(&inbox) {
        Some(v) if v == version => Ok(Some(version)),
        _ => Err("下載的安裝檔簽章不符，不提示更新".into()),
    }
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let bytes = agent()
        .get(url)
        .header("User-Agent", "zoo-type")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    let tmp = to.with_extension("download");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, to).map_err(|e| e.to_string())
}

/// 使用者按「更新到 vX」：啟動以 SYSTEM 執行的排程工作（安裝時建立，一般使用者可以啟動）
pub fn install() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("schtasks.exe")
        .args(["/Run", "/TN", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("無法執行 schtasks：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "無法啟動更新工作（{}）",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// 自動更新以 SYSTEM 執行安裝程式，沒辦法改使用者的「登入時啟動」：服務程序啟動時改成目前版本的路徑
pub fn fix_autostart() {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
    };
    use windows::core::HSTRING;
    if zct_core::channel::CURRENT.id != zct_core::channel::OFFICIAL.id {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let key = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = HSTRING::from(zct_core::channel::CURRENT.dir);
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    // 使用者關掉了登入時啟動（值不存在）就不加回來
    if found.is_err() {
        return;
    }
    let current = String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]);
    let want = format!("\"{}\"", exe.display());
    if current == want {
        return;
    }
    let wide: Vec<u16> = want.encode_utf16().chain(std::iter::once(0)).collect();
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            REG_SZ.0,
            Some(wide.as_ptr().cast()),
            (wide.len() * 2) as u32,
        )
    };
    if r.is_ok() {
        log::info!("登入時啟動改成：{want}");
    }
}
