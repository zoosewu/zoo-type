//! 新舊版本並存
//!
//! 每個版本裝在 `安裝根目錄\<版本資料夾>\`，根目錄的 `current.txt` 記錄目前版本
//! （第一行資料夾名稱、第二行協定版本），由安裝程式寫入。
//!
//! 更新後，已開著的 App 仍載入舊 DLL，舊 DLL 會連到同一個資料夾裡的舊服務程序：
//! - 協定版本相同：舊 DLL 本來就能和新服務程序溝通 → 舊服務程序直接結束，讓位給新版
//! - 協定版本不同：舊服務程序繼續服務舊 App，但改為唯讀（不存學習、不寫設定、不同步），
//!   避免和新版互相覆蓋使用者資料；沒有連線時自行結束
//!
//! 連線名稱與單一實例的 mutex 都帶協定版本，所以不同協定的服務程序可以同時存在。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use zct_ipc::PROTOCOL_VERSION;

static READ_ONLY: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// 舊版唯讀：不存學習、不寫設定檔、不做 git 同步
pub fn read_only() -> bool {
    READ_ONLY.load(Ordering::Relaxed)
}

pub fn set_read_only() {
    READ_ONLY.store(true, Ordering::Relaxed);
}

/// 目前的連線數（沒有連線的舊版服務程序會自行結束）
pub fn active() -> usize {
    ACTIVE.load(Ordering::Relaxed)
}

/// 連線期間持有；結束時自動減一
pub struct ActiveGuard;

impl ActiveGuard {
    pub fn new() -> ActiveGuard {
        ACTIVE.fetch_add(1, Ordering::Relaxed);
        ActiveGuard
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    /// 目前版本（或不是用版本資料夾安裝的，例如開發時直接執行）
    Current,
    /// 已有更新的版本
    Stale {
        same_protocol: bool,
        /// 目前版本的服務程序
        current_exe: PathBuf,
    },
}

/// 讀 `current.txt` 判斷自己是不是目前版本
pub fn state() -> State {
    let Some(exe) = std::env::current_exe().ok() else {
        return State::Current;
    };
    let Some(folder) = exe.parent() else {
        return State::Current;
    };
    let Some(root) = folder.parent() else {
        return State::Current;
    };
    let Some(own) = folder.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return State::Current;
    };
    let Ok(text) = std::fs::read_to_string(root.join("current.txt")) else {
        return State::Current;
    };
    parse(&text, &own, root)
}

fn parse(text: &str, own: &str, root: &std::path::Path) -> State {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let (Some(dir), Some(protocol)) = (
        lines.next(),
        lines.next().and_then(|p| p.parse::<u32>().ok()),
    ) else {
        return State::Current;
    };
    if dir.eq_ignore_ascii_case(own) {
        return State::Current;
    }
    State::Stale {
        same_protocol: protocol == PROTOCOL_VERSION,
        current_exe: root.join(dir).join("zoo-type-service.exe"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn current_and_stale() {
        let root = Path::new(r"C:\ZCT");
        let p = PROTOCOL_VERSION;
        assert_eq!(
            parse(&format!("0.2.0\n{p}\n"), "0.2.0", root),
            State::Current
        );
        assert_eq!(
            parse(&format!("0.3.0\n{p}\n"), "0.2.0", root),
            State::Stale {
                same_protocol: true,
                current_exe: root.join("0.3.0").join("zoo-type-service.exe"),
            }
        );
        assert!(matches!(
            parse(&format!("0.3.0\n{}\n", p + 1), "0.2.0", root),
            State::Stale {
                same_protocol: false,
                ..
            }
        ));
        // 格式不對就當作目前版本（照常運作）
        assert_eq!(parse("", "0.2.0", root), State::Current);
        assert_eq!(parse("0.3.0\nabc", "0.2.0", root), State::Current);
    }
}
