use crate::Reply;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zct_core::paths::DataRoot;
use zct_core::{Config, Engine};

/// One in-flight job per host. Jobs own an engine snapshot, never a C/Swift handle.
#[derive(Default)]
pub struct SyncWorker {
    state: Arc<Mutex<(bool, String)>>,
    last: Option<(PathBuf, Instant)>,
}
impl SyncWorker {
    pub fn status(&self, data: &DataRoot) -> Reply {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let message = if state.0 {
            "同步中…".into()
        } else {
            std::fs::read_to_string(data.local().join("sync-status.txt")).unwrap_or_else(|_| {
                if state.1.is_empty() {
                    "尚未同步".into()
                } else {
                    state.1.clone()
                }
            })
        };
        Reply::SyncStatus {
            running: state.0,
            message,
        }
    }
    pub fn running(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).0
    }
    pub fn tick(&mut self, data: &DataRoot, engine: &Arc<Engine>) {
        let Ok(config) = Config::load(&data.config()) else {
            return;
        };
        if !config.sync.enabled {
            return;
        }
        let interval = Duration::from_secs(config.sync.interval_minutes.max(1).saturating_mul(60));
        if self
            .last
            .as_ref()
            .is_none_or(|(root, time)| root != data.root() || time.elapsed() >= interval)
        {
            let _ = self.request(data, engine);
        }
    }
    pub fn request(&mut self, data: &DataRoot, engine: &Arc<Engine>) -> Result<Reply, String> {
        let config = Config::load(&data.config())
            .map_err(|e| e.to_string())?
            .sync;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.0 {
            return Err("同步正在進行中".into());
        }
        state.0 = true;
        let state_copy = self.state.clone();
        let data_copy = data.clone();
        let engine = engine.clone();
        let spawned = std::thread::Builder::new()
            .name("git-sync".into())
            .spawn(move || {
                let result = (|| {
                    std::fs::create_dir_all(data_copy.local()).map_err(|e| e.to_string())?;
                    let lock = std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(false)
                        .write(true)
                        .open(data_copy.local().join("sync.lock"))
                        .map_err(|e| e.to_string())?;
                    lock.try_lock()
                        .map_err(|_| "其他程序正在同步".to_string())?;
                    engine.save().map_err(|e| e.to_string())?;
                    let result = crate::git::run_once(data_copy.root(), &config, &engine);
                    // Persist imports before reporting success, also when push failed.
                    engine.save().map_err(|e| e.to_string())?;
                    result
                })();
                let message = match result {
                    Ok(message) => format!("成功：{message}"),
                    Err(error) => format!("失敗：{error}"),
                };
                let _ = std::fs::write(data_copy.local().join("sync-status.txt"), &message);
                let mut state = state_copy.lock().unwrap_or_else(|e| e.into_inner());
                *state = (false, message);
            });
        if let Err(error) = spawned {
            state.0 = false;
            return Err(error.to_string());
        }
        drop(state);
        self.last = Some((data.root().to_owned(), Instant::now()));
        Ok(self.status(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(worker: &SyncWorker) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while worker.running() {
            assert!(Instant::now() < deadline, "background sync timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn manual_sync_runs_off_thread_even_when_automatic_sync_is_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let data = DataRoot::new(tmp.path());
        Config::default().save(&data.config()).unwrap();
        let dict = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/dict");
        let engine = Arc::new(Engine::open(&dict, &data.learning()).unwrap());
        let mut worker = SyncWorker::default();
        worker.tick(&data, &engine);
        assert!(!data.root().join(".git").exists());
        worker.request(&data, &engine).unwrap();
        wait(&worker);
        assert!(data.root().join(".git").exists());
        let Reply::SyncStatus { running, message } = worker.status(&data) else {
            panic!()
        };
        assert!(!running && message.contains("成功"), "{message}");
        let mut config = Config::default();
        config.sync.remote = tmp
            .path()
            .join("missing.git")
            .to_string_lossy()
            .into_owned();
        config.save(&data.config()).unwrap();
        worker.request(&data, &engine).unwrap();
        wait(&worker);
        let Reply::SyncStatus { message, .. } = worker.status(&data) else {
            panic!()
        };
        assert!(message.contains("失敗"), "{message}");
        config.sync.remote.clear();
        config.sync.enabled = true;
        config.save(&data.config()).unwrap();
        let mut automatic = SyncWorker::default();
        automatic.tick(&data, &engine);
        wait(&automatic);
        assert!(automatic.last.is_some());
    }
}
