//! 一條 DLL 連線 = 一個 App 執行緒 = 一個輸入 session

use zct_core::{InputMode, KeyResult, Session, SessionOptions, Width};
use zct_ipc::pipe::ServerConn;
use zct_ipc::{PROTOCOL_VERSION, Request, Response};

use crate::app::Shared;
use crate::ui::UiCmd;

pub fn serve(id: u64, mut conn: ServerConn, shared: &Shared) {
    let mut session: Option<Session> = None;
    // 最近一次按鍵結果，等 DLL 回報游標位置後再決定候選窗怎麼顯示
    let mut last: Option<KeyResult> = None;

    loop {
        let req = match conn.recv() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => {
                log::warn!("[{id}] 讀取失敗：{e}");
                break;
            }
        };
        let resp = match req {
            Request::Hello { version, pid, exe } => {
                if version != PROTOCOL_VERSION {
                    log::warn!("[{id}] 協定版本不符：{version}（{exe}）");
                    Response::Error(format!("protocol {PROTOCOL_VERSION} expected"))
                } else {
                    log::info!("[{id}] 連線：{exe}（pid {pid}）");
                    match Session::new(&SessionOptions {
                        sys_dict_dir: shared.sys_dict_dir.clone(),
                        user_dir: shared.data.chewing_user_dir(),
                        config: shared.config.clone(),
                    }) {
                        Ok(s) => {
                            session = Some(s);
                            Response::Ok
                        }
                        Err(e) => {
                            log::error!("[{id}] {e}");
                            Response::Error(e.to_string())
                        }
                    }
                }
            }
            Request::TestKeyDown(ev) => match session.as_mut() {
                Some(s) => Response::Eat(s.test_key_down(&ev)),
                None => Response::Eat(false),
            },
            Request::Key(ev) => match session.as_mut() {
                Some(s) => {
                    let r = s.process_key(&ev);
                    last = Some(r.clone());
                    Response::Key(r)
                }
                None => Response::Key(KeyResult::default()),
            },
            Request::Caret(rect) => {
                if let Some(r) = last.take() {
                    match r.candidates {
                        Some(page) => shared.ui.send(UiCmd::Show {
                            owner: id,
                            page,
                            anchor: rect,
                        }),
                        None => shared.ui.send(UiCmd::Hide { owner: id }),
                    }
                    let hint = match (r.mode_changed, r.width_changed) {
                        (Some(InputMode::Chinese), _) => Some("中"),
                        (Some(InputMode::English), _) => Some("A"),
                        (None, Some(Width::Full)) => Some("全形"),
                        (None, Some(Width::Half)) => Some("半形"),
                        (None, None) => None,
                    };
                    if let Some(text) = hint {
                        shared.ui.send(UiCmd::Hint { text, anchor: rect });
                    }
                }
                Response::Ok
            }
            Request::Reset => {
                if let Some(s) = session.as_mut() {
                    s.reset();
                }
                last = None;
                shared.ui.send(UiCmd::Hide { owner: id });
                Response::Ok
            }
        };
        if let Err(e) = conn.send(&resp) {
            log::warn!("[{id}] 回應失敗：{e}");
            break;
        }
    }
    shared.ui.send(UiCmd::Hide { owner: id });
    log::debug!("[{id}] 斷線");
}
