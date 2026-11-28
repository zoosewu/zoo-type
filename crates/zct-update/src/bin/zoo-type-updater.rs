//! 以 SYSTEM 身分安裝更新（由排程工作「ZooType Updater」啟動，一般使用者可以觸發）
//!
//! 服務程序把安裝檔與 `.sig` 下載到 `%ProgramData%\ZooType\update`（使用者可寫，稱為 inbox）。這裡：
//! 1. 複製到只有系統管理員能寫的 `Program Files\ZooType\update`（之後驗證與執行的都是這份，不會被中途換掉）
//! 2. 驗證簽章（zoo-release 的共用金鑰，見 zoo-release 的 docs/SIGNING.md）
//! 3. 共用金鑰也簽其他專案的檔案：再讀安裝檔的版本資訊，產品名稱必須是動物園輸入法、版號要比這個程式新（防止降級）
//! 4. 靜默執行安裝檔
//!
//! 任何一步失敗都不安裝。SYSTEM 對 inbox 只讀不寫：使用者可以把 inbox 換成指向系統資料夾的連結，
//! 在那裡寫 log 或刪檔就等於替使用者在系統資料夾動手腳。log 寫在 `Program Files\ZooType\update\updater.log`，
//! inbox 的清理交給服務程序（使用者身分）。

#![windows_subsystem = "windows"]

use std::io::Write;
use std::path::{Path, PathBuf};

use zct_update::{INBOX_SETUP, INBOX_SIG};
use zoo_update::Version;

/// 安裝檔版本資訊裡的產品名稱（installer/zoo-type.iss 的 VersionInfoProductName）
const PRODUCT: &str = "動物園輸入法";

fn main() {
    // Program Files\ZooType\<版本>\zoo-type-updater.exe → Program Files\ZooType\update
    let Some(staging) = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("update")))
    else {
        return;
    };
    if std::fs::create_dir_all(&staging).is_err() {
        return;
    }
    let result = match zct_update::inbox() {
        Some(inbox) => run(&inbox, &staging),
        None => Err("找不到 ProgramData".into()),
    };
    log(
        &staging,
        &match &result {
            Ok(v) => format!("已安裝 {v}"),
            Err(e) => format!("沒有安裝：{e}"),
        },
    );
}

fn run(inbox: &Path, staging: &Path) -> Result<Version, String> {
    let setup = staging.join(INBOX_SETUP);
    for name in [INBOX_SETUP, INBOX_SIG] {
        let _ = std::fs::remove_file(staging.join(name));
        std::fs::copy(inbox.join(name), staging.join(name))
            .map_err(|e| format!("無法複製 {name}：{e}"))?;
    }
    let sig = std::fs::read_to_string(staging.join(INBOX_SIG)).map_err(|e| e.to_string())?;
    zoo_update::verify_signature_file(&setup, &sig, zoo_update::keys::ALL)
        .map_err(|e| format!("簽章不符：{e}"))?;
    let (product, text) = product_version(&setup).ok_or("安裝檔沒有版本資訊")?;
    if product != PRODUCT {
        return Err(format!("不是動物園輸入法的安裝檔（{product}）"));
    }
    let version = Version::parse(&text).ok_or(format!("版號格式不對：{text}"))?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).ok_or("目前的版號不是正式版")?;
    if version <= current {
        return Err(format!("{version} 沒有比目前的 {current} 新"));
    }
    log(staging, &format!("安裝 {version}（目前 {current}）"));
    let status = std::process::Command::new(&setup)
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/SYSTEMUPDATE",
        ])
        .status()
        .map_err(|e| format!("無法執行安裝檔：{e}"))?;
    let _ = std::fs::remove_file(&setup);
    if !status.success() {
        return Err(format!("安裝檔結束代碼 {:?}", status.code()));
    }
    Ok(version)
}

/// 檔案版本資訊的（ProductName, ProductVersion）
#[cfg(windows)]
fn product_version(path: &Path) -> Option<(String, String)> {
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    use windows::core::{HSTRING, PCWSTR};
    let file = HSTRING::from(path.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(&file, None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&file, None, size, data.as_mut_ptr().cast()) }.ok()?;
    let query = |key: &str| -> Option<(*const u16, u32)> {
        let key = HSTRING::from(key);
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        unsafe {
            VerQueryValueW(
                data.as_ptr().cast(),
                PCWSTR(key.as_ptr()),
                &mut ptr,
                &mut len,
            )
        }
        .as_bool()
        .then_some((ptr.cast::<u16>().cast_const(), len))
    };
    // 第一組語言與字碼頁
    let (lang, len) = query("\\VarFileInfo\\Translation")?;
    if len < 4 {
        return None;
    }
    let (lang, codepage) = unsafe { (*lang, *lang.add(1)) };
    let text = |name: &str| -> Option<String> {
        let (ptr, len) = query(&format!(
            "\\StringFileInfo\\{lang:04x}{codepage:04x}\\{name}"
        ))?;
        let chars = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
        let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
        Some(String::from_utf16_lossy(&chars[..end]).trim().to_owned())
    };
    Some((text("ProductName")?, text("ProductVersion")?))
}

#[cfg(not(windows))]
fn product_version(_: &Path) -> Option<(String, String)> {
    None
}

fn log(staging: &Path, line: &str) {
    let path: PathBuf = staging.join("updater.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "{secs} {line}");
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn reads_product_name_and_version_from_a_windows_binary() {
        // 系統內建的 notepad.exe 一定有版本資訊
        let windir = std::env::var("WINDIR").unwrap();
        let (name, version) =
            super::product_version(std::path::Path::new(&format!("{windir}\\notepad.exe")))
                .unwrap();
        assert!(!name.is_empty() && !version.is_empty(), "{name} {version}");
    }
}
