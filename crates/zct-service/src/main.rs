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
mod sync;
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
    use zct_core::Engine;
    use zct_core::paths::DataRoot;

    use crate::{conn, logger, settings, sync, ui};

    pub struct Shared {
        pub settings: Arc<settings::Settings>,
        /// 觸發「立即同步」
        pub sync: std::sync::mpsc::Sender<()>,
        /// 所有連線共用的詞庫與學習紀錄
        pub engine: Arc<Engine>,
        /// 每日備份資料夾
        pub backup: PathBuf,
        /// 手動快捷鍵或系統匣開啟的全域遊戲模式
        pub game: Arc<AtomicBool>,
        pub ui: ui::UiHandle,
        /// 診斷用：資料夾 local/ 下有 `debug-keys` 檔案時，記錄非字元鍵（Shift、Delete…）的事件；
        /// 字母、數字、符號一律不記，避免記下使用者打的內容
        pub debug_keys: bool,
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

    /// 背景存檔：學習紀錄只在記憶體中更新，這裡定期寫檔並每日備份
    fn spawn_saver(engine: Arc<Engine>, backup: PathBuf) -> std::io::Result<()> {
        const INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
        const KEEP_DAYS: usize = 7;
        std::thread::Builder::new()
            .name("saver".into())
            .spawn(move || {
                loop {
                    if let Err(e) = engine.save() {
                        log::error!("學習紀錄存檔失敗：{e}");
                    }
                    if let Err(e) = engine.backup(&backup, KEEP_DAYS) {
                        log::warn!("備份失敗：{e}");
                    }
                    std::thread::sleep(INTERVAL);
                }
            })
            .map(|_| ())
    }

    /// 每秒檢查設定檔；主題變更立即套用到候選窗（其他設定在 App 重新連線時生效）
    fn spawn_watcher(
        shared: Arc<Shared>,
        mut theme: zct_core::config::ThemeConfig,
    ) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("watcher".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    let current = shared.settings.current().0.theme;
                    if current != theme {
                        theme = current;
                        shared.ui.send(ui::UiCmd::Theme(theme.clone()));
                    }
                }
            })
            .map(|_| ())
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
        let settings = Arc::new(settings::Settings::new(&data));
        let theme = settings.current().0.theme;
        let sys_dict_dir = find_sys_dict()?;
        log::info!("系統詞庫：{}", sys_dict_dir.display());

        match Engine::migrate_from(&data.legacy_chewing_dir(), &data.learning()) {
            Ok(true) => log::info!("已從舊位置遷移學習資料"),
            Ok(false) => {}
            Err(e) => log::warn!("遷移學習資料失敗：{e}"),
        }
        let engine = Arc::new(Engine::open(&sys_dict_dir, &data.learning())?);
        spawn_saver(engine.clone(), data.backup())?;
        let sync = sync::spawn(data.clone(), settings.clone(), engine.clone())?;

        let game = Arc::new(AtomicBool::new(false));
        let ui = ui::spawn(theme.clone())?;
        let shared = Arc::new(Shared {
            settings,
            sync,
            engine,
            backup: data.backup(),
            game,
            ui,
            debug_keys: data.local().join("debug-keys").exists(),
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
