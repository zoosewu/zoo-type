//! 開發用：真實 TSF 環境的整合測試工具
//!
//! 建立一個含標準編輯框的視窗、對這個程序啟用「已安裝的」ZCT，
//! 以 SendInput 模擬鍵盤（與實體鍵盤相同路徑），最後印出編輯框內容。
//!
//! cargo run -p zct-tsf --example tsf_probe -- "wu0 fu4" "{enter}" "{ctrl+alt+e}"
//! 環境變數 ZCT_PROBE_RICH=1：改用 RichEdit（Windows 11 記事本的控制項，有 Ctrl+Alt+E = € 等內建快捷鍵）
//! 按鍵：一般字元依美式鍵盤；{enter} {space} {esc} {bs} {left} {right} {down} {shift} {ctrl+shift+;} …

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr, TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE,
    TF_IPPMF_FORPROCESS, TF_PROFILETYPE_INPUTPROCESSOR,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{GUID, w};

const CLSID_ZCT: GUID = GUID::from_u128(0x68e048cc_2635_4a97_8a2f_1599af3afcf5);
const PROFILE_ZCT: GUID = GUID::from_u128(0xd890a24e_551e_4d5f_a0f1_745be13b6287);

extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}

fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    unsafe {
        let mut msg = MSG::default();
        while Instant::now() < end {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                ..Default::default()
            },
        },
    }
}

fn vk_of(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name {
        "enter" => VK_RETURN,
        "space" => VK_SPACE,
        "tab" => VK_TAB,
        "esc" => VK_ESCAPE,
        "bs" => VK_BACK,
        "del" => VK_DELETE,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "home" => VK_HOME,
        "end" => VK_END,
        "shift" => VK_LSHIFT,
        ";" => VK_OEM_1,
        "'" => VK_OEM_7,
        "," => VK_OEM_COMMA,
        "." => VK_OEM_PERIOD,
        "/" => VK_OEM_2,
        "-" => VK_OEM_MINUS,
        c if c.len() == 1 => {
            let b = c.as_bytes()[0].to_ascii_uppercase();
            if b.is_ascii_alphanumeric() {
                VIRTUAL_KEY(b as u16)
            } else {
                return None;
            }
        }
        // F1–F24
        f if f.starts_with('f') => match f[1..].parse::<u16>() {
            Ok(n @ 1..=24) => VIRTUAL_KEY(VK_F1.0 + n - 1),
            _ => return None,
        },
        _ => return None,
    })
}

static PROBE_WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 安全檢查：SendInput 會送到「前景視窗」，前景不是測試視窗時立刻停止，絕不把按鍵送進使用者的視窗
fn ensure_foreground() {
    let ours = PROBE_WINDOW.load(std::sync::atomic::Ordering::Relaxed);
    let fg = unsafe { GetForegroundWindow() }.0 as isize;
    if fg != ours {
        eprintln!("中止：測試視窗不在前景，為避免按鍵送進其他視窗而停止");
        std::process::exit(2);
    }
}

/// 送出一個按鍵（可含修飾鍵），每個事件之間讓訊息迴圈跑一下
fn press(combo: &str) {
    let parts: Vec<&str> = combo.split('+').collect();
    let (mods, main) = if combo.ends_with("++") {
        (parts[..parts.len() - 2].to_vec(), "+")
    } else {
        (parts[..parts.len() - 1].to_vec(), parts[parts.len() - 1])
    };
    let mod_vks: Vec<VIRTUAL_KEY> = mods
        .iter()
        .map(|m| match *m {
            "ctrl" => VK_LCONTROL,
            "alt" => VK_LMENU,
            "shift" => VK_LSHIFT,
            other => panic!("未知修飾鍵 {other}"),
        })
        .collect();
    let main_vk = vk_of(main).unwrap_or_else(|| panic!("未知按鍵 {main}"));
    let mut seq: Vec<INPUT> = mod_vks.iter().map(|&v| key(v, false)).collect();
    seq.push(key(main_vk, false));
    seq.push(key(main_vk, true));
    seq.extend(mod_vks.iter().rev().map(|&v| key(v, true)));
    for i in seq {
        ensure_foreground();
        unsafe { SendInput(&[i], size_of::<INPUT>() as i32) };
        pump(30);
    }
}

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let hinst = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: w!("ZctProbe"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let main = CreateWindowExW(
            Default::default(),
            w!("ZctProbe"),
            w!("ZCT TSF Probe"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            200,
            200,
            700,
            200,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .unwrap();
        let rich = std::env::var_os("ZCT_PROBE_RICH").is_some();
        if rich {
            LoadLibraryW(w!("Msftedit.dll")).unwrap();
        }
        let edit = CreateWindowExW(
            Default::default(),
            if rich { w!("RICHEDIT50W") } else { w!("EDIT") },
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(ES_MULTILINE as u32),
            10,
            10,
            660,
            120,
            Some(main),
            None,
            Some(hinst.into()),
            None,
        )
        .unwrap();

        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER).unwrap();
        profiles
            .ActivateProfile(
                TF_PROFILETYPE_INPUTPROCESSOR,
                0x0404,
                &CLSID_ZCT,
                &PROFILE_ZCT,
                HKL::default(),
                TF_IPPMF_FORPROCESS | TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE,
            )
            .expect("無法啟用 ZCT（是否已安裝？）");
        // 背景程式不能直接搶前景：暫時與目前前景執行緒共用輸入狀態後再設定（不送出任何按鍵）
        PROBE_WINDOW.store(main.0 as isize, std::sync::atomic::Ordering::Relaxed);
        let fg = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg, None);
        let me = windows::Win32::System::Threading::GetCurrentThreadId();
        let attached = fg_thread != 0 && fg_thread != me && windows::Win32::System::Threading::AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(main);
        let _ = SetForegroundWindow(main);
        let _ = SetFocus(Some(edit));
        if attached {
            let _ = windows::Win32::System::Threading::AttachThreadInput(me, fg_thread, false);
        }
        pump(800);
        ensure_foreground();

        for arg in std::env::args().skip(1) {
            let mut rest = arg.as_str();
            while !rest.is_empty() {
                if let Some(end) = rest.strip_prefix('{').and_then(|r| r.find('}')) {
                    press(&rest[1..=end]);
                    rest = &rest[end + 2..];
                } else {
                    let c = &rest[..1];
                    press(if c == " " { "space" } else { c });
                    rest = &rest[1..];
                }
            }
            pump(300);
        }
        pump(500);
        let mut buf = [0u16; 1024];
        let n = GetWindowTextW(edit, &mut buf) as usize;
        println!("編輯框內容：{:?}", String::from_utf16_lossy(&buf[..n]));
    }
}
