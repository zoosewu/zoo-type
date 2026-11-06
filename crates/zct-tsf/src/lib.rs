//! ZCT 的 Windows TSF 文字服務（薄層）
//!
//! 只負責：接收按鍵 → 轉送給服務程序 → 依結果更新組字 / 送出文字 → 回報游標位置。
//! 所有判斷都在 zct-core（服務程序內），這裡不做任何輸入法邏輯。

#![cfg(windows)]

mod client;
mod detect;
mod display;
mod edit;
mod factory;
mod keys;
mod langbar;
mod register;
mod text_service;

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, E_FAIL, E_POINTER, HINSTANCE, S_FALSE,
};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::core::{BOOL, GUID, HRESULT, Interface};

/// TSF 文字服務的 CLSID
pub const CLSID_ZCT: GUID = GUID::from_u128(0x68e048cc_2635_4a97_8a2f_1599af3afcf5);
/// 語言設定檔 GUID
pub const PROFILE_ZCT: GUID = GUID::from_u128(0xd890a24e_551e_4d5f_a0f1_745be13b6287);
/// zh-TW
pub const LANGID_ZH_TW: u16 = 0x0404;

static DLL_INSTANCE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) fn dll_instance() -> HINSTANCE {
    HINSTANCE(DLL_INSTANCE.load(Ordering::Relaxed))
}

/// 每個 COM 回呼都要經過這裡：panic 不能跨過 FFI 邊界（會直接讓 App 當掉）
pub(crate) fn guard<T>(f: impl FnOnce() -> windows::core::Result<T>) -> windows::core::Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err(E_FAIL.into()),
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        DLL_INSTANCE.store(hinst.0, Ordering::Relaxed);
    }
    true.into()
}

#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    guard(|| unsafe {
        if ppv.is_null() || rclsid.is_null() {
            return Err(E_POINTER.into());
        }
        *ppv = std::ptr::null_mut();
        if *rclsid != CLSID_ZCT {
            return Err(CLASS_E_CLASSNOTAVAILABLE.into());
        }
        let factory: IClassFactory = factory::Factory.into();
        factory.query(riid, ppv).ok()
    })
    .into()
}

/// 保持載入：TSF 會頻繁建立 / 釋放物件，卸載再載入沒有好處
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[unsafe(no_mangle)]
extern "system" fn DllRegisterServer() -> HRESULT {
    guard(register::register).into()
}

#[unsafe(no_mangle)]
extern "system" fn DllUnregisterServer() -> HRESULT {
    guard(register::unregister).into()
}
