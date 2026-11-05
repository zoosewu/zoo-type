//! 簡單的檔案 logger（服務程序沒有主控台）

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct FileLogger {
    file: Mutex<File>,
}

impl log::Log for FileLogger {
    /// 只記錄 ZCT 自己的訊息；第三方套件只留警告以上
    ///
    /// chewing 的 info 等級會記下每一次按鍵，等於把打字內容寫進硬碟，絕不能開
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        let ours = m.target().starts_with("zct");
        m.level() <= log::max_level() && (ours || m.level() <= log::Level::Warn)
    }

    fn log(&self, r: &log::Record<'_>) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{ts} {:<5} {}", r.level(), r.args());
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

pub fn init(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // 超過 1MB 就重新開始，避免無限成長
    if std::fs::metadata(path).map(|m| m.len() > 1 << 20).unwrap_or(false) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    let Ok(file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let logger = Box::leak(Box::new(FileLogger {
        file: Mutex::new(file),
    }));
    if log::set_logger(logger).is_ok() {
        let level = if cfg!(debug_assertions) {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        };
        log::set_max_level(level);
    }
}

#[cfg(test)]
mod tests {
    use log::{Level, Log, Metadata};

    use super::FileLogger;

    #[test]
    fn third_party_info_is_never_logged() {
        log::set_max_level(log::LevelFilter::Debug);
        let file = tempfile_file();
        let l = FileLogger {
            file: std::sync::Mutex::new(file),
        };
        let meta = |target, level| Metadata::builder().target(target).level(level).build();
        // chewing 的 info 會記下每一次按鍵
        assert!(!l.enabled(&meta("chewing::editor", Level::Info)));
        assert!(l.enabled(&meta("chewing::editor", Level::Warn)));
        assert!(l.enabled(&meta("zct_service::conn", Level::Info)));
    }

    fn tempfile_file() -> std::fs::File {
        let p = std::env::temp_dir().join(format!("zct-logger-test-{}", std::process::id()));
        std::fs::File::create(p).unwrap()
    }
}
