use std::path::Path;
use std::sync::Arc;

use crate::Platform;
use serde::{Deserialize, Serialize};
use zct_core::{DayStats, Engine, PhraseAction, PhraseEntry};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub dictionary: bool,
    pub learning: bool,
    pub backups: bool,
    pub restore: bool,
    pub sync: bool,
    pub ai: bool,
    pub api_key: bool,
    pub auto_update: bool,
    pub game_detection: bool,
    pub composition_bar_hotkey: bool,
    pub data_location: bool,
}
impl Capabilities {
    pub fn for_platform(platform: Platform) -> Self {
        let windows = platform == Platform::Windows;
        Self {
            dictionary: true,
            learning: true,
            backups: true,
            restore: true,
            sync: true,
            ai: windows,
            api_key: windows,
            auto_update: windows && zct_core::channel::CURRENT.id == "official",
            game_detection: windows,
            composition_bar_hotkey: windows,
            data_location: true,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    ListPhrases,
    AddPhrase {
        word: String,
        bopomofo: String,
    },
    Phrase {
        word: String,
        bopomofo: String,
        action: PhraseAction,
    },
    EditPhrase {
        original: PhraseEntry,
        word: String,
        bopomofo: String,
    },
    SuggestReading {
        word: String,
    },
    Stats,
    Backups,
    BackupNow,
    Restore {
        day: String,
    },
    SyncNow,
    SyncStatus,
    ApiKeyStatus,
    WriteApiKey {
        key: String,
    },
    TestAi {
        config: zct_core::config::AiConfig,
    },
}
impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No words, endpoint credentials or API keys in diagnostic output.
        f.write_str("SettingsAction(redacted)")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhraseView {
    #[serde(flatten)]
    pub entry: PhraseEntry,
    pub state: i32,
    pub kind: String,
}
impl From<PhraseEntry> for PhraseView {
    fn from(entry: PhraseEntry) -> Self {
        let state = if entry.is_banned() {
            2
        } else if entry.is_pinned() {
            1
        } else {
            0
        };
        let kind = if entry.bopomofo.is_empty() {
            "english"
        } else if zct_core::abbrev::is_abbrev(&entry.bopomofo) {
            "abbrev"
        } else {
            "phrase"
        }
        .into();
        Self { entry, state, kind }
    }
}
pub fn accuracy(chars: u64, corrections: u64) -> f64 {
    if chars == 0 {
        1.0
    } else {
        (1.0 - corrections as f64 / chars as f64).clamp(0.0, 1.0)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayView {
    #[serde(flatten)]
    pub entry: DayStats,
    pub accuracy: f64,
}
impl From<DayStats> for DayView {
    fn from(entry: DayStats) -> Self {
        Self {
            accuracy: accuracy(entry.chars, entry.corrections),
            entry,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reply {
    Done { restart_required: bool },
    Phrases { items: Vec<PhraseView> },
    Stats { items: Vec<DayView> },
    Backups { items: Vec<String> },
    ApiKeyStatus { present: bool },
    AiTest { message: String },
    Reading { value: String },
    SyncStatus { running: bool, message: String },
}
impl Reply {
    pub fn done() -> Self {
        Self::Done {
            restart_required: false,
        }
    }
}
/// A platform adapter may call the engine directly (macOS), or translate to IPC
/// (Windows). Requests and replies never contain UI or OS handle types.
pub trait Backend {
    fn execute(&mut self, action: Action) -> Result<Reply, String>;
}
/// Used by the engine-owning process, shared by IMK and the Windows service.
/// After Restore (including a failed restore), the owner must reload the engine.
/// Existing sessions must be dropped or reactivated before accepting new edits.
pub struct EngineBackend<'a> {
    engine: &'a Arc<Engine>,
    backup: &'a Path,
}
impl<'a> EngineBackend<'a> {
    pub fn new(engine: &'a Arc<Engine>, backup: &'a Path) -> Self {
        Self { engine, backup }
    }
}
impl Backend for EngineBackend<'_> {
    fn execute(&mut self, action: Action) -> Result<Reply, String> {
        match action {
            Action::ListPhrases => {
                let mut items = self.engine.phrases();
                items.extend(self.engine.english_entries());
                Ok(Reply::Phrases {
                    items: items.into_iter().map(PhraseView::from).collect(),
                })
            }
            Action::AddPhrase { word, bopomofo } => {
                self.engine.add_phrase(&word, &bopomofo)?;
                self.engine.save().map_err(|e| e.to_string())?;
                Ok(Reply::done())
            }
            Action::Phrase {
                word,
                bopomofo,
                action,
            } => {
                self.engine.phrase_action(&word, &bopomofo, action)?;
                self.engine.save().map_err(|e| e.to_string())?;
                Ok(Reply::done())
            }
            Action::EditPhrase {
                original,
                word,
                bopomofo,
            } => edit_phrase(self, original, word, bopomofo),
            Action::SuggestReading { word } => {
                let readings = zct_core::readings::Readings::load(
                    &self.engine.system_dictionary().join("tsi.csv"),
                )
                .map_err(|e| e.to_string())?;
                Ok(Reply::Reading {
                    value: readings.reading_of(&word).unwrap_or_default(),
                })
            }
            Action::Stats => Ok(Reply::Stats {
                items: self.engine.stats().into_iter().map(DayView::from).collect(),
            }),
            Action::Backups => Ok(Reply::Backups {
                items: Engine::backups(self.backup),
            }),
            Action::BackupNow => {
                self.engine
                    .save()
                    .and_then(|_| self.engine.backup(self.backup, 7))
                    .map_err(|e| e.to_string())?;
                Ok(Reply::done())
            }
            Action::Restore { day } => {
                self.engine
                    .restore(self.backup, &day)
                    .map_err(|e| e.to_string())?;
                Ok(Reply::Done {
                    restart_required: true,
                })
            }
            Action::ApiKeyStatus | Action::WriteApiKey { .. } | Action::TestAi { .. } => {
                Err("此平台尚未提供 AI 與憑證儲存功能".into())
            }
            Action::SyncNow | Action::SyncStatus => Err("此主機尚未提供 git 同步執行器".into()),
        }
    }
}

/// Editing has one shared workflow for direct and IPC backends. Validate/add the
/// replacement before removing the original; preserve its pin/ban state.
pub fn edit_phrase(
    backend: &mut impl Backend,
    original: PhraseEntry,
    word: String,
    bopomofo: String,
) -> Result<Reply, String> {
    let (word, bopomofo) = zct_core::Engine::normalize_phrase(&word, &bopomofo)?;
    let old = zct_core::Engine::normalize_phrase(&original.word, &original.bopomofo)?;
    if (word.clone(), bopomofo.clone()) == old {
        return Ok(Reply::done());
    }
    let Reply::Phrases { items } = backend.execute(Action::ListPhrases)? else {
        return Err("詞庫回應種類錯誤".into());
    };
    if items
        .iter()
        .any(|row| row.entry.word == word && row.entry.bopomofo == bopomofo)
    {
        return Err("詞庫已有相同詞語與讀音".into());
    }
    backend.execute(Action::AddPhrase {
        word: word.clone(),
        bopomofo: bopomofo.clone(),
    })?;
    if !zct_core::abbrev::is_abbrev(&bopomofo) {
        let action = if original.is_banned() {
            Some(PhraseAction::Ban)
        } else if original.is_pinned() && !bopomofo.is_empty() {
            Some(PhraseAction::Pin)
        } else if !bopomofo.is_empty() {
            Some(PhraseAction::Unban)
        } else {
            None
        };
        if let Some(action) = action {
            backend.execute(Action::Phrase {
                word: word.clone(),
                bopomofo: bopomofo.clone(),
                action,
            })?;
        }
    }
    backend.execute(Action::Phrase {
        word: original.word,
        bopomofo: original.bopomofo,
        action: PhraseAction::Remove,
    })
}
