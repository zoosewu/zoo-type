//! Windows named pipe 傳輸（訊息模式）
//!
//! - 管道名稱含使用者 SID，不同使用者互不干擾
//! - 安全描述元允許 AppContainer（Store App、開始選單搜尋）與 Low integrity 程序連線（參考小狼毫）
//! - 用戶端每次呼叫都有逾時，逾時即視為斷線，絕不讓 App 卡住

use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, LocalFree, WAIT_TIMEOUT,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    SetNamedPipeHandleState, WaitNamedPipeW,
};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken};
use windows::core::{HSTRING, PWSTR};

use crate::{Error, INITIAL_BUFFER, MAX_MESSAGE, Request, Response, Result, decode, encode};

/// 擁有的 HANDLE，離開作用域自動關閉
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

// HANDLE 只是核心物件的代號，可以跨執行緒移動
unsafe impl Send for Owned {}

/// 目前程序使用者的 SID 字串，例如 `S-1-5-21-...`
pub fn current_user_sid() -> Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let token = Owned(token);

        let mut len = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);

        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s)?;
        let text = s.to_string().unwrap_or_default();
        LocalFree(Some(HLOCAL(s.0.cast())));
        Ok(text)
    }
}

/// 正式管道名稱
pub fn pipe_name() -> Result<HSTRING> {
    pipe_name_with("")
}

/// 加上後綴的管道名稱（測試用，避免與正在執行的服務程序衝突）
///
/// 名稱帶協定版本：新舊版本並存時，舊 DLL 連到舊服務程序、新 DLL 連到新服務程序
pub fn pipe_name_with(suffix: &str) -> Result<HSTRING> {
    Ok(HSTRING::from(format!(
        r"\\.\pipe\zct-v{}-{}{suffix}",
        crate::PROTOCOL_VERSION,
        current_user_sid()?
    )))
}

/// 服務程序端：接受連線
pub struct Listener {
    name: HSTRING,
    sddl: HSTRING,
    first: bool,
}

pub struct ServerConn {
    pipe: Owned,
    buf: Vec<u8>,
}

impl Listener {
    pub fn new() -> Result<Listener> {
        Listener::with_name(pipe_name()?)
    }

    pub fn with_name(name: HSTRING) -> Result<Listener> {
        let sid = current_user_sid()?;
        // 使用者本人與 SYSTEM：完整權限（建立更多管道實例需要 FILE_CREATE_PIPE_INSTANCE）
        // AC / S-1-15-2-2：所有 AppContainer 與 LPAC 程序可讀寫
        // S:(ML;;NW;;;LW)：允許 Low integrity 程序寫入
        let sddl = format!(
            "D:(A;;GA;;;SY)(A;;GA;;;{sid})(A;;GRGW;;;AC)(A;;GRGW;;;S-1-15-2-2)S:(ML;;NW;;;LW)"
        );
        Ok(Listener {
            name,
            sddl: HSTRING::from(sddl),
            first: true,
        })
    }

    /// 阻塞直到有用戶端連線
    pub fn accept(&mut self) -> Result<ServerConn> {
        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &self.sddl,
                SDDL_REVISION_1,
                &mut sd,
                None,
            )?;
            let sa = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd.0,
                bInheritHandle: false.into(),
            };
            let mut open_mode = PIPE_ACCESS_DUPLEX;
            if self.first {
                // 防止其他程序搶先建立同名管道冒充服務程序
                open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
            }
            let handle = CreateNamedPipeW(
                &self.name,
                open_mode,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                INITIAL_BUFFER as u32,
                INITIAL_BUFFER as u32,
                0,
                Some(&sa),
            );
            LocalFree(Some(HLOCAL(sd.0)));
            if handle.is_invalid() {
                return Err(windows::core::Error::from_thread().into());
            }
            self.first = false;
            let pipe = Owned(handle);
            match ConnectNamedPipe(pipe.0, None) {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
                Err(e) => return Err(e.into()),
            }
            Ok(ServerConn {
                pipe,
                buf: vec![0; INITIAL_BUFFER],
            })
        }
    }
}

impl ServerConn {
    /// 讀取一則請求；用戶端斷線時回傳 `Ok(None)`
    pub fn recv(&mut self) -> Result<Option<Request>> {
        let mut read = 0u32;
        match unsafe { ReadFile(self.pipe.0, Some(&mut self.buf), Some(&mut read), None) } {
            Ok(()) => decode(&self.buf[..read as usize]).map(Some),
            Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => {
                Err(Error::TooLarge(INITIAL_BUFFER))
            }
            // ERROR_BROKEN_PIPE 等：用戶端已離開
            Err(_) => Ok(None),
        }
    }

    pub fn send(&mut self, resp: &Response) -> Result<()> {
        let bytes = encode(resp)?;
        unsafe { WriteFile(self.pipe.0, Some(&bytes), None, None)? };
        Ok(())
    }
}

impl Drop for ServerConn {
    fn drop(&mut self) {
        unsafe {
            let _ = DisconnectNamedPipe(self.pipe.0);
        }
    }
}

/// DLL 端：同步呼叫，每次都有逾時
pub struct Client {
    pipe: Owned,
    event: Owned,
    buf: Vec<u8>,
}

impl Client {
    pub fn connect() -> Result<Client> {
        Client::connect_to(&pipe_name()?)
    }

    pub fn connect_to(name: &HSTRING) -> Result<Client> {
        unsafe {
            let open = || {
                CreateFileW(
                    name,
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED | FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                )
            };
            let handle = match open() {
                Ok(h) => h,
                Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                    // 所有實例都忙：短暫等待一次，不無限等待（小狼毫的無限等待曾造成卡死）
                    if !WaitNamedPipeW(name, 50).as_bool() {
                        return Err(Error::Timeout);
                    }
                    open()?
                }
                Err(e) => return Err(e.into()),
            };
            let pipe = Owned(handle);
            SetNamedPipeHandleState(pipe.0, Some(&PIPE_READMODE_MESSAGE), None, None)?;
            let event = Owned(CreateEventW(None, true, false, None)?);
            Ok(Client {
                pipe,
                event,
                buf: vec![0; INITIAL_BUFFER],
            })
        }
    }

    /// 等一個 overlapped 操作完成；回傳（讀到的位元組數, 訊息是否還有剩）
    ///
    /// 逾時會取消 I/O 並等待取消完成，確保核心不再寫入緩衝區
    unsafe fn finish(
        &self,
        ov: &OVERLAPPED,
        started: windows::core::Result<()>,
        ms: u32,
    ) -> Result<(usize, bool)> {
        let more = |e: &windows::core::Error| e.code() == ERROR_MORE_DATA.to_hresult();
        let mut n = 0u32;
        unsafe {
            match started {
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    match GetOverlappedResultEx(self.pipe.0, ov, &mut n, ms, false) {
                        Ok(()) => Ok((n as usize, false)),
                        Err(e) if more(&e) => Ok((n as usize, true)),
                        Err(e)
                            if e.code() == windows::core::HRESULT::from_win32(WAIT_TIMEOUT.0) =>
                        {
                            let _ = CancelIoEx(self.pipe.0, Some(ov));
                            let _ = GetOverlappedResult(self.pipe.0, ov, &mut n, true);
                            Err(Error::Timeout)
                        }
                        Err(e) => Err(e.into()),
                    }
                }
                // 立即完成（成功或還有剩）：從 OVERLAPPED 取得實際讀到的位元組數
                Ok(()) => {
                    let r = GetOverlappedResult(self.pipe.0, ov, &mut n, false);
                    Ok((n as usize, r.as_ref().is_err_and(more)))
                }
                Err(e) if more(&e) => {
                    let _ = GetOverlappedResult(self.pipe.0, ov, &mut n, false);
                    Ok((n as usize, true))
                }
                Err(e) => Err(e.into()),
            }
        }
    }

    /// 送出請求並等待回應；逾時回傳 `Error::Timeout`（呼叫端應丟棄這個連線）
    ///
    /// 回應超過緩衝區時（`ERROR_MORE_DATA`）會擴充緩衝區繼續讀，直到 `MAX_MESSAGE`
    pub fn call(&mut self, req: &Request, timeout: Duration) -> Result<Response> {
        let bytes = encode(req)?;
        let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        let new_ov = |event: HANDLE| OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        unsafe {
            let ov = new_ov(self.event.0);
            let started = windows::Win32::System::Pipes::TransactNamedPipe(
                self.pipe.0,
                Some(bytes.as_ptr().cast()),
                bytes.len() as u32,
                Some(self.buf.as_mut_ptr().cast()),
                self.buf.len() as u32,
                std::ptr::null_mut(),
                Some(&ov as *const _ as *mut _),
            );
            let (mut total, mut more) = self.finish(&ov, started, ms)?;
            while more {
                if total >= self.buf.len() {
                    if self.buf.len() >= MAX_MESSAGE {
                        return Err(Error::TooLarge(MAX_MESSAGE));
                    }
                    let grown = (self.buf.len() * 2).min(MAX_MESSAGE);
                    self.buf.resize(grown, 0);
                }
                let ov = new_ov(self.event.0);
                let started = ReadFile(
                    self.pipe.0,
                    Some(&mut self.buf[total..]),
                    None,
                    Some(&ov as *const _ as *mut _),
                );
                let (n, m) = self.finish(&ov, started, ms)?;
                total += n;
                more = m;
            }
            let resp = decode(&self.buf[..total]);
            // 讀過大型回應後縮回，避免每個 App 都長期佔用大緩衝區
            if self.buf.len() > INITIAL_BUFFER {
                self.buf.truncate(INITIAL_BUFFER);
                self.buf.shrink_to_fit();
            }
            resp
        }
    }
}
