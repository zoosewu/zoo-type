//! Shared Git runner. Hosts schedule it off the input/UI thread.
use std::path::Path;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};
use zct_core::config::SyncConfig;
use zct_core::engine::today;
use zct_core::{Engine, sync};
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const BRANCH: &str = "main";

const GITIGNORE: &str = "# 動物園輸入法：單機資料不同步\nlocal/\n*.tmp\n.settings-write.lock\n";
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
        #[cfg(target_os = "macos")]
        if std::env::var_os("GIT_SSH_COMMAND").is_none() {
            cmd.env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=15");
        }
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
    let settings_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".settings-write.lock"))
        .map_err(|e| e.to_string())?;
    settings_lock
        .try_lock()
        .map_err(|_| "設定或同步正由其他程序寫入，請稍後重試".to_string())?;
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
    let english = root.join("dict/english.txt");
    let local_english = std::fs::read_to_string(&english).ok();
    if fresh && !remote.is_empty() && git.run(&["fetch", "origin", BRANCH])?.status.success() {
        git.ok(&["reset", "--hard", &format!("origin/{BRANCH}")])?;
        let remote_dict = std::fs::read_to_string(&dict).unwrap_or_default();
        changed += sync::import(engine, &remote_dict);
        if let Some(local) = local_english {
            std::fs::create_dir_all(root.join("dict")).map_err(|e| e.to_string())?;
            let remote_words = std::fs::read_to_string(&english).unwrap_or_default();
            std::fs::write(&english, format!("{remote_words}\n{local}"))
                .map_err(|e| e.to_string())?;
        }
    }

    let ignore = root.join(".gitignore");
    let text = std::fs::read_to_string(&ignore).map_err(|e| e.to_string())?;
    if !text.lines().any(|line| line == ".settings-write.lock") {
        std::fs::write(&ignore, format!("{text}\n.settings-write.lock\n"))
            .map_err(|e| e.to_string())?;
    }

    // Merge commits also need an identity, not just our explicit commits.
    for (key, fallback) in [
        ("user.name", "動物園輸入法"),
        ("user.email", "zoo-type@localhost"),
    ] {
        if !git
            .ok(&["config", key])
            .is_ok_and(|value| !value.is_empty())
        {
            git.ok(&["config", "--local", key, fallback])?;
        }
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

    engine.reload_user_words(&root.join("dict/english.txt"));

    // 5. push
    git.ok(&["push", "-u", "origin", BRANCH])?;
    Ok(format!("同步完成，套用遠端 {changed} 筆詞庫變更"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
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
        std::fs::create_dir_all(a_root.join("dict")).unwrap();
        std::fs::create_dir_all(b_root.join("dict")).unwrap();
        std::fs::write(a_root.join("dict/english.txt"), "zoosyncalpha\n").unwrap();
        std::fs::write(b_root.join("dict/english.txt"), "zoosyncbeta\n").unwrap();
        a.add_phrase("酷音", "ㄎㄨˋ ㄧㄣ").unwrap();
        run_once(&a_root, &cfg, &a).unwrap();
        let msg = run_once(&b_root, &cfg, &b).unwrap();
        assert!(msg.contains("1 筆"), "{msg}");
        assert!(
            b.phrases()
                .iter()
                .any(|p| p.word == "酷音" && p.is_pinned())
        );

        assert!(b.is_english("zoosyncalpha"));
        assert!(b.is_english("zoosyncbeta"));

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
    #[test]
    fn settings_conflict_is_reported_and_merge_is_aborted() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        let a_root = tmp.path().join("a");
        let b_root = tmp.path().join("b");
        for path in [&remote, &a_root, &b_root] {
            std::fs::create_dir_all(path).unwrap();
        }
        git(&remote, &["init", "--bare", "-b", BRANCH]);
        let config = SyncConfig {
            enabled: true,
            remote: remote.to_string_lossy().into_owned(),
            interval_minutes: 10,
        };
        std::fs::write(a_root.join("config.toml"), "value = 1\n").unwrap();
        let a = engine(&a_root);
        let b = engine(&b_root);
        run_once(&a_root, &config, &a).unwrap();
        run_once(&b_root, &config, &b).unwrap();
        std::fs::write(a_root.join("config.toml"), "value = 2\n").unwrap();
        std::fs::write(b_root.join("config.toml"), "value = 3\n").unwrap();
        run_once(&a_root, &config, &a).unwrap();
        let error = run_once(&b_root, &config, &b).unwrap_err();
        assert!(error.contains("衝突"), "{error}");
        assert_eq!(
            std::fs::read_to_string(b_root.join("config.toml")).unwrap(),
            "value = 3\n"
        );
        assert!(!b_root.join(".git/MERGE_HEAD").exists());
        assert!(
            Git { root: &b_root }
                .ok(&["status", "--porcelain"])
                .unwrap()
                .is_empty()
        );
    }
}
