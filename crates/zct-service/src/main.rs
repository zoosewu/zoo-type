//! ZCT 常駐服務程序（Windows）：引擎 session、候選窗繪製
//!
//! 每位使用者一個實例。TSF DLL 透過 named pipe 連線，每個 App 執行緒一條連線、一個 session。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod conn;
#[cfg(windows)]
mod logger;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn main() {
    if let Err(e) = app::run() {
        log::error!("服務程序結束：{e}");
        eprintln!("zct-service: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("zct-service 只支援 Windows");
}

#[cfg(windows)]
mod app {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::core::w;
    use zct_core::paths::DataRoot;

    use crate::{conn, logger, settings, ui};

    pub struct Shared {
        pub data: DataRoot,
        pub sys_dict_dir: PathBuf,
        pub settings: settings::Settings,
        /// 手動快捷鍵開啟的全域遊戲模式
        pub game: AtomicBool,
        pub ui: ui::UiHandle,
    }

    /// 系統詞庫：正式安裝放在執行檔旁的 `dict/`；開發時可用 `ZCT_DICT` 指定
    fn find_sys_dict() -> Result<PathBuf, String> {
        if let Some(p) = std::env::var_os("ZCT_DICT") {
            return Ok(p.into());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = exe.parent().ok_or("找不到執行檔資料夾")?.join("dict");
        if dir.join("static_dict.bin").exists() {
            Ok(dir)
        } else {
            Err(format!("找不到系統詞庫：{}", dir.display()))
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        // 單一實例：同一個登入工作階段只跑一個服務程序
        let _mutex = unsafe { CreateMutexW(None, true, w!("Local\\zct-service"))? };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }

        let data = DataRoot::discover().ok_or("找不到資料根目錄")?;
        logger::init(&data.local().join("logs").join("service.log"));
        log::info!("啟動，資料根目錄：{}", data.root().display());

        // 設定檔寫錯時仍要能打字：settings 會記錄錯誤並改用上一份正確的設定或預設值
        let settings = settings::Settings::new(&data);
        settings.current();
        let sys_dict_dir = find_sys_dict()?;
        log::info!("系統詞庫：{}", sys_dict_dir.display());

        let ui = ui::spawn()?;
        let shared = Arc::new(Shared {
            data,
            sys_dict_dir,
            settings,
            game: AtomicBool::new(false),
            ui,
        });

        let mut listener = zct_ipc::pipe::Listener::new()?;
        let mut next_id = 1u64;
        loop {
            match listener.accept() {
                Ok(c) => {
                    let id = next_id;
                    next_id += 1;
                    let shared = shared.clone();
                    std::thread::Builder::new()
                        .name(format!("conn-{id}"))
                        .spawn(move || conn::serve(id, c, &shared))?;
                }
                Err(e) => {
                    log::error!("接受連線失敗：{e}");
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    }
}
