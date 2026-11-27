//! 一條 DLL 連線 = 一個 App 執行緒 = 一個輸入 session

use zct_core::{InputMode, KeyResult, Output};
use zct_host::HostSession;
use zct_ipc::pipe::ServerConn;
use zct_ipc::{Command, PROTOCOL_VERSION, Request, Response};
use zct_preferences::{Action, Backend, EngineBackend, Reply};

use crate::app::Shared;
use crate::ui::UiCmd;

/// 按鍵診斷：只記錄非字元鍵，絕不記錄字母、數字與符號
fn debug_key(shared: &Shared, id: u64, stage: &str, ev: &zct_core::KeyEvent, result: &str) {
    use zct_core::Key;
    if !shared.debug_keys {
        return;
    }
    // 字元鍵不記哪一個字（log 不能有打字內容）；只記帶 Ctrl / Alt / Win 的組合鍵有出現
    let key = match ev.key {
        Key::Char(_) | Key::Numpad(_) if ev.mods.has_command_modifier() => {
            "字元鍵（隱藏）".to_string()
        }
        Key::Char(_) | Key::Numpad(_) => return,
        k => format!("{k:?}"),
    };
    log::info!(
        "[{id}] 按鍵診斷 {stage} {key} {} shift={} ctrl={} alt={} right_alt={} t={} {result}",
        if ev.released { "放開" } else { "按下" },
        ev.mods.shift,
        ev.mods.ctrl,
        ev.mods.alt,
        ev.mods.right_alt,
        ev.time_ms,
    );
}

/// 更新走到哪一步；新版已經裝好、這是還在服務舊 App 的舊版時，提示重新開啟 App
fn update_state(shared: &Shared) -> zct_core::menu::UpdateState {
    use zct_core::menu::UpdateState;
    let Some(updater) = &shared.update else {
        return UpdateState::Off;
    };
    match crate::version::installed() {
        Some(v) => UpdateState::Installed(v),
        None => updater.status().state(),
    }
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
            shared.host.toggle_game();
        }
        Command::CheckUpdate => {
            log::info!("輸入法圖示：檢查更新");
            if let Some(u) = &shared.update {
                u.check_in_background();
            }
        }
        Command::InstallUpdate => {
            log::info!("輸入法圖示：安裝更新");
            let Some(updater) = &shared.update else {
                return;
            };
            // 安裝程式在背景跑、沒有回報：先顯示「正在安裝」，換版後舊服務程序改顯示「已更新」
            updater.set_installing(true);
            if let Err(e) = crate::update::install() {
                log::warn!("{e}");
                updater.set_installing(false);
            }
        }
        Command::Restart => {
            log::info!("重新啟動");
            let _ = shared.host.save();
            shared.ui.send(UiCmd::Exit);
        }
        Command::PrepareMove => {
            log::info!("變更資料位置：存檔並停止寫入");
            if let Err(e) = shared.host.freeze() {
                log::warn!("{e}");
            }
            // 同步進行中也一樣停止寫入：資料要搬走了
            crate::version::set_read_only();
            shared.host.set_read_only();
        }
    }
}

pub fn serve(id: u64, mut conn: ServerConn, shared: &Shared) {
    let _active = crate::version::ActiveGuard::new();
    let mut session: Option<HostSession> = None;
    // 最近一次按鍵結果（提示文字在 notice），等 DLL 回報游標位置後再決定候選窗怎麼顯示
    let mut last: Option<KeyResult> = None;
    // 三個彈出視窗怎麼顯示（規則與 macOS 共用）
    let mut popups = zct_core::popup::Popups::<zct_ipc::Rect>::default();

    loop {
        let req = match conn.recv() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => {
                log::warn!("[{id}] 讀取失敗：{e}");
                break;
            }
        };
        // 套用新設定、同步遊戲模式、收下回來的 AI 結果（只查詢狀態的請求也要最新）
        if let Some(s) = session.as_mut() {
            s.sync();
        }
        let resp = match req {
            Request::Hello { version, pid, exe } => {
                if version != PROTOCOL_VERSION {
                    log::warn!("[{id}] 協定版本不符：{version}（{exe}）");
                    Response::Error(format!("protocol {PROTOCOL_VERSION} expected"))
                } else {
                    log::info!("[{id}] 連線：{exe}（pid {pid}）");
                    // 切到別的輸入法時連線會斷：同一個程序再連回來沿用中英狀態
                    match HostSession::open(&shared.host, id, &exe, &format!("pid:{pid}")) {
                        Ok(s) => {
                            let h = s.session().heuristics();
                            let (detect_legacy, detect_fullscreen) =
                                (h.fullscreen_without_caret, h.exclusive_fullscreen);
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
                    let r = s.key(&ev);
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
                    last = Some(r.clone());
                    Response::Key(r)
                }
                None => Response::Key(KeyResult::default()),
            },
            Request::Reconvert { text } => match session.as_mut() {
                Some(s) => {
                    let start = std::time::Instant::now();
                    let r = s.reconvert(&text);
                    // 不記文字內容（log 不能有打字內容）
                    log::info!(
                        "[{id}] 重新選字：{} 個字，{} ms",
                        text.chars().count(),
                        start.elapsed().as_millis()
                    );
                    last = Some(r.clone());
                    Response::Key(r)
                }
                None => Response::Key(KeyResult::default()),
            },
            Request::Caret { rect, fresh } => {
                if shared.debug_keys {
                    log::info!("[{id}] 游標位置 {rect:?} fresh={fresh}");
                }
                if let Some(r) = last.take() {
                    use zct_core::popup::{PopupAction, PopupContent};
                    let content = PopupContent {
                        composing: !r.preedit.text.is_empty(),
                        composition_bar: session.as_ref().is_some_and(|s| s.composition_bar()),
                        candidates: r.candidates.is_some(),
                        notice: r.notice.is_some(),
                    };
                    let plan = popups.plan(content, rect, fresh);
                    match plan.bar {
                        PopupAction::Show(anchor) => shared.ui.send(UiCmd::Composition {
                            owner: id,
                            preedit: Some(r.preedit.clone()),
                            anchor,
                        }),
                        _ => shared.ui.send(UiCmd::Composition {
                            owner: id,
                            preedit: None,
                            anchor: rect,
                        }),
                    }
                    match (plan.candidates, r.candidates) {
                        (PopupAction::Show(anchor), Some(page)) => shared.ui.send(UiCmd::Show {
                            owner: id,
                            page,
                            anchor,
                        }),
                        _ => shared.ui.send(UiCmd::HideCandidates { owner: id }),
                    }
                    if let (PopupAction::Show(anchor), Some(text)) = (plan.hint, r.notice) {
                        shared.ui.send(UiCmd::Hint { text, anchor });
                    }
                }
                Response::Ok
            }
            Request::ListPhrases
            | Request::AddPhrase { .. }
            | Request::Phrase { .. }
            | Request::Stats
            | Request::Backups => {
                let action = match req {
                    Request::ListPhrases => Action::ListPhrases,
                    Request::AddPhrase { word, bopomofo } => Action::AddPhrase { word, bopomofo },
                    Request::Phrase {
                        word,
                        bopomofo,
                        action,
                    } => Action::Phrase {
                        word,
                        bopomofo,
                        action,
                    },
                    Request::Stats => Action::Stats,
                    Request::Backups => Action::Backups,
                    _ => unreachable!(),
                };
                match EngineBackend::new(&shared.host.engine(), &shared.host.data().backup())
                    .execute(action)
                {
                    Ok(Reply::Done { .. }) => Response::Ok,
                    Ok(Reply::Phrases { items }) => {
                        Response::Phrases(items.into_iter().map(|row| row.entry).collect())
                    }
                    Ok(Reply::Stats { items }) => {
                        Response::Stats(items.into_iter().map(|row| row.entry).collect())
                    }
                    Ok(Reply::Backups { items }) => Response::Backups(items),
                    Ok(_) => Response::Error("管理回應種類錯誤".into()),
                    Err(e) => Response::Error(e),
                }
            }
            Request::Restore { day } => {
                let result =
                    EngineBackend::new(&shared.host.engine(), &shared.host.data().backup())
                        .execute(Action::Restore { day });
                // Reload even after a copy error: restore may already have frozen
                // the old engine. Reconnection creates fresh sessions.
                let ui = shared.ui.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    ui.send(UiCmd::Exit);
                });
                match result {
                    Ok(_) => Response::Ok,
                    Err(error) => Response::Error(error),
                }
            }
            Request::Status => Response::Status {
                chinese: session
                    .as_ref()
                    .is_none_or(|s| s.session().mode() == InputMode::Chinese),
                simplified: session
                    .as_ref()
                    .is_some_and(|s| s.session().output() == Output::Simplified),
                game: shared.host.game(),
                right_alt: session
                    .as_ref()
                    .is_some_and(|s| s.session().right_alt_enabled()),
                update: update_state(shared),
                passthrough: session
                    .as_ref()
                    .and_then(|s| s.session().passthrough_keys()),
            },
            Request::ToggleMode | Request::ToggleOutput => match session.as_mut() {
                Some(s) => {
                    let r = if req == Request::ToggleMode {
                        s.toggle_mode()
                    } else {
                        s.toggle_output()
                    };
                    last = Some(r.clone());
                    Response::Key(r)
                }
                None => Response::Key(KeyResult::default()),
            },
            Request::Command(cmd) => {
                run_command(cmd, shared);
                Response::Ok
            }
            Request::SyncNow => match shared.host.sync_now() {
                Ok(_) => Response::Ok,
                Err(e) => {
                    log::warn!("同步：{e}");
                    Response::Error(e)
                }
            },
            Request::CheckUpdate => match shared.update.as_ref().map(|u| u.check()) {
                Some(Ok(version)) => Response::Update(version),
                Some(Err(e)) => Response::Error(e),
                None => Response::Error("只有正式版會檢查更新".into()),
            },
            Request::Reset | Request::Focus => {
                if shared.debug_keys {
                    log::info!("[{id}] 清除組字（{req:?}）");
                }
                if let Some(s) = session.as_mut() {
                    if req == Request::Focus {
                        s.focus();
                    } else {
                        s.reset();
                    }
                }
                last = None;
                popups.reset();
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
    // 中英狀態由 HostSession 記住
    drop(session);
    // App 關閉時順便存檔，縮短當機時遺失學習的時間窗
    if let Err(e) = shared.host.save() {
        log::error!("學習紀錄存檔失敗：{e}");
    }
    log::debug!("[{id}] 斷線");
}
