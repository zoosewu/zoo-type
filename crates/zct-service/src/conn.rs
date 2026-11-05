//! 一條 DLL 連線 = 一個 App 執行緒 = 一個輸入 session

use std::sync::atomic::Ordering;

use zct_core::{InputMode, KeyResult, Output, Session, SessionOptions, Width};
use zct_ipc::pipe::ServerConn;
use zct_ipc::{PROTOCOL_VERSION, Request, Response};

use crate::app::Shared;
use crate::ui::UiCmd;

/// 狀態提示文字；同時發生多個變化時，只顯示最重要的一個
fn hint(r: &KeyResult, game: Option<bool>) -> Option<String> {
    if let Some(on) = game {
        return Some(if on { "遊戲模式" } else { "一般模式" }.into());
    }
    if let Some(n) = &r.notice {
        return Some(n.clone());
    }
    static_hint(r).map(str::to_owned)
}

fn static_hint(r: &KeyResult) -> Option<&'static str> {
    if let Some(m) = r.mode_changed {
        return Some(match m {
            InputMode::Chinese => "中",
            InputMode::English => "A",
        });
    }
    if let Some(w) = r.width_changed {
        return Some(match w {
            Width::Full => "全形",
            Width::Half => "半形",
        });
    }
    r.output_changed.map(|o| match o {
        Output::Traditional => "繁",
        Output::Simplified => "简",
    })
}

fn open_session(id: u64, exe: &str, shared: &Shared) -> Result<Session, String> {
    let (config, apps) = shared.settings.current();
    let rule = apps.find(exe).cloned();
    if let Some(r) = &rule {
        log::info!("[{id}] 套用 App 規則「{}」", if r.name.is_empty() { exe } else { &r.name });
    }
    Session::new(&SessionOptions {
        engine: shared.engine.clone(),
        config,
        rule,
    })
    .map_err(|e| e.to_string())
}

pub fn serve(id: u64, mut conn: ServerConn, shared: &Shared) {
    let mut session: Option<Session> = None;
    // 最近一次按鍵結果與狀態提示，等 DLL 回報游標位置後再決定候選窗怎麼顯示
    let mut last: Option<(KeyResult, Option<String>)> = None;

    loop {
        let req = match conn.recv() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => {
                log::warn!("[{id}] 讀取失敗：{e}");
                break;
            }
        };
        if let Some(s) = session.as_mut() {
            s.set_global_game(shared.game.load(Ordering::Relaxed));
        }
        let resp = match req {
            Request::Hello { version, pid, exe } => {
                if version != PROTOCOL_VERSION {
                    log::warn!("[{id}] 協定版本不符：{version}（{exe}）");
                    Response::Error(format!("protocol {PROTOCOL_VERSION} expected"))
                } else {
                    log::info!("[{id}] 連線：{exe}（pid {pid}）");
                    match open_session(id, &exe, shared) {
                        Ok(s) => {
                            let (detect_legacy, detect_fullscreen) = s.detection();
                            session = Some(s);
                            Response::Welcome {
                                detect_legacy,
                                detect_fullscreen,
                            }
                        }
                        Err(e) => {
                            log::error!("[{id}] {e}");
                            Response::Error(e)
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
                    let game = r.toggle_game.then(|| {
                        let on = !shared.game.fetch_xor(true, Ordering::Relaxed);
                        s.set_global_game(on);
                        log::info!("[{id}] 遊戲模式：{}", if on { "開" } else { "關" });
                        on
                    });
                    last = Some((r.clone(), hint(&r, game)));
                    Response::Key(r)
                }
                None => Response::Key(KeyResult::default()),
            },
            Request::Caret(rect) => {
                if let Some((r, hint)) = last.take() {
                    match r.candidates {
                        Some(page) => shared.ui.send(UiCmd::Show {
                            owner: id,
                            page,
                            anchor: rect,
                        }),
                        None => shared.ui.send(UiCmd::Hide { owner: id }),
                    }
                    if let Some(text) = hint {
                        shared.ui.send(UiCmd::Hint { text, anchor: rect });
                    }
                }
                Response::Ok
            }
            Request::Reset | Request::Focus => {
                if let Some(s) = session.as_mut() {
                    if req == Request::Focus {
                        s.focus();
                    } else {
                        s.reset();
                    }
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
    // App 關閉時順便存檔，縮短當機時遺失學習的時間窗
    if let Err(e) = shared.engine.save() {
        log::error!("學習紀錄存檔失敗：{e}");
    }
    log::debug!("[{id}] 斷線");
}
