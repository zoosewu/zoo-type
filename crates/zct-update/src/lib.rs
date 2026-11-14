//! 自動更新的信任基礎：安裝檔的數位簽章（ed25519）
//!
//! 發布時用私鑰（GitHub Secrets `ZOO_TYPE_SIGNING_KEY`）簽安裝檔，程式內建公鑰驗證。
//! 以 SYSTEM 執行的 `zoo-type-updater` 只安裝簽章正確、而且比目前新的版本：
//! SHA256 和安裝檔放在同一個 Release，能改安裝檔的人也能改 SHA256，所以只比對雜湊不夠。
//!
//! 簽的內容是「格式標記 + 版號 + 安裝檔的 SHA256」：版號也在簽章裡，拿舊版的合法安裝檔改名也無法降級。
//! `.sig` 檔是三行文字：格式標記、版號、簽章（hex）。

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

/// 發布用私鑰對應的公鑰（`zct-release keygen` 產生，2026-11-14）
pub const PUBLIC_KEY: [u8; 32] = [
    219, 161, 217, 167, 143, 167, 234, 214, 247, 194, 87, 171, 242, 200, 175, 78, 182, 90, 129, 93,
    191, 61, 207, 95, 192, 208, 141, 127, 94, 164, 86, 3,
];

const MAGIC: &str = "zoo-type-update-v1";

/// 服務程序把下載的安裝檔放在 `%ProgramData%\ZooType\update`（檔名固定），updater 從這裡取
pub const INBOX_SETUP: &str = "setup.exe";
pub const INBOX_SIG: &str = "setup.exe.sig";

pub fn inbox() -> Option<std::path::PathBuf> {
    std::env::var_os("ProgramData")
        .map(|p| std::path::PathBuf::from(p).join("ZooType").join("update"))
}

/// 排程工作名稱（安裝時建立、服務程序啟動它）
pub const TASK_NAME: &str = "ZooType Updater";

/// 版號：`主.次.修`，可以帶 `-rc.1` 這類後綴（比新舊時後綴版比正式版舊）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub numbers: [u64; 3],
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches('v');
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p.to_owned())),
            None => (s, None),
        };
        let mut it = core.split('.').map(|n| n.parse::<u64>().ok());
        let numbers = [it.next()??, it.next()??, it.next()??];
        if it.next().is_some() || pre.as_deref() == Some("") {
            return None;
        }
        Some(Version { numbers, pre })
    }

    /// 這個程式自己的版號
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo.toml 的版號")
    }

    pub fn is_newer_than(&self, other: &Version) -> bool {
        self.cmp_key() > other.cmp_key()
    }

    fn cmp_key(&self) -> ([u64; 3], bool, &str) {
        // 同樣的數字：沒有後綴（正式版）比有後綴新
        (
            self.numbers,
            self.pre.is_none(),
            self.pre.as_deref().unwrap_or(""),
        )
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c] = self.numbers;
        write!(f, "{a}.{b}.{c}")?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

fn message(version: &Version, file: &[u8]) -> Vec<u8> {
    let hash = hex(&Sha256::digest(file));
    format!("{MAGIC}\n{version}\n{hash}\n").into_bytes()
}

/// 用私鑰（32 位元組）簽安裝檔，回傳 `.sig` 檔內容
pub fn sign(secret: &[u8; 32], version: &Version, file: &[u8]) -> String {
    let key = SigningKey::from_bytes(secret);
    let sig = key.sign(&message(version, file));
    format!("{MAGIC}\n{version}\n{}\n", hex(&sig.to_bytes()))
}

/// 驗證安裝檔與 `.sig`；正確時回傳簽章裡的版號
pub fn verify(public: &[u8; 32], file: &[u8], sig_text: &str) -> Result<Version, String> {
    let mut lines = sig_text.lines();
    if lines.next() != Some(MAGIC) {
        return Err("簽章檔格式不對".into());
    }
    let version = lines
        .next()
        .and_then(Version::parse)
        .ok_or("簽章檔沒有版號")?;
    let bytes = lines.next().and_then(unhex).ok_or("簽章檔沒有簽章")?;
    let sig = Signature::from_slice(&bytes).map_err(|_| "簽章長度不對")?;
    let key = VerifyingKey::from_bytes(public).map_err(|_| "公鑰無效")?;
    key.verify_strict(&message(&version, file), &sig)
        .map_err(|_| "簽章不符：安裝檔不是官方發布的，或已被修改".to_owned())?;
    Ok(version)
}

/// 產生新的私鑰（32 位元組）與對應的公鑰
pub fn keygen() -> Result<([u8; 32], [u8; 32]), String> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| format!("無法取得亂數：{e}"))?;
    let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
    Ok((secret, public))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let (secret, public) = keygen().unwrap();
        let v = Version::parse("0.2.0").unwrap();
        let file = b"installer bytes";
        let sig = sign(&secret, &v, file);
        assert_eq!(verify(&public, file, &sig), Ok(v.clone()));
        // 檔案被改
        assert!(verify(&public, b"tampered", &sig).is_err());
        // 改簽章檔裡的版號（想把舊版當新版裝）
        let forged = sig.replace("0.2.0", "9.9.9");
        assert!(verify(&public, file, &forged).is_err());
        // 別把鑰匙的簽章
        let (other, _) = keygen().unwrap();
        assert!(verify(&public, file, &sign(&other, &v, file)).is_err());
    }

    #[test]
    fn versions_compare() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.1.1").is_newer_than(&v("0.1.0")));
        assert!(v("v0.2.0").is_newer_than(&v("0.1.9")));
        assert!(v("0.2.0").is_newer_than(&v("0.2.0-rc.1")));
        assert!(!v("0.1.0").is_newer_than(&v("0.1.0")));
        assert!(Version::parse("0.1").is_none());
        assert!(Version::parse("0.1.0-").is_none());
        assert_eq!(v("1.2.3-rc.1").to_string(), "1.2.3-rc.1");
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(unhex(&hex(&[0, 15, 255])), Some(vec![0, 15, 255]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }
}
