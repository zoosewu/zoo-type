//! 一個輸入 context（Windows：一條 DLL 連線；macOS：一個 IMK client）

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use zct_core::{Key, KeyEvent, KeyResult, Mods, Output, Session, SessionOptions};

use crate::ai::{self, Reply};
use crate::{Host, notice};

pub struct HostSession {
    host: Arc<Host>,
    id: u64,
    app: String,
    /// 記住中英狀態用的 key（Windows：程序；macOS：bundle ID）
    context: String,
    inner: Session,
    /// 建立 `inner` 時的 [`Host::revision`]
    revision: u64,
    composition_bar: bool,
    ai: Option<Receiver<Reply>>,
    /// 按下其他鍵就加一，背景 AI 回來時代數不同就丟掉
    ai_generation: Arc<AtomicU64>,
}

impl HostSession {
    /// `id`：平台用來對應畫面的編號；`app`：比對 App 規則的名稱（Windows 執行檔、macOS bundle ID）；
    /// `context`：記住中英狀態的 key，空字串表示不記
    pub fn open(
        host: &Arc<Host>,
        id: u64,
        app: &str,
        context: &str,
    ) -> Result<HostSession, String> {
        let revision = host.revision();
        let (inner, composition_bar) = new_session(host, id, app)?;
        let mut s = HostSession {
            host: host.clone(),
            id,
            app: app.to_owned(),
            context: context.to_owned(),
            inner,
            revision,
            composition_bar,
            ai: None,
            ai_generation: Arc::new(AtomicU64::new(0)),
        };
        if let Some(mode) = host.recall_mode(context) {
            s.inner.restore_mode(mode);
        }
        s.inner.set_global_game(host.game());
        Ok(s)
    }

    pub fn host(&self) -> &Arc<Host> {
        &self.host
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// 唯讀查詢（中英、輸出字形、偵測、放行的快捷鍵…）
    pub fn session(&self) -> &Session {
        &self.inner
    }

    pub fn composition_bar(&self) -> bool {
        self.composition_bar
    }

    pub fn game(&self) -> bool {
        self.host.game()
    }

    /// 背景 AI 請求還沒回來
    pub fn ai_pending(&self) -> bool {
        self.ai.is_some()
    }

    pub fn is_idle(&self) -> bool {
        self.inner.is_idle() && self.ai.is_none()
    }

    /// 設定或引擎變了、而且沒有組字：換成新設定的 session（沿用中英等狀態）。回傳有沒有換
    pub fn refresh(&mut self) -> bool {
        let revision = self.host.revision();
        if revision == self.revision || !self.is_idle() {
            return false;
        }
        // 失敗時沿用舊 session，不每次按鍵重試
        self.revision = revision;
        match new_session(&self.host, self.id, &self.app) {
            Ok((mut fresh, bar)) => {
                fresh.keep_state_of(&self.inner);
                self.inner = fresh;
                self.composition_bar = bar;
                log::info!("[{}] 套用新設定", self.id);
                true
            }
            Err(e) => {
                log::error!("[{}] {e}", self.id);
                false
            }
        }
    }

    /// 每次操作前：套用新設定、同步全域遊戲模式、收下已經回來的 AI 結果
    /// （按鍵等操作會自己呼叫；只查詢狀態前由平台呼叫）
    pub fn sync(&mut self) {
        self.refresh();
        self.inner.set_global_game(self.host.game());
        if let Some(Ok(reply)) = self.ai.as_ref().map(|rx| rx.try_recv()) {
            self.ai = None;
            self.apply_ai(reply);
        }
    }

    fn apply_ai(&mut self, (original, fixed): Reply) -> Option<String> {
        match fixed {
            Ok(fixed) => {
                (!self.inner.set_ai_suggestion(&original, fixed)).then(|| notice::AI_NO_TYPO.into())
            }
            Err(_) => {
                self.inner.set_ai_suggestion(&original, original.clone());
                Some(notice::AI_FAILED.into())
            }
        }
    }

    fn cancel_ai(&mut self) {
        self.ai = None;
        self.ai_generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn test_key_down(&mut self, ev: &KeyEvent) -> bool {
        self.sync();
        self.inner.test_key_down(ev)
    }

    pub fn key(&mut self, ev: &KeyEvent) -> KeyResult {
        self.sync();
        // 按下其他鍵：還在等的 AI 結果作廢（使用者之後打出同樣的字也一樣）
        if !ev.released && self.ai.is_some() {
            self.cancel_ai();
        }
        let mut r = self.inner.process_key(ev);
        if let Some(original) = r.ai_request.clone() {
            self.start_ai(original, &mut r);
        }
        let game = r.toggle_game.then(|| {
            let on = self.host.toggle_game();
            self.inner.set_global_game(on);
            on
        });
        if r.toggle_composition_bar {
            self.toggle_composition_bar(&mut r);
        }
        self.finish(r, game)
    }

    fn start_ai(&mut self, original: String, r: &mut KeyResult) {
        let cfg = self.host.settings().current().0.ai;
        let request = ai::Request {
            endpoint: zct_ai::Endpoint {
                base_url: cfg.endpoint,
                model: cfg.model,
                api_key: self.host.ai_key(),
                timeout: Duration::from_secs(cfg.timeout_seconds.max(1)),
            },
            engine: self.inner.engine(),
            original,
            syllables: self.inner.syllables(),
            generation: self.ai_generation.clone(),
            id: self.id,
        };
        match ai::spawn(request, self.host.ai_notify.clone()) {
            Ok(rx) => self.ai = Some(rx),
            Err(e) => {
                log::warn!("[{}] {e}", self.id);
                r.notice = Some(notice::AI_START_FAILED.into());
            }
        }
    }

    fn toggle_composition_bar(&mut self, r: &mut KeyResult) {
        let on = !self.composition_bar;
        match self.host.settings().set_composition_bar(&self.app, on) {
            Ok(()) => {
                self.composition_bar = on;
                log::info!(
                    "[{}] {}（{}）",
                    self.id,
                    notice::on_off("組字提示列", on),
                    self.app
                );
                r.notice = Some(notice::on_off("組字提示列", on));
            }
            Err(e) => {
                log::warn!("[{}] 寫入 App 規則失敗：{e}", self.id);
                r.notice = Some(format!("無法儲存組字提示列：{e}"));
            }
        }
        // 自己寫的 apps.toml 不用重新建立 session
        self.revision = self.host.revision();
    }

    fn finish(&self, mut r: KeyResult, game: Option<bool>) -> KeyResult {
        r.notice = notice::hint(&r, game);
        r
    }

    /// 背景 AI 結果回來了就套用並回傳要顯示的內容；還沒回來是 None
    pub fn poll_ai(&mut self) -> Option<KeyResult> {
        let reply = match self.ai.as_ref()?.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                (self.inner.preedit().text, Err("AI worker stopped".into()))
            }
        };
        self.ai = None;
        let notice = self.apply_ai(reply);
        Some(KeyResult {
            preedit: self.inner.preedit(),
            candidates: self.inner.candidates(),
            notice,
            ..Default::default()
        })
    }

    pub fn reconvert(&mut self, text: &str) -> KeyResult {
        self.sync();
        self.cancel_ai();
        let r = self.inner.reconvert(text);
        self.finish(r, None)
    }

    /// 點輸入法圖示或選單切換中英（組字中先送出）
    pub fn toggle_mode(&mut self) -> KeyResult {
        self.sync();
        self.cancel_ai();
        let r = self.inner.toggle_mode();
        self.finish(r, None)
    }

    pub fn toggle_output(&mut self) -> KeyResult {
        self.sync();
        self.cancel_ai();
        let r = self.inner.toggle_output();
        self.finish(r, None)
    }

    /// 選單切換遊戲模式：先送出組字
    pub fn toggle_game(&mut self) -> KeyResult {
        self.sync();
        self.cancel_ai();
        let on = self.host.toggle_game();
        self.inner.set_global_game(on);
        let commit = self.inner.flush_output();
        self.inner.reset();
        self.finish(
            KeyResult {
                eaten: true,
                commit: Some(commit),
                ..Default::default()
            },
            Some(on),
        )
    }

    /// 候選窗翻頁（滑鼠捲動）
    pub fn page(&mut self, next: bool) -> KeyResult {
        self.sync();
        self.cancel_ai();
        let key = if next { Key::PageDown } else { Key::PageUp };
        let r = self.inner.process_key(&KeyEvent::down(key, Mods::NONE, 0));
        self.finish(r, None)
    }

    /// 目前的畫面（不改變狀態）
    pub fn current(&mut self) -> KeyResult {
        self.sync();
        KeyResult {
            preedit: self.inner.preedit(),
            candidates: self.inner.candidates(),
            ..Default::default()
        }
    }

    /// 滑鼠點選候選；候選已改變或不可選時 `eaten` 是 false
    pub fn choose(&mut self, page: usize, row: usize) -> KeyResult {
        self.sync();
        self.cancel_ai();
        let r = self.inner.choose_candidate(page, row);
        self.finish(r, None)
    }

    /// 失焦：送出組字（已轉成設定的輸出字形）並清空
    pub fn flush(&mut self) -> KeyResult {
        self.cancel_ai();
        let commit = self.inner.flush_output();
        self.inner.reset();
        KeyResult {
            commit: Some(commit),
            ..Default::default()
        }
    }

    pub fn focus(&mut self) {
        self.cancel_ai();
        self.inner.focus();
    }

    pub fn reset(&mut self) {
        self.cancel_ai();
        self.inner.reset();
    }

    pub fn simplified(&self) -> bool {
        self.inner.output() == Output::Simplified
    }
}

impl Drop for HostSession {
    fn drop(&mut self) {
        self.ai_generation.fetch_add(1, Ordering::AcqRel);
        self.host.remember_mode(&self.context, self.inner.mode());
    }
}

fn new_session(host: &Host, id: u64, app: &str) -> Result<(Session, bool), String> {
    let (config, apps) = host.settings().current();
    let rule = apps.find(app).cloned();
    if let Some(r) = &rule {
        log::info!(
            "[{id}] 套用 App 規則「{}」",
            if r.name.is_empty() { app } else { &r.name }
        );
    }
    let bar = crate::composition_bar_for(app, &config, &apps);
    let session = Session::new(&SessionOptions {
        engine: host.engine(),
        config,
        rule,
    })
    .map_err(|e| e.to_string())?;
    Ok((session, bar))
}
