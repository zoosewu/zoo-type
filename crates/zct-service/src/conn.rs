//! 一條 DLL 連線 = 一個 App 執行緒 = 一個輸入 session

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zct_core::CandidatePage;

use zct_core::{InputMode, KeyResult, Output, Session, SessionOptions, Width};
use zct_ipc::pipe::ServerConn;
use zct_ipc::{Command, PROTOCOL_VERSION, Request, Response};

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
        Output::Traditional => "動",
        Output::Simplified => "动",
    })
}

/// 按鍵診斷：只記錄非字元鍵，絕不記錄字母、數字與符號
fn debug_key(shared: &Shared, id: u64, stage: &str, ev: &zct_core::KeyEvent, result: &str) {
    use zct_core::Key;
    if !shared.debug_keys || matches!(ev.key, Key::Char(_) | Key::Numpad(_)) {
        return;
    }
    log::info!(
        "[{id}] 按鍵診斷 {stage} {:?} {} shift={} ctrl={} alt={} t={} {result}",
        ev.key,
        if ev.released { "放開" } else { "按下" },
        ev.mods.shift,
        ev.mods.ctrl,
        ev.mods.alt,
        ev.time_ms,
    );
}

/// 預設開啟組字提示列的純終端機（App 自己不顯示或常被 TUI 藏起組字）
const TERMINALS: &[&str] = &[
    "WindowsTerminal.exe",
    "OpenConsole.exe",
    "conhost.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "mintty.exe",
];

/// 這個 App 要不要顯示組字提示列：App 規則 > 純終端機 > 全域設定
fn composition_bar_for(exe: &str, shared: &Shared) -> bool {
    let (config, apps) = shared.settings.current();
    apps.find(exe)
        .and_then(|r| r.composition_bar)
        .unwrap_or_else(|| TERMINALS.iter().any(|t| t.eq_ignore_ascii_case(exe)) || config.theme.composition_bar)
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

/// 背景 AI 請求的結果：（原文, 修正結果或錯誤）
type AiMailbox = Arc<Mutex<Option<(String, Result<String, String>)>>>;

/// 在背景呼叫 AI；結果放進信箱，並直接更新候選窗（不必等下一次按鍵）
fn spawn_ai(id: u64, original: String, shared: &Shared, mailbox: AiMailbox) {
    let cfg = shared.settings.current().0.ai;
    let ui = shared.ui.clone();
    let endpoint = zct_ai::Endpoint {
        base_url: cfg.endpoint.clone(),
        model: cfg.model.clone(),
        api_key: zct_ipc::secret::read_ai_key(),
        timeout: Duration::from_secs(cfg.timeout_seconds.max(1)),
    };
    std::thread::spawn(move || {
        let result = zct_ai::correct(&endpoint, &original).map_err(|e| e.to_string());
        match &result {
            Ok(fixed) if *fixed != original => ui.send(UiCmd::Show {
                owner: id,
                page: CandidatePage {
                    items: vec![fixed.clone()],
                    page: 0,
                    total_pages: 1,
                    highlight: 0,
                    pinned: vec![false],
                    ..Default::default()
                },
                anchor: None,
            }),
            Ok(_) => ui.send(UiCmd::Hint {
                text: "AI：沒有發現錯字".into(),
                anchor: None,
            }),
            Err(e) => {
                log::warn!("[{id}] {e}");
                ui.send(UiCmd::Hint {
                    text: "AI 修正失敗".into(),
                    anchor: None,
                });
            }
        }
        if let Ok(mut m) = mailbox.lock() {
            *m = Some((original, result));
        }
    });
}

/// 輸入法圖示右鍵選單
fn run_command(cmd: Command, shared: &Shared) {
    match cmd {
        Command::OpenSettings => {
            let exe = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("zoo-type-settings.exe")));
            match exe.filter(|p| p.exists()) {
                Some(exe) => {
                    if let Err(e) = std::process::Command::new(exe).spawn() {
                        log::warn!("無法開啟設定程式：{e}");
                    }
                }
                None => log::warn!("找不到設定程式 zoo-type-settings.exe"),
            }
        }
        Command::ToggleGame => {
            let on = !shared.game.fetch_xor(true, Ordering::Relaxed);
            log::info!("輸入法圖示：遊戲模式{}", if on { "開" } else { "關" });
        }
        Command::Restart => {
            log::info!("輸入法圖示：重新啟動");
            let _ = shared.engine.save();
            shared.ui.send(UiCmd::Exit);
        }
    }
}

pub fn serve(id: u64, mut conn: ServerConn, shared: &Shared) {
    let _active = crate::version::ActiveGuard::new();
    let mut session: Option<Session> = None;
    let mailbox: AiMailbox = Arc::new(Mutex::new(None));
    // 最近一次按鍵結果與狀態提示，等 DLL 回報游標位置後再決定候選窗怎麼顯示
    let mut last: Option<(KeyResult, Option<String>)> = None;
    // 這個連線的 App 與組字提示列設定（連線時決定，快捷鍵切換）
    let mut app = String::new();
    let mut bar = false;
    // 組字提示列的位置：組字開始時的游標位置，組字結束前不跟著游標移動
    let mut bar_anchor: Option<zct_ipc::Rect> = None;

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
            // AI 結果回來了：交給 session，接下來的 Enter / 1 就能採用
            if let Some((original, Ok(fixed))) = mailbox.lock().ok().and_then(|mut m| m.take()) {
                s.set_ai_suggestion(&original, fixed);
            }
        }
        let resp = match req {
            Request::Hello { version, pid, exe } => {
                if version != PROTOCOL_VERSION {
                    log::warn!("[{id}] 協定版本不符：{version}（{exe}）");
                    Response::Error(format!("protocol {PROTOCOL_VERSION} expected"))
                } else {
                    log::info!("[{id}] 連線：{exe}（pid {pid}）");
                    bar = composition_bar_for(&exe, shared);
                    app = exe.clone();
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
                Some(s) => {
                    let eat = s.test_key_down(&ev);
                    debug_key(shared, id, "test", &ev, &format!("eat={eat}"));
                    Response::Eat(eat)
                }
                None => Response::Eat(false),
            },
            Request::Key(ev) => match session.as_mut() {
                Some(s) => {
                    let r = s.process_key(&ev);
                    debug_key(
                        shared,
                        id,
                        "key",
                        &ev,
                        &format!(
                            "eaten={} mode={:?} composing={}",
                            r.eaten,
                            r.mode_changed,
                            !r.preedit.text.is_empty()
                        ),
                    );
                    if r.bell {
                        // 系統預設提示音；非同步播放，不會卡住按鍵處理
                        unsafe {
                            let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
                                windows::Win32::UI::WindowsAndMessaging::MB_OK,
                            );
                        }
                    }
                    if let Some(text) = &r.ai_request {
                        spawn_ai(id, text.clone(), shared, mailbox.clone());
                    }
                    let game = r.toggle_game.then(|| {
                        let on = !shared.game.fetch_xor(true, Ordering::Relaxed);
                        s.set_global_game(on);
                        log::info!("[{id}] 遊戲模式：{}", if on { "開" } else { "關" });
                        on
                    });
                    let mut tip = hint(&r, game);
                    if r.toggle_composition_bar {
                        bar = !bar;
                        if let Err(e) = shared.settings.set_composition_bar(&app, bar) {
                            log::warn!("[{id}] 寫入 App 規則失敗：{e}");
                        }
                        log::info!("[{id}] 組字提示列：{}（{app}）", if bar { "開" } else { "關" });
                        tip = Some(format!("組字提示列：{}", if bar { "開" } else { "關" }));
                    }
                    last = Some((r.clone(), tip));
                    Response::Key(r)
                }
                None => Response::Key(KeyResult::default()),
            },
            Request::Caret(rect) => {
                if let Some((r, hint)) = last.take() {
                    if r.preedit.text.is_empty() {
                        bar_anchor = None;
                    } else if bar_anchor.is_none() {
                        bar_anchor = rect;
                    }
                    shared.ui.send(UiCmd::Composition {
                        owner: id,
                        preedit: bar.then(|| r.preedit.clone()),
                        anchor: bar_anchor.or(rect),
                    });
                    match r.candidates {
                        Some(page) => shared.ui.send(UiCmd::Show {
                            owner: id,
                            page,
                            anchor: rect,
                        }),
                        None => shared.ui.send(UiCmd::HideCandidates { owner: id }),
                    }
                    if let Some(text) = hint {
                        shared.ui.send(UiCmd::Hint { text, anchor: rect });
                    }
                }
                Response::Ok
            }
            Request::ListPhrases => Response::Phrases(shared.engine.phrases()),
            Request::AddPhrase { word, bopomofo } => {
                match shared.engine.add_phrase(&word, &bopomofo) {
                    Ok(()) => Response::Ok,
                    Err(e) => Response::Error(e),
                }
            }
            Request::Phrase {
                word,
                bopomofo,
                action,
            } => match shared.engine.phrase_action(&word, &bopomofo, action) {
                Ok(()) => Response::Ok,
                Err(e) => Response::Error(e),
            },
            Request::Stats => Response::Stats(shared.engine.stats()),
            Request::Backups => Response::Backups(zct_core::Engine::backups(&shared.backup)),
            Request::Restore { day } => match shared.engine.restore(&shared.backup, &day) {
                Ok(()) => {
                    log::info!("已從備份 {day} 還原，服務程序結束後重新載入");
                    // 先回應再結束；DLL 會在下次按鍵時重新啟動服務程序
                    let ui = shared.ui.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        ui.send(UiCmd::Exit);
                    });
                    Response::Ok
                }
                Err(e) => Response::Error(e.to_string()),
            },
            Request::Status => Response::Status {
                chinese: session.as_ref().is_none_or(|s| s.mode() == InputMode::Chinese),
                simplified: session.as_ref().is_some_and(|s| s.output() == Output::Simplified),
                game: shared.game.load(Ordering::Relaxed),
            },
            Request::ToggleMode => match session.as_mut() {
                Some(s) => {
                    let r = s.toggle_mode();
                    last = Some((r.clone(), hint(&r, None)));
                    Response::Key(r)
                }
                None => Response::Key(KeyResult::default()),
            },
            Request::Command(cmd) => {
                run_command(cmd, shared);
                Response::Ok
            }
            Request::SyncNow => {
                let _ = shared.sync.send(());
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
                bar_anchor = None;
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
