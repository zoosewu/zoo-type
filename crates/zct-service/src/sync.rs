//! git 同步（需求 4.8）：呼叫系統的 git，沿用使用者既有的 SSH / 憑證設定
//!
//! 流程：匯出 → commit → pull（merge）→ 匯入 → push。結果寫在 `local/sync-status.txt`。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zct_core::config::SyncConfig;
use zct_core::engine::today;
use zct_core::paths::DataRoot;
use zct_core::{Engine, sync};

use crate::settings::Settings;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const BRANCH: &str = "main";

const GITIGNORE: &str = "# 動物園輸入法：單機資料不同步\nlocal/\n*.tmp\n";
const GITATTRIBUTES: &str = "# 詞庫與統計逐行合併，衝突的行兩邊都保留，讀入時以時間較新的為準\ndict/*.tsv merge=union\ndict/english.txt merge=union\nstats/*.tsv merge=union\n";

fn machine() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct Git<'a> {
    root: &'a Path,
}

impl Git<'_> {
    fn run(&self, args: &[&str]) -> Result<Output, String> {
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(self.root).args(args);
        // 不能跳出輸入帳密的提示（服務程序沒有主控台）
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.output().map_err(|e| format!("無法執行 git：{e}"))
    }

    fn ok(&self, args: &[&str]) -> Result<String, String> {
        let out = self.run(args)?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        } else {
            Err(format!(
                "git {} 失敗：{}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
}

fn write_if_missing(path: &Path, text: &str) -> std::io::Result<()> {
    if !path.exists() {
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// 跑一次同步；回傳給使用者看的結果
pub fn run_once(root: &Path, cfg: &SyncConfig, engine: &Engine) -> Result<String, String> {
    let git = Git { root };
    git.ok(&["--version"])
        .map_err(|_| "找不到 git，請先安裝 Git".to_owned())?;

    if !root.join(".git").exists() {
        git.ok(&["init", "-b", BRANCH])?;
    }
    write_if_missing(&root.join(".gitignore"), GITIGNORE).map_err(|e| e.to_string())?;
    write_if_missing(&root.join(".gitattributes"), GITATTRIBUTES).map_err(|e| e.to_string())?;

    let remote = cfg.remote.trim();
    if !remote.is_empty() {
        match git.ok(&["remote", "get-url", "origin"]) {
            Ok(url) if url == remote => {}
            Ok(_) => {
                git.ok(&["remote", "set-url", "origin", remote])?;
            }
            Err(_) => {
                git.ok(&["remote", "add", "origin", remote])?;
            }
        }
    }

    let dict = root.join("dict").join("user.tsv");

    // 0. 新電腦加入：本機還沒有任何 commit 時，先採用遠端的內容（設定、App 規則、詞庫），
    //    否則兩邊是不相關的歷史而無法合併。`local/` 不受影響；本機詞庫在下面匯出時合併進去。
    let mut changed = 0;
    let fresh = !git
        .run(&["rev-parse", "--verify", "HEAD"])?
        .status
        .success();
    if fresh && !remote.is_empty() && git.run(&["fetch", "origin", BRANCH])?.status.success() {
        git.ok(&["reset", "--hard", &format!("origin/{BRANCH}")])?;
        let remote_dict = std::fs::read_to_string(&dict).unwrap_or_default();
        changed += sync::import(engine, &remote_dict);
    }

    // 1. 匯出
    std::fs::create_dir_all(dict.parent().unwrap_or(root)).map_err(|e| e.to_string())?;
    let prev = std::fs::read_to_string(&dict).unwrap_or_default();
    std::fs::write(&dict, sync::export(engine, &prev, now())).map_err(|e| e.to_string())?;
    let stats_dir = root.join("stats");
    std::fs::create_dir_all(&stats_dir).map_err(|e| e.to_string())?;
    let local_stats = engine.dir().join("stats.tsv");
    if local_stats.exists() {
        std::fs::copy(&local_stats, stats_dir.join(format!("{}.tsv", machine())))
            .map_err(|e| e.to_string())?;
    }

    // 2. commit（沒有設定 git 身分時使用預設值，避免 commit 失敗）
    git.ok(&["add", "-A"])?;
    let staged = !git.run(&["diff", "--cached", "--quiet"])?.status.success();
    if staged {
        let msg = format!("動物園輸入法：{} {}", machine(), today());
        let has_identity = git
            .ok(&["config", "user.email"])
            .is_ok_and(|s| !s.is_empty());
        if has_identity {
            git.ok(&["commit", "-m", &msg])?;
        } else {
            git.ok(&[
                "-c",
                "user.name=動物園輸入法",
                "-c",
                "user.email=zoo-type@localhost",
                "commit",
                "-m",
                &msg,
            ])?;
        }
    }
    if remote.is_empty() {
        return Ok("已在本機 commit（未設定遠端）".into());
    }

    // 3. pull：遠端還沒有分支時略過；衝突時放棄合併，不留下半套的檔案
    let pull = git.run(&["pull", "--no-rebase", "--no-edit", "origin", BRANCH])?;
    if !pull.status.success() {
        let err = String::from_utf8_lossy(&pull.stderr).to_string();
        if !err.contains("couldn't find remote ref") {
            let _ = git.run(&["merge", "--abort"]);
            return Err(format!("同步衝突或無法取得遠端：{}", err.trim()));
        }
    }

    // 4. 匯入合併後的詞庫
    let merged = std::fs::read_to_string(&dict).unwrap_or_default();
    changed += sync::import(engine, &merged);

    // 5. push
    git.ok(&["push", "-u", "origin", BRANCH])?;
    Ok(format!("同步完成，套用遠端 {changed} 筆詞庫變更"))
}

fn write_status(data: &DataRoot, result: &Result<String, String>) {
    let text = match result {
        Ok(s) => format!("{} 成功：{s}", today()),
        Err(e) => format!("{} 失敗：{e}", today()),
    };
    let _ = std::fs::write(data.local().join("sync-status.txt"), text);
}

/// 背景同步執行緒；回傳的 Sender 用來觸發「立即同步」
pub fn spawn(
    data: DataRoot,
    settings: Arc<Settings>,
    engine: Arc<Engine>,
) -> std::io::Result<Sender<()>> {
    let (tx, rx): (Sender<()>, Receiver<()>) = channel();
    std::thread::Builder::new()
        .name("sync".into())
        .spawn(move || {
            let root: PathBuf = data.root().to_path_buf();
            loop {
                let cfg = settings.current().0.sync;
                let wait = Duration::from_secs(cfg.interval_minutes.max(1) * 60);
                let manual = match rx.recv_timeout(wait) {
                    Ok(()) => true,
                    Err(RecvTimeoutError::Timeout) => false,
                    Err(RecvTimeoutError::Disconnected) => return,
                };
                let cfg = settings.current().0.sync;
                // 已安裝新版時舊版不同步，避免用舊資料覆蓋
                if (!cfg.enabled && !manual) || crate::version::read_only() {
                    continue;
                }
                // 先存檔，確保匯出的是最新的詞庫
                let _ = engine.save();
                let result = run_once(&root, &cfg, &engine);
                match &result {
                    Ok(s) => log::info!("同步：{s}"),
                    Err(e) => log::warn!("同步：{e}"),
                }
                write_status(&data, &result);
            }
        })?;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zct_core::PhraseAction;

    fn engine(dir: &Path) -> Engine {
        let dict = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/dict");
        Engine::open(&dict, &dir.join("local").join("learning")).unwrap()
    }

    fn git(dir: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn two_machines_sync_through_remote() {
        let tmp = std::env::temp_dir().join(format!("zct-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (remote, a_root, b_root) = (tmp.join("remote.git"), tmp.join("a"), tmp.join("b"));
        for d in [&remote, &a_root, &b_root] {
            std::fs::create_dir_all(d).unwrap();
        }
        git(&remote, &["init", "--bare", "-b", BRANCH]);
        let cfg = SyncConfig {
            enabled: true,
            remote: remote.to_string_lossy().into_owned(),
            interval_minutes: 10,
        };

        let a = engine(&a_root);
        let b = engine(&b_root);
        a.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
        run_once(&a_root, &cfg, &a).unwrap();
        let msg = run_once(&b_root, &cfg, &b).unwrap();
        assert!(msg.contains("1 筆"), "{msg}");
        assert!(
            b.phrases()
                .iter()
                .any(|p| p.word == "酷音" && p.is_pinned())
        );

        // B 封鎖後同步，A 拿到封鎖
        std::thread::sleep(Duration::from_millis(1100)); // 修改時間以秒計
        b.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", PhraseAction::Ban)
            .unwrap();
        run_once(&b_root, &cfg, &b).unwrap();
        run_once(&a_root, &cfg, &a).unwrap();
        assert!(
            a.phrases()
                .iter()
                .any(|p| p.word == "酷音" && p.is_banned())
        );

        // A 移除後同步，B 也移除（不會被同步檔加回來）
        std::thread::sleep(Duration::from_millis(1100));
        a.phrase_action("酷音", "ㄎㄨˋ ㄧㄣ", PhraseAction::Remove)
            .unwrap();
        run_once(&a_root, &cfg, &a).unwrap();
        assert!(
            a.phrases().iter().all(|p| p.word != "酷音"),
            "A 自己同步後不會加回來"
        );
        run_once(&b_root, &cfg, &b).unwrap();
        assert!(b.phrases().iter().all(|p| p.word != "酷音"));

        // local/ 不進 git
        let tracked = Git { root: &a_root }.ok(&["ls-files"]).unwrap();
        assert!(tracked.contains("dict/user.tsv"));
        assert!(!tracked.contains("local/"), "{tracked}");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
