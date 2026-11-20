//! ZCT 常駐服務程序（Windows）：引擎 session、候選窗繪製
//!
//! 每位使用者一個實例。TSF DLL 透過 named pipe 連線，每個 App 執行緒一條連線、一個 session。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod conn;
#[cfg(windows)]
mod logger;
#[cfg(windows)]
mod ui;
mod update;
#[cfg(windows)]
mod version;

#[cfg(windows)]
fn main() {
    // 安裝程式用：寫 current.txt 時需要協定版本
    if std::env::args().any(|a| a == "--protocol") {
        println!("{}", zct_ipc::PROTOCOL_VERSION);
        return;
    }
    // 安裝腳本用：確認這組程式是正式版還是開發版
    if std::env::args().any(|a| a == "--channel") {
        println!("{}", zct_core::channel::CURRENT.id);
        return;
    }
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

    use windows::core::HSTRING;

    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };

    use zct_core::CandidatePage;
    use zct_host::{AiDisplay, Host, Platform};

    use crate::{conn, logger, ui, version};

    pub struct Shared {
        /// 所有連線共用的引擎、設定、遊戲模式與中英狀態記憶
        pub host: Arc<Host>,
        pub ui: ui::UiHandle,
        /// 診斷用：資料夾 local/ 下有 `debug-keys` 檔案時，記錄非字元鍵（Shift、Delete…）的事件；
        /// 字母、數字、符號一律不記，避免記下使用者打的內容
        pub debug_keys: bool,
        /// 已下載、可以安裝的新版
        /// 自動更新（只有正式版，而且不是用 ZCT_HOME 另開的服務程序）
        pub update: Option<Arc<zct_host::update::Updater>>,
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

    /// 背景存檔：學習紀錄只在記憶體中更新，這裡每 30 秒存檔、每日備份、到了間隔就自動同步
    fn spawn_saver(host: Arc<Host>) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("saver".into())
            .spawn(move || {
                loop {
                    host.tick();
                    std::thread::sleep(std::time::Duration::from_secs(30));
                }
            })
            .map(|_| ())
    }

    /// 每秒檢查設定檔；主題變更立即套用到候選窗，其他設定由各連線在沒有組字時換上
    fn spawn_watcher(
        shared: Arc<Shared>,
        mut theme: zct_core::config::ThemeConfig,
    ) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("watcher".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    let current = shared.host.settings().current().0.theme;
                    if current != theme {
                        theme = current;
                        shared.ui.send(ui::UiCmd::Theme(theme.clone()));
                    }
                }
            })
            .map(|_| ())
    }

    /// 已有更新的版本：協定相同就讓位給新版；不同就改為唯讀，定期檢查時沒有連線就結束
    fn handle_stale(host: Option<&Host>) {
        match version::state() {
            version::State::Current => {}
            version::State::Stale {
                same_protocol: true,
                current_exe,
            } => {
                log::info!(
                    "已有新版（協定相同），改由新版服務：{}",
                    current_exe.display()
                );
                // 新版還沒啟動時（舊 DLL 叫起了舊服務程序）先啟動新版；它會等這個程序結束才拿到 mutex
                let _ = std::process::Command::new(&current_exe).spawn();
                std::process::exit(0);
            }
            version::State::Stale {
                same_protocol: false,
                ..
            } => {
                version::set_stale();
                if !version::read_only() {
                    log::info!("已有新版（協定不同），繼續服務舊 App，但不再寫入使用者資料");
                    version::set_read_only();
                }
                if let Some(host) = host {
                    host.set_read_only();
                }
                // 剛啟動（`host` 是 None）多半是舊 DLL 叫起這個舊版、正要連線：先服務，
                // 由之後每 30 秒的檢查判斷要不要結束；啟動時就結束會讓舊 App 一直連不上（只能打英文）
                if host.is_some() && version::active() == 0 {
                    log::info!("舊版沒有連線，結束");
                    std::process::exit(0);
                }
            }
        }
    }

    /// 每 30 秒檢查是否已安裝新版本
    fn spawn_version_watch(host: Arc<Host>) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("version".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    handle_stale(Some(&host));
                }
            })
            .map(|_| ())
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        // 單一實例：同一個登入工作階段、同一個協定版本、同一個資料位置只跑一個服務程序
        let name = HSTRING::from(format!(
            "Local\\{}-service-v{}{}",
            zct_core::channel::CURRENT.pipe,
            zct_ipc::PROTOCOL_VERSION,
            zct_core::paths::instance_suffix()
        ));
        let mut mutex = unsafe { CreateMutexW(None, true, &name)? };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // 舊版讓位時：等它結束（最多 3 秒）再接手
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(mutex) };
            let mut acquired = false;
            for _ in 0..30 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                mutex = unsafe { CreateMutexW(None, true, &name)? };
                if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
                    acquired = true;
                    break;
                }
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(mutex) };
            }
            if !acquired {
                return Ok(());
            }
        }
        let _mutex = mutex;
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }

        // 改名前的資料夾搬移、開發版第一次從正式版複製一份資料（關掉同步）
        let (data, done) = zct_core::paths::prepare_data()?;
        logger::init(&data.local().join("logs").join("service.log"));
        log::info!("啟動，資料根目錄：{}", data.root().display());
        for m in done {
            log::info!("{m}");
        }
        handle_stale(None);

        // 設定檔寫錯時仍要能打字：主機的設定會記錄錯誤並改用上一份正確的設定或預設值
        let theme = zct_core::Config::load(&data.config())
            .map(|c| c.theme)
            .unwrap_or_default();
        let sys_dict_dir = find_sys_dict()?;
        log::info!("系統詞庫：{}", sys_dict_dir.display());

        let debug_keys = data.local().join("debug-keys").exists();
        let ui = ui::spawn(theme.clone())?;
        // AI 結果回來時直接更新候選窗，不必等下一次按鍵
        let notify_ui = ui.clone();
        let host = Host::open(Platform::Windows, data, sys_dict_dir)?
            .with_ai_key_source(Box::new(zct_ipc::secret::read_ai_key))
            .with_ai_notify(Box::new(move |owner, display| match display {
                AiDisplay::Suggestion(text) => notify_ui.send(ui::UiCmd::Show {
                    owner,
                    page: CandidatePage {
                        items: vec![text],
                        page: 0,
                        total_pages: 1,
                        highlight: 0,
                        pinned: vec![false],
                        ..Default::default()
                    },
                    anchor: None,
                }),
                AiDisplay::Notice(text) => notify_ui.send(ui::UiCmd::Hint { text, anchor: None }),
            }));
        if version::read_only() {
            host.set_read_only();
        }
        let host = Arc::new(host);
        spawn_saver(host.clone())?;
        spawn_version_watch(host.clone())?;

        crate::update::fix_autostart();
        let update = match zct_host::update::Target::windows() {
            Some(target) if zct_host::update::manages_updates() && !version::read_only() => {
                let updater = zct_host::update::Updater::new(target);
                updater.spawn_schedule(host.clone())?;
                Some(updater)
            }
            _ => None,
        };
        let shared = Arc::new(Shared {
            host,
            ui,
            debug_keys,
            update,
        });
        if shared.debug_keys {
            log::info!("按鍵診斷已開啟（只記錄非字元鍵）");
        }
        spawn_watcher(shared.clone(), theme)?;

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
