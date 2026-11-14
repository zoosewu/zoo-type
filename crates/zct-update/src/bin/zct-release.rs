//! 發布用工具
//!
//! - `zct-release keygen <私鑰檔>`：產生簽章金鑰。私鑰寫進檔案（不印出來，存進 GitHub Secrets
//!   `ZOO_TYPE_SIGNING_KEY` 後另外備份），公鑰印出來貼進 `zct-update/src/lib.rs` 的 `PUBLIC_KEY`
//! - `zct-release sign <安裝檔> <版號>`：用環境變數 `ZOO_TYPE_SIGNING_KEY`（私鑰 hex）簽章，寫出 `<安裝檔>.sig`

use std::process::ExitCode;

use zct_update::{PUBLIC_KEY, Version, hex, keygen, sign, unhex, verify};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["keygen", path] => cmd_keygen(path),
        ["sign", file, version] => cmd_sign(file, version),
        _ => Err("用法：zct-release keygen <私鑰檔> | zct-release sign <安裝檔> <版號>".into()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zct-release：{e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_keygen(path: &str) -> Result<(), String> {
    if std::path::Path::new(path).exists() {
        return Err(format!(
            "{path} 已經存在，不覆蓋（換金鑰後舊版程式就無法驗證新的安裝檔）"
        ));
    }
    let (secret, public) = keygen()?;
    std::fs::write(path, format!("{}\n", hex(&secret)))
        .map_err(|e| format!("無法寫入 {path}：{e}"))?;
    println!("私鑰已寫入 {path}");
    println!("公鑰（貼進 PUBLIC_KEY）：{public:?}");
    Ok(())
}

fn cmd_sign(file: &str, version: &str) -> Result<(), String> {
    let version = Version::parse(version).ok_or("版號格式不對")?;
    let key = std::env::var("ZOO_TYPE_SIGNING_KEY").map_err(|_| "沒有設定 ZOO_TYPE_SIGNING_KEY")?;
    let secret: [u8; 32] = unhex(&key)
        .and_then(|b| b.try_into().ok())
        .ok_or("ZOO_TYPE_SIGNING_KEY 不是 64 個 hex 字元")?;
    let bytes = std::fs::read(file).map_err(|e| format!("無法讀取 {file}：{e}"))?;
    let sig = sign(&secret, &version, &bytes);
    // 確認私鑰和程式內建的公鑰是一對：不是的話，發出去的版本沒有人能自動更新
    verify(&PUBLIC_KEY, &bytes, &sig).map_err(|_| "私鑰和程式內建的公鑰不是一對")?;
    let out = format!("{file}.sig");
    std::fs::write(&out, sig).map_err(|e| format!("無法寫入 {out}：{e}"))?;
    println!("已簽章：{out}");
    Ok(())
}
