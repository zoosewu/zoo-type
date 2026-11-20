use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zct_core::paths::DataRoot;
use zct_core::{AppRules, Config};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub config: Config,
    pub apps: AppRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub document: Document,
    pub revision: String,
    pub data_root: PathBuf,
    pub location_locked: bool,
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    data: DataRoot,
}

fn canonical(mut config: Config) -> Config {
    let (primary, secondary) = config.punctuation.effective();
    config.punctuation.primary = primary;
    config.punctuation.secondary = secondary;
    config.punctuation.shift = None;
    config.punctuation.right_alt = None;
    config
}
pub fn validate(config: &Config) -> Result<Config, String> {
    let config = canonical(config.clone());
    let text = toml::to_string(&config).map_err(|e| e.to_string())?;
    let config = Config::parse(&text).map_err(|e| e.to_string())?;
    // 數字欄位的範圍在設定目錄（兩個平台的設定畫面用同一份）
    let json = serde_json::to_value(&config).map_err(|e| e.to_string())?;
    for field in crate::catalog(crate::Platform::Windows).fields {
        let Some(range) = field.range else { continue };
        let value = field
            .path
            .split('.')
            .fold(&json, |v, key| &v[key])
            .as_f64()
            .unwrap_or(f64::NAN);
        if !(range.min as f64..=range.max as f64).contains(&value) {
            return Err(format!(
                "{}應在 {}–{} 之間",
                field.label, range.min, range.max
            ));
        }
    }
    Ok(config)
}

impl SettingsStore {
    pub fn new(data: DataRoot) -> Self {
        Self { data }
    }
    pub fn data(&self) -> &DataRoot {
        &self.data
    }
    pub fn config(&self) -> Result<Config, String> {
        Config::load(&self.data.config())
            .map(canonical)
            .map_err(|e| e.to_string())
    }
    pub fn apps(&self) -> Result<AppRules, String> {
        AppRules::load(&self.data.root().join("apps.toml")).map_err(|e| e.to_string())
    }
    pub fn revision(&self) -> Result<String, String> {
        Ok(revision(
            &read(&self.data.config())?,
            &read(&self.data.root().join("apps.toml"))?,
        ))
    }
    pub fn load(&self) -> Result<Snapshot, String> {
        // Read bytes once so the revision describes exactly the returned document.
        let config = read(&self.data.config())?;
        let apps = read(&self.data.root().join("apps.toml"))?;
        let document = Document {
            config: canonical(
                Config::parse(std::str::from_utf8(&config).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
            ),
            apps: AppRules::parse(std::str::from_utf8(&apps).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        };
        Ok(Snapshot {
            document,
            revision: revision(&config, &apps),
            data_root: self.data.root().to_owned(),
            location_locked: std::env::var_os(zct_core::paths::ENV_HOME).is_some(),
        })
    }
    /// Both UIs use this path. Validate everything before touching either file,
    /// reject stale edits, and restore the first file if the second write fails.
    /// Each file replacement is atomic; this is not a cross-file crash transaction.
    pub fn save(
        &self,
        document: &Document,
        expected_revision: Option<&str>,
    ) -> Result<Snapshot, String> {
        let config = validate(&document.config)?;
        let config_bytes = toml::to_string_pretty(&config)
            .map_err(|e| e.to_string())?
            .into_bytes();
        let apps_bytes = toml::to_string_pretty(&document.apps)
            .map_err(|e| e.to_string())?
            .into_bytes();
        fs::create_dir_all(self.data.root()).map_err(|e| e.to_string())?;
        let lock = self.data.root().join(".settings-write.lock");
        let _lock = Lock::acquire(lock)?;
        let config_path = self.data.config();
        let apps_path = self.data.root().join("apps.toml");
        let old_config = read(&config_path)?;
        let old_apps = read(&apps_path)?;
        if expected_revision.is_some_and(|expected| expected != revision(&old_config, &old_apps)) {
            return Err("設定已由其他視窗或程式變更，請重新載入後再儲存".into());
        }
        let existed = config_path.exists();
        // Stage both files first, including serialization and disk allocation.
        let first = stage(&config_path, &config_bytes)?;
        let second = stage(&apps_path, &apps_bytes)?;
        first.persist(&config_path).map_err(|e| e.to_string())?;
        if let Err(error) = second.persist(&apps_path) {
            let rollback = if existed {
                atomic_write(&config_path, &old_config)
            } else {
                fs::remove_file(&config_path).map_err(|e| e.to_string())
            };
            return Err(match rollback {
                Ok(()) => format!("無法儲存 App 規則，已還原一般設定：{error}"),
                Err(rollback) => format!("儲存失敗：{error}；還原一般設定失敗：{rollback}"),
            });
        }
        self.load()
    }
    pub fn save_config(&self, config: &Config) -> Result<(), String> {
        let mut snapshot = self.load()?;
        snapshot.document.config = config.clone();
        self.save(&snapshot.document, Some(&snapshot.revision))
            .map(|_| ())
    }
    pub fn save_apps(&self, apps: &AppRules) -> Result<(), String> {
        let mut snapshot = self.load()?;
        snapshot.document.apps = apps.clone();
        self.save(&snapshot.document, Some(&snapshot.revision))
            .map(|_| ())
    }
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    match fs::read(path) {
        Ok(data) => Ok(data),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}
fn revision(config: &[u8], apps: &[u8]) -> String {
    let hash = config
        .iter()
        .chain([0xff].iter())
        .chain(apps)
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}
fn stage(path: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile, String> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("設定路徑缺少父資料夾")?)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    Ok(file)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    stage(path, bytes)?
        .persist(path)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
struct Lock {
    _file: fs::File,
}
impl Lock {
    fn acquire(path: PathBuf) -> Result<Self, String> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.try_lock()
            .map_err(|e| format!("設定正由其他程式寫入：{e}"))?;
        Ok(Self { _file: file })
    }
}

/// Shared path validation; each host freezes/reloads its engine around this call.
pub fn relocate_directory(
    pointer: &Path,
    current: &Path,
    destination: &Path,
) -> Result<zct_core::paths::ChangeRoot, String> {
    if std::env::var_os(zct_core::paths::ENV_HOME).is_some() {
        return Err("ZCT_HOME 已指定資料位置，不能從設定視窗變更".into());
    }
    if !destination.is_absolute() {
        return Err("資料位置必須是絕對路徑".into());
    }
    if zct_core::paths::has_data(destination) {
        SettingsStore::new(DataRoot::new(destination)).load()?;
    }
    zct_core::paths::change_data_root(pointer, current, destination)
}
