//! Platform-neutral settings contract. UIs render the catalog and edit documents;
//! host adapters implement runtime actions without owning configuration semantics.
mod backend;
mod catalog;
pub mod git;
mod store;
mod sync_worker;
pub use sync_worker::SyncWorker;

pub use backend::{
    Action, Backend, Capabilities, DayView, EngineBackend, PhraseView, Reply, accuracy, edit_phrase,
};
pub use catalog::{Catalog, Choice, Field, FieldKind, Lock, Page, Platform, app_fields, catalog};
pub use store::{Document, SettingsStore, Snapshot, relocate_directory, validate};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Load {},
    Catalog {},
    Save {
        document: Box<Document>,
        revision: String,
    },
    Manage {
        action: Action,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Snapshot { snapshot: Box<Snapshot> },
    Catalog { catalog: Catalog },
    Management { reply: Reply },
}

pub struct Manager<B> {
    store: SettingsStore,
    backend: B,
    platform: Platform,
}
impl<B: Backend> Manager<B> {
    pub fn new(store: SettingsStore, backend: B, platform: Platform) -> Self {
        Self {
            store,
            backend,
            platform,
        }
    }
    pub fn execute(&mut self, request: Request) -> Result<Response, String> {
        match request {
            Request::Load {} => Ok(Response::Snapshot {
                snapshot: Box::new(self.store.load()?),
            }),
            Request::Catalog {} => Ok(Response::Catalog {
                catalog: catalog(self.platform),
            }),
            Request::Save { document, revision } => Ok(Response::Snapshot {
                snapshot: Box::new(self.store.save(&document, Some(&revision))?),
            }),
            Request::Manage { action } => Ok(Response::Management {
                reply: self.backend.execute(action)?,
            }),
        }
    }
}
