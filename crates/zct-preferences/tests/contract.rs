use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;
use zct_core::paths::DataRoot;
use zct_core::{AppRule, Config, Engine};
use zct_preferences::*;

fn store() -> (tempfile::TempDir, SettingsStore) {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(DataRoot::new(directory.path()));
    (directory, store)
}
fn leaves(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    if let Some(object) = value.as_object().filter(|o| !o.is_empty()) {
        for (key, value) in object {
            let path = if prefix.is_empty() {
                key.to_owned()
            } else {
                format!("{prefix}.{key}")
            };
            leaves(value, &path, out);
        }
    } else {
        out.insert(prefix.to_owned());
    }
}
#[test]
fn every_config_and_app_field_has_one_shared_descriptor() {
    let win = catalog(Platform::Windows);
    let mac = catalog(Platform::Macos);
    assert_eq!(
        serde_json::to_value(&win.fields).unwrap(),
        serde_json::to_value(&mac.fields).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&win.app_fields).unwrap(),
        serde_json::to_value(&mac.app_fields).unwrap()
    );
    let config = serde_json::to_value(Config::default()).unwrap();
    let mut paths = BTreeSet::new();
    leaves(&config, "", &mut paths);
    let described: BTreeSet<_> = win.fields.iter().map(|f| f.path.clone()).collect();
    assert_eq!(
        paths, described,
        "a Config field was added without a settings UI descriptor"
    );
    assert_eq!(described.len(), win.fields.len(), "duplicate field paths");
    let pages: BTreeSet<_> = win.pages.iter().map(|p| p.id.as_str()).collect();
    for field in &win.fields {
        assert!(pages.contains(field.page.as_str()));
        let mut current = &config;
        for key in field.path.split('.') {
            current = &current[key];
        }
        assert_eq!(field.default, *current);
        if field.kind == FieldKind::Choice {
            assert!(field.choices.iter().any(|v| v.value == field.default));
        }
    }
    let keys: BTreeSet<_> = serde_json::to_value(AppRule::default())
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        win.app_fields.iter().map(|f| f.path.clone()).collect()
    );
    assert!(win.capabilities.ai && !mac.capabilities.ai);
    assert!(win.capabilities.sync && mac.capabilities.sync);
}
#[test]
fn document_save_preserves_cross_platform_values_and_rejects_stale_edits() {
    let (_directory, store) = store();
    let first = store.load().unwrap();
    let mut document = first.document.clone();
    document.config.ai.enabled = true;
    document.config.sync.enabled = true;
    document
        .config
        .punctuation
        .full
        .insert("shift+,".into(), "，".into());
    document.apps.apps.push(AppRule {
        name: "Code".into(),
        windows: Some("Code.exe".into()),
        macos: Some("com.microsoft.VSCode".into()),
        ..Default::default()
    });
    let saved = store.save(&document, Some(&first.revision)).unwrap();
    assert_eq!(saved.document, document);
    let mut stale = first.document.clone();
    stale.config.theme.font_size = 24.0;
    assert!(
        store
            .save(&stale, Some(&first.revision))
            .unwrap_err()
            .contains("變更")
    );
    assert_eq!(store.load().unwrap().document, document);
    let mut invalid = document;
    invalid.config.theme.font_size = 200.0;
    assert!(store.save(&invalid, Some(&saved.revision)).is_err());
    assert_eq!(store.load().unwrap().revision, saved.revision);
    invalid.config.theme.font_size = 16.0;
    invalid.config.candidates.per_page = 0;
    assert!(store.save(&invalid, Some(&saved.revision)).is_err());
    assert_eq!(store.load().unwrap().revision, saved.revision);
}
#[test]
fn failure_before_replacing_either_file_preserves_existing_bytes() {
    let (dir, store) = store();
    let initial = store.save(&Document::default(), None).unwrap();
    let path = dir.path().join("config.toml");
    let before = std::fs::read(&path).unwrap();
    std::fs::remove_file(dir.path().join("apps.toml")).unwrap();
    std::fs::create_dir(dir.path().join("apps.toml")).unwrap();
    let mut document = initial.document;
    document.config.theme.font_size = 24.0;
    assert!(store.save(&document, None).is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}
#[test]
fn conflicting_writers_are_rejected_and_locks_release_without_manual_cleanup() {
    let (dir, store) = store();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.path().join(".settings-write.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(store.save(&Document::default(), None).is_err());
    drop(lock);
    assert!(store.save(&Document::default(), None).is_ok());
    assert!(store.save(&Document::default(), None).is_ok());
}
#[test]
fn administrative_actions_use_the_shared_engine_and_restore_requires_reactivation() {
    let (dir, _) = store();
    let data = DataRoot::new(dir.path());
    let dict = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/dict");
    let engine = Arc::new(Engine::open(&dict, &data.learning()).unwrap());
    let backup = data.backup();
    let mut backend = EngineBackend::new(&engine, &backup);
    backend
        .execute(Action::AddPhrase {
            word: "測試".into(),
            bopomofo: "ㄘㄜˋ ㄕˋ".into(),
        })
        .unwrap();
    backend.execute(Action::BackupNow).unwrap();
    backend
        .execute(Action::Phrase {
            word: "測試".into(),
            bopomofo: "ㄘㄜˋ ㄕˋ".into(),
            action: zct_core::PhraseAction::Ban,
        })
        .unwrap();
    let Reply::Backups { items } = backend.execute(Action::Backups).unwrap() else {
        panic!()
    };
    let reply = backend
        .execute(Action::Restore {
            day: items[0].clone(),
        })
        .unwrap();
    assert!(matches!(
        reply,
        Reply::Done {
            restart_required: true
        }
    ));
    assert!(
        !engine.save().unwrap(),
        "the old engine must not overwrite restored learning"
    );
    let reloaded = Engine::open(&dict, &data.learning()).unwrap();
    assert!(
        reloaded
            .phrases()
            .iter()
            .any(|p| p.word == "測試" && !p.is_banned())
    );
}
#[test]
fn json_contract_dispatches_without_ui_or_transport_dependencies() {
    struct Mock;
    impl Backend for Mock {
        fn execute(&mut self, action: Action) -> Result<Reply, String> {
            match action {
                Action::Stats => Ok(Reply::Stats { items: vec![] }),
                _ => Err("unsupported".into()),
            }
        }
    }
    let (_directory, store) = store();
    let mut manager = Manager::new(store, Mock, Platform::Macos);
    let request: Request =
        serde_json::from_value(json!({"op":"manage","action":{"op":"stats"}})).unwrap();
    assert!(matches!(
        manager.execute(request).unwrap(),
        Response::Management {
            reply: Reply::Stats { .. }
        }
    ));
    assert!(
        serde_json::from_value::<Request>(json!({"op":"save","document":{},"revision":""}))
            .is_err()
    );
    assert!(serde_json::from_value::<Request>(json!({"op":"load","ignored":true})).is_err());
}

#[test]
fn phrase_edits_preserve_state_and_reject_invalid_replacements() {
    let (dir, _) = store();
    let dict = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/dict");
    let engine = Arc::new(Engine::open(&dict, &dir.path().join("learning")).unwrap());
    let backup = dir.path().join("backup");
    let mut backend = EngineBackend::new(&engine, &backup);
    backend
        .execute(Action::AddPhrase {
            word: "測試".into(),
            bopomofo: "ㄘㄜˋ ㄕˋ".into(),
        })
        .unwrap();
    backend
        .execute(Action::Phrase {
            word: "測試".into(),
            bopomofo: "ㄘㄜˋ ㄕˋ".into(),
            action: zct_core::PhraseAction::Ban,
        })
        .unwrap();
    let original = engine
        .phrases()
        .into_iter()
        .find(|p| p.word == "測試")
        .unwrap();
    assert!(
        backend
            .execute(Action::EditPhrase {
                original: original.clone(),
                word: "新詞".into(),
                bopomofo: "ㄒㄧㄣ".into()
            })
            .is_err()
    );
    assert!(
        engine
            .phrases()
            .iter()
            .any(|p| p.word == "測試" && p.is_banned())
    );
    backend
        .execute(Action::EditPhrase {
            original,
            word: "新詞".into(),
            bopomofo: "ㄒㄧㄣ ㄘˊ".into(),
        })
        .unwrap();
    assert!(
        engine
            .phrases()
            .iter()
            .any(|p| p.word == "新詞" && p.is_banned())
    );
    assert!(!engine.phrases().iter().any(|p| p.word == "測試"));
    let secret = Action::WriteApiKey {
        key: "never-log-this".into(),
    };
    assert!(!format!("{secret:?}").contains("never-log-this"));
}
