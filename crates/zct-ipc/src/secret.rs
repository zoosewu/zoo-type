//! 機密資料（AI API Key）存在 Windows 認證管理員，絕不寫進會被 git 同步的設定檔

use windows::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use windows::core::{HSTRING, PWSTR};

/// 認證管理員中的名稱（「一般認證」分類下可以看到）；開發版是 `ZooType-dev/ai-api-key`
pub fn ai_api_key() -> String {
    format!("{}/ai-api-key", zct_core::channel::CURRENT.dir)
}
/// 改名前的名稱：讀取時自動搬到新名稱
const LEGACY_AI_API_KEY: &str = "ZCT/ai-api-key";

/// 讀 AI API Key；只有舊名稱時搬到新名稱
pub fn read_ai_key() -> Option<String> {
    if let Some(k) = read(&ai_api_key()) {
        return Some(k);
    }
    if zct_core::channel::CURRENT.id != zct_core::channel::OFFICIAL.id {
        return None;
    }
    let old = read(LEGACY_AI_API_KEY)?;
    if write(&ai_api_key(), &old).is_ok() {
        delete(LEGACY_AI_API_KEY);
    }
    Some(old)
}

pub fn write(target: &str, secret: &str) -> windows::core::Result<()> {
    let mut name: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let mut user: Vec<u16> = zct_core::channel::CURRENT
        .dir
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut blob = secret.as_bytes().to_vec();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user.as_mut_ptr()),
        ..Default::default()
    };
    unsafe { CredWriteW(&cred, 0) }
}

/// 沒有存過時回傳 None
pub fn read(target: &str) -> Option<String> {
    unsafe {
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        CredReadW(&HSTRING::from(target), CRED_TYPE_GENERIC, None, &mut p).ok()?;
        let cred = &*p;
        let bytes =
            std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize)
                .to_vec();
        CredFree(p as *const _);
        String::from_utf8(bytes).ok()
    }
}

pub fn delete(target: &str) {
    unsafe {
        let _ = CredDeleteW(&HSTRING::from(target), CRED_TYPE_GENERIC, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_delete() {
        let target = format!("ZooType/test-{}", std::process::id());
        assert_eq!(read(&target), None);
        write(&target, "sk-測試-123").unwrap();
        assert_eq!(read(&target).as_deref(), Some("sk-測試-123"));
        delete(&target);
        assert_eq!(read(&target), None);
    }
}
