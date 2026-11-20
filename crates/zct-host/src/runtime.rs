//! 每個程序一份的工作：資料目錄、開啟引擎、存檔與備份、git 同步、凍結與恢復、搬移資料

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use zct_core::Engine;
use zct_core::paths::DataRoot;
use zct_preferences::Reply;

use crate::{Host, Platform};

/// 備份保留天數
const KEEP_BACKUPS: usize = 7;

fn open_engine(dictionary: &Path, data: &DataRoot) -> Result<Arc<Engine>, String> {
    let mut engine = Engine::open(dictionary, &data.learning()).map_err(|e| e.to_string())?;
    engine.load_user_words(&data.root().join("dict").join("english.txt"));
    Ok(Arc::new(engine))
}

impl Host {
    /// 開啟引擎（學習資料先從舊位置搬過來）。`dictionary`：系統詞庫資料夾
    pub fn open(platform: Platform, data: DataRoot, dictionary: PathBuf) -> Result<Host, String> {
        match Engine::migrate_from(&data.legacy_chewing_dir(), &data.learning()) {
            Ok(true) => log::info!("已從舊位置遷移學習資料"),
            Ok(false) => {}
            Err(e) => log::warn!("遷移學習資料失敗：{e}"),
        }
        let engine = open_engine(&dictionary, &data)?;
        Ok(Host::new(platform, data, dictionary, engine))
    }

    pub fn data(&self) -> DataRoot {
        self.data.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn read_only(&self) -> bool {
        self.read_only.load(Ordering::Acquire)
    }

    fn set_writable(&self, on: bool) {
        self.read_only.store(!on, Ordering::Release);
        self.settings.set_read_only(!on);
    }

    /// 已安裝新版、這個程序只服務舊 App：之後不存檔、不同步、不寫設定（不會恢復）
    pub fn set_read_only(&self) {
        if !self.read_only() {
            log::info!("改為唯讀：不再寫入使用者資料");
        }
        self.set_writable(false);
        self.engine().freeze();
    }

    /// 定期呼叫（約 30 秒）：存檔、每日備份、到了間隔就自動同步
    pub fn tick(&self) {
        if self.read_only() {
            return;
        }
        let engine = self.engine();
        let data = self.data();
        if let Err(e) = engine.save() {
            log::error!("學習紀錄存檔失敗：{e}");
        }
        if let Err(e) = engine.backup(&data.backup(), KEEP_BACKUPS) {
            log::warn!("備份失敗：{e}");
        }
        self.sync().tick(&data, &engine);
    }

    /// App 關閉等時機：只存檔
    pub fn save(&self) -> Result<(), String> {
        if self.read_only() {
            return Ok(());
        }
        self.engine().save().map(|_| ()).map_err(|e| e.to_string())
    }

    fn sync(&self) -> std::sync::MutexGuard<'_, zct_preferences::SyncWorker> {
        self.sync.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 立即同步（在背景執行）
    pub fn sync_now(&self) -> Result<Reply, String> {
        if self.read_only() {
            return Err("已安裝新版，舊版不同步".into());
        }
        self.sync().request(&self.data(), &self.engine())
    }

    pub fn sync_status(&self) -> Reply {
        self.sync().status(&self.data())
    }

    pub fn sync_running(&self) -> bool {
        self.sync().running()
    }

    /// 更新或搬移資料前：存檔並停止寫入。git 同步進行中時拒絕
    pub fn freeze(&self) -> Result<(), String> {
        if self.sync_running() {
            return Err("git 同步進行中，完成後再試".into());
        }
        let engine = self.engine();
        engine.save().map_err(|e| e.to_string())?;
        engine.freeze();
        self.set_writable(false);
        Ok(())
    }

    /// 更新失敗回復：重新開啟引擎、恢復寫入；現有 session 在沒有組字時換上
    pub fn recover(&self) -> Result<(), String> {
        self.replace_engine(open_engine(&self.dictionary, &self.data())?);
        self.set_writable(true);
        Ok(())
    }

    /// 還原備份後重新開啟引擎（還原會凍結舊引擎，不能讓它寫回）
    pub fn reload_engine(&self) -> Result<(), String> {
        self.replace_engine(open_engine(&self.dictionary, &self.data())?);
        Ok(())
    }

    /// 資料搬到 `destination`（原本的資料保留）：之後讀寫新位置。
    /// 用 `ZCT_HOME` 指定位置或同步進行中時拒絕；失敗時也會重新開啟引擎
    pub fn relocate(&self, destination: &Path) -> Result<(), String> {
        if std::env::var_os(zct_core::paths::ENV_HOME).is_some() {
            return Err("ZCT_HOME 已指定資料位置".into());
        }
        if self.sync_running() {
            return Err("請等待 git 同步完成後再搬移資料".into());
        }
        let pointer = zct_core::paths::pointer_dir().ok_or("找不到資料位置指標")?;
        let engine = self.engine();
        engine.save().map_err(|e| e.to_string())?;
        engine.freeze();
        let current = self.data();
        let result = zct_preferences::relocate_directory(&pointer, current.root(), destination);
        if result.is_ok() {
            let data = DataRoot::new(destination);
            self.settings.set_root(&data);
            *self.data.write().unwrap_or_else(|e| e.into_inner()) = data;
        }
        self.reload_engine()?;
        result.map(|_| ())
    }
}
