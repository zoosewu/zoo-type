//! 以 SYSTEM 身分安裝更新（由排程工作「ZooType Updater」啟動，一般使用者可以觸發）
//!
//! 服務程序把安裝檔與 `.sig` 下載到 `%ProgramData%\ZooType\update`（使用者可寫，稱為 inbox）。這裡：
//! 1. 複製到只有系統管理員能寫的 `Program Files\ZooType\update`（之後驗證與執行的都是這份，不會被中途換掉）
//! 2. 驗證簽章（內建公鑰），而且版號要比這個程式新（防止降級）
//! 3. 靜默執行安裝檔
//!
//! 任何一步失敗都不安裝。SYSTEM 對 inbox 只讀不寫：使用者可以把 inbox 換成指向系統資料夾的連結，
//! 在那裡寫 log 或刪檔就等於替使用者在系統資料夾動手腳。log 寫在 `Program Files\ZooType\update\updater.log`，
//! inbox 的清理交給服務程序（使用者身分）。

#![windows_subsystem = "windows"]

use std::io::Write;
use std::path::{Path, PathBuf};

use zct_update::{INBOX_SETUP, INBOX_SIG, PUBLIC_KEY, Version, verify};

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
    let bytes = std::fs::read(&setup).map_err(|e| e.to_string())?;
    let sig = std::fs::read_to_string(staging.join(INBOX_SIG)).map_err(|e| e.to_string())?;
    let version = verify(&PUBLIC_KEY, &bytes, &sig)?;
    drop(bytes);
    let current = Version::current();
    if !version.is_newer_than(&current) {
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
