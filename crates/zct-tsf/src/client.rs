//! 與服務程序的連線：斷線自動重連、必要時啟動服務程序，任何失敗都回傳 None（按鍵放行）

use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation, TokenIsAppContainer,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use zct_ipc::pipe::Client;
use zct_ipc::{PROTOCOL_VERSION, Request, Response};

use crate::detect::Heuristics;
use crate::dll_instance;

/// 單次呼叫上限：超過就放行按鍵，絕不讓 App 卡住
const CALL_TIMEOUT: Duration = Duration::from_millis(100);
/// 連線失敗後多久再試
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// 啟動服務程序後多久內不重複啟動
const SPAWN_COOLDOWN: Duration = Duration::from_secs(10);
/// 剛啟動服務程序時，在這次按鍵裡等它就緒的上限（平常零點幾秒就好）；等到了這個鍵就不會漏給 App
const SPAWN_WAIT: Duration = Duration::from_millis(1500);
const SPAWN_POLL: Duration = Duration::from_millis(50);

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

#[derive(Default)]
pub struct Ipc {
    client: Option<Client>,
    /// 服務程序在 Hello 時告知（依設定與 App 規則）
    heuristics: Heuristics,
    retry_at: Option<Instant>,
    spawned_at: Option<Instant>,
}

impl Ipc {
    pub fn call(&mut self, req: &Request) -> Option<Response> {
        if self.client.is_none() && !self.connect() {
            return None;
        }
        let client = self.client.as_mut()?;
        match client.call(req, CALL_TIMEOUT) {
            Ok(r) => Some(r),
            Err(_) => {
                self.client = None;
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
                None
            }
        }
    }

    /// 這個 App 要做的啟發式偵測；尚未連線時先嘗試連線，連不上就都不做
    pub fn heuristics(&mut self) -> Heuristics {
        if self.client.is_none() && !self.connect() {
            return Heuristics::default();
        }
        self.heuristics
    }

    pub fn disconnect(&mut self) {
        self.client = None;
    }

    fn connect(&mut self) -> bool {
        if self.retry_at.is_some_and(|t| Instant::now() < t) {
            return false;
        }
        let mut found = try_connect();
        // 連不上：啟動服務程序並等它就緒，這次的按鍵才不會漏給 App
        if found.is_none() && self.spawn_service() {
            let deadline = Instant::now() + SPAWN_WAIT;
            while found.is_none() && Instant::now() < deadline {
                std::thread::sleep(SPAWN_POLL);
                found = try_connect();
            }
        }
        match found {
            Some((c, heuristics)) => {
                self.client = Some(c);
                self.heuristics = heuristics;
                self.retry_at = None;
                true
            }
            None => {
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
                false
            }
        }
    }

    /// 服務程序沒在跑時，從一般權限的桌面程式啟動它
    ///
    /// 不從 AppContainer（沙箱）或提權程序啟動：子程序會繼承受限 / 管理員權限。回傳是否啟動了
    fn spawn_service(&mut self) -> bool {
        if self
            .spawned_at
            .is_some_and(|t| t.elapsed() < SPAWN_COOLDOWN)
        {
            return false;
        }
        if is_app_container() || is_elevated() {
            return false;
        }
        let Some(dir) = dll_dir() else { return false };
        let exe = dir.join("zoo-type-service.exe");
        if !exe.exists() {
            return false;
        }
        self.spawned_at = Some(Instant::now());
        std::process::Command::new(exe)
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()
            .is_ok()
    }
}

/// 連線並打招呼；服務程序告知這個 App 要做的啟發式偵測
fn try_connect() -> Option<(Client, Heuristics)> {
    let mut c = Client::connect().ok()?;
    let hello = Request::Hello {
        version: PROTOCOL_VERSION,
        pid: std::process::id(),
        exe: exe_name(),
    };
    match c.call(&hello, Duration::from_millis(500)) {
        Ok(Response::Welcome {
            detect_legacy,
            detect_fullscreen,
        }) => Some((
            c,
            Heuristics {
                legacy_fullscreen: detect_legacy,
                exclusive_fullscreen: detect_fullscreen,
            },
        )),
        _ => None,
    }
}

fn exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

fn dll_dir() -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(dll_instance().into()), &mut buf) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let path = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n]));
    path.parent().map(|p| p.to_path_buf())
}

fn token_u32(class: windows::Win32::Security::TOKEN_INFORMATION_CLASS) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut value = 0u32;
        let mut len = 0u32;
        let r = GetTokenInformation(
            token,
            class,
            Some(&mut value as *mut u32 as *mut _),
            size_of::<u32>() as u32,
            &mut len,
        );
        let _ = windows::Win32::Foundation::CloseHandle(token);
        r.ok().map(|_| value)
    }
}

fn is_app_container() -> bool {
    token_u32(TokenIsAppContainer).is_none_or(|v| v != 0)
}

fn is_elevated() -> bool {
    debug_assert_eq!(size_of::<TOKEN_ELEVATION>(), size_of::<u32>());
    token_u32(TokenElevation).is_none_or(|v| v != 0)
}
