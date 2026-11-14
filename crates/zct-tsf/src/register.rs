//! `regsvr32` 註冊 / 解除註冊（需要系統管理員權限，寫入 HKLM）

use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    GUID_TFCAT_TIP_KEYBOARD, GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    ITfCategoryMgr, ITfInputProcessorProfileMgr, ITfInputProcessorProfiles,
};
use windows::core::{GUID, HSTRING, Interface, Result};

use crate::{CLSID_ZCT, LANGID_ZH_TW, PROFILE_ZCT, dll_instance};

/// 輸入法名稱（Win + 空白鍵清單、設定 → 語言）
const DESCRIPTION: &str = zct_core::channel::CURRENT.name;

const CATEGORIES: [GUID; 4] = [
    GUID_TFCAT_TIP_KEYBOARD,
    // 組字區底線與選字目標標示
    GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    // Windows 8+ 的沙箱 App（Store App、開始選單）
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn clsid_key() -> String {
    format!(r"Software\Classes\CLSID\{{{:?}}}", CLSID_ZCT)
}

fn dll_path() -> String {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(dll_instance().into()), &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// 圖示檔：安裝根目錄的 `zoo-type-icon.dll`（install.ps1 複製的，路徑不隨版本改變）；沒有時用 DLL 本身
///
/// 工作列會記住語言設定檔的圖示路徑，更新換了版本資料夾、舊資料夾刪掉後就讀不到，改顯示「繁體」，
/// 要重啟 explorer 才會重讀。固定路徑就不會發生。
fn icon_path() -> String {
    let dll = dll_path();
    std::path::Path::new(&dll)
        .parent()
        .and_then(std::path::Path::parent)
        .map(|base| base.join("zoo-type-icon.dll"))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(dll)
}

fn check(e: WIN32_ERROR) -> Result<()> {
    if e == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(e.to_hresult().into())
    }
}

fn set_value(key: HKEY, name: Option<&str>, value: &str) -> Result<()> {
    let mut data: Vec<u8> = wide(value)
        .iter()
        .chain(std::iter::once(&0))
        .flat_map(|c| c.to_le_bytes())
        .collect();
    data.shrink_to_fit();
    unsafe {
        let name = name.map(HSTRING::from).unwrap_or_default();
        check(RegSetValueExW(key, &name, None, REG_SZ, Some(&data)))
    }
}

fn create_key(path: &str) -> Result<HKEY> {
    let mut key = HKEY::default();
    unsafe {
        check(RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        ))?;
    }
    Ok(key)
}

fn register_com() -> Result<()> {
    let root = create_key(&clsid_key())?;
    let r = set_value(root, None, DESCRIPTION);
    unsafe {
        let _ = RegCloseKey(root);
    }
    r?;
    let inproc = create_key(&format!(r"{}\InprocServer32", clsid_key()))?;
    let r = set_value(inproc, None, &dll_path())
        .and_then(|_| set_value(inproc, Some("ThreadingModel"), "Apartment"));
    unsafe {
        let _ = RegCloseKey(inproc);
    }
    r
}

pub fn register() -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        register_com()?;

        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        // 圖示：DLL 內第 0 個圖示資源（白底黑字「動」，見 build.rs）；
        // 工作列只認 DLL / EXE 的圖示資源，讀不到時會改顯示語言簡稱「繁體」
        let icon = wide(&icon_path());
        profiles.RegisterProfile(
            &CLSID_ZCT,
            LANGID_ZH_TW,
            &PROFILE_ZCT,
            &wide(DESCRIPTION),
            &icon,
            0,
            HKL::default(),
            0,
            true,
            0,
        )?;
        // 設為中文（台灣）的預設輸入法：Alt+Shift 切回中文時用的是這個，不是清單順序
        // （寫在目前使用者的 HKCU；失敗不影響註冊）。只有正式版：開發版只在開發時手動切過去用
        let official = zct_core::channel::CURRENT.id == zct_core::channel::OFFICIAL.id;
        if official && let Ok(old) = profiles.cast::<ITfInputProcessorProfiles>() {
            let _ = old.SetDefaultLanguageProfile(LANGID_ZH_TW, &CLSID_ZCT, &PROFILE_ZCT);
        }

        let cats: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for cat in &CATEGORIES {
            cats.RegisterCategory(&CLSID_ZCT, cat, &CLSID_ZCT)?;
        }
    }
    Ok(())
}

/// 盡量清乾淨：任何一步失敗都繼續做下一步
pub fn unregister() -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(cats) =
            CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)
        {
            for cat in &CATEGORIES {
                let _ = cats.UnregisterCategory(&CLSID_ZCT, cat, &CLSID_ZCT);
            }
        }
        if let Ok(profiles) = CoCreateInstance::<_, ITfInputProcessorProfileMgr>(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            let _ = profiles.UnregisterProfile(&CLSID_ZCT, LANGID_ZH_TW, &PROFILE_ZCT, 0);
        }
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(clsid_key()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn clsid_key_uses_registry_format() {
        assert_eq!(
            super::clsid_key(),
            r"Software\Classes\CLSID\{68E048CC-2635-4A97-8A2F-1599AF3AFCF5}"
        );
    }
}
