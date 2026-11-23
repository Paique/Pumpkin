//! Shared module state: configuration, the edit queue and one session
//! per player (selection, clipboard, mask, farwand and pending
//! confirmation).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use pumpkin_data::BlockStateId;
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use uuid::Uuid;

use crate::clipboard::Clipboard;
use crate::config::WorldEditConfig;
use crate::history::HistoryStore;
use crate::mask::Mask;
use crate::queue::EditQueue;

/// An edit whose diff is already on disk as history pages, waiting for
/// `//confirm` to apply (the pages hold nothing in memory).
pub struct PendingApply {
    /// History sequence of the sealed pages.
    pub seq: u64,
    /// Total changes the diff produced.
    pub total: u64,
    /// Serialized description shown to the player (`//set stone`).
    pub description: String,
}

/// Per-player editing state.
pub struct Session {
    /// Both selection corners once set.
    pub selection: Option<[BlockPos; 2]>,
    /// The copied region (kept across relogs, like FAWE disk clipboards).
    pub clipboard: Option<Arc<Clipboard>>,
    /// Session-wide mask applied to every edit.
    pub gmask: Option<Mask>,
    /// Whether the wand selects by crosshair ray at long range.
    pub farwand: bool,
    /// Edit diffed to disk but not applied yet.
    pub pending: Option<PendingApply>,
    /// On-disk undo/redo stacks for this player.
    pub history: HistoryStore,
}

impl Session {
    /// Creates a session with an opened history store.
    #[must_use]
    pub const fn new(history: HistoryStore) -> Self {
        Self {
            selection: None,
            clipboard: None,
            gmask: None,
            farwand: false,
            pending: None,
            history,
        }
    }
}

/// Module-wide state behind `Arc`.
///
/// Paths and configuration attach on `on_enable` (the module object is
/// built before the data folder exists); sessions are rebuilt lazily.
pub struct WeState {
    config: RwLock<WorldEditConfig>,
    history_root: RwLock<PathBuf>,
    schematics: RwLock<PathBuf>,
    sessions: Mutex<HashMap<Uuid, Arc<Mutex<Session>>>>,
    queue: EditQueue,
    worker: Mutex<Option<tokio::task::AbortHandle>>,
}

impl Default for WeState {
    fn default() -> Self {
        Self::new()
    }
}

impl WeState {
    /// Creates the state with empty paths; call [`Self::configure`] on
    /// enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: RwLock::new(WorldEditConfig::default()),
            history_root: RwLock::new(PathBuf::new()),
            schematics: RwLock::new(PathBuf::new()),
            sessions: Mutex::new(HashMap::new()),
            queue: EditQueue::new(),
            worker: Mutex::new(None),
        }
    }

    /// The shared edit queue (drained by the module's worker task).
    #[must_use]
    pub const fn queue(&self) -> &EditQueue {
        &self.queue
    }

    /// Stores (and replaces) the queue worker's abort handle.
    pub fn set_worker(&self, handle: tokio::task::AbortHandle) {
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *worker = Some(handle);
    }

    /// Aborts the queue worker, if running (module disable).
    pub fn abort_worker(&self) {
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(handle) = worker.take() {
            handle.abort();
        }
    }

    /// Attaches the data folder, (re)loads the configuration and drops
    /// in-memory sessions (history files stay on disk and are re-opened
    /// lazily with fresh retention pruning).
    pub fn configure(&self, data_folder: &std::path::Path) {
        {
            let mut config = self
                .config
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *config = WorldEditConfig::load(data_folder);
        }
        {
            let mut root = self
                .history_root
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *root = data_folder.join("history");
        }
        {
            let mut schematics = self
                .schematics
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *schematics = data_folder
                .parent()
                .and_then(|modules| modules.parent())
                .map_or_else(
                    || data_folder.join("schematics"),
                    |carrot_root| carrot_root.join("schematics"),
                );
        }
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Drops in-memory sessions on disable (no orphan state).
    pub fn clear_transient(&self) {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Runs `f` with a snapshot of the configuration.
    pub fn with_config<T>(&self, f: impl FnOnce(&WorldEditConfig) -> T) -> T {
        let config = self
            .config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&config)
    }

    /// The shared schematics folder (`carrot/schematics`).
    #[must_use]
    pub fn schematics_dir(&self) -> PathBuf {
        self.schematics
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The per-player session, created (with its history store) on first
    /// use. Never hold two session locks at once.
    pub fn session(&self, uuid: Uuid) -> Arc<Mutex<Session>> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sessions
            .entry(uuid)
            .or_insert_with(|| {
                let (capacity, days) =
                    self.with_config(|config| (config.history_size, config.history_days));
                let root = self
                    .history_root
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                let history = HistoryStore::open(&root, uuid, capacity, days);
                Arc::new(Mutex::new(Session::new(history)))
            })
            .clone()
    }
}
