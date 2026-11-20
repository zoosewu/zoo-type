//! 自動更新的 Windows 部分：安裝與登入時啟動
//!
//! 查、下載、驗證、排程在共用的 `zct_host::update`（下載到 `%ProgramData%\ZooType\update`）。
//! 輸入法圖示的選單出現「更新到 vX」；按下後啟動排程工作「ZooType Updater」（以 SYSTEM 執行，不跳 UAC），
//! 真正的把關在那裡：只安裝簽章正確、比目前新的版本。

use zct_host::update::manages_updates;
use zct_update::TASK_NAME;

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
    if !manages_updates() {
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
