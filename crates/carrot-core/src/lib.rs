//! Runtime-toggleable module system for the carrot server.
//!
//! Carrot features are implemented as native modules that compile together with
//! the server binary but can be enabled and disabled while the server is
//! running. A module registers its commands, event handlers, permissions and
//! configuration through a [`ModuleContext`] when enabled; disabling a module
//! removes everything it registered, mirroring the plugin unload semantics of
//! the Pumpkin plugin manager.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod command;
pub mod config;
pub mod groups;
pub mod manager;
pub mod module;

pub use config::CarrotConfig;
pub use groups::{GroupDef, GroupsConfig};
pub use manager::{Carrot, ModuleStateInfo};
pub use module::{CarrotModule, ModuleContext};

use std::sync::{Arc, OnceLock};

/// Errors produced by the carrot module system.
#[derive(thiserror::Error, Debug)]
pub enum CarrotError {
    /// No module with the given id is registered.
    #[error("unknown module '{0}'")]
    UnknownModule(String),
    /// The module is already enabled.
    #[error("module '{0}' is already enabled")]
    AlreadyEnabled(String),
    /// The module is not enabled.
    #[error("module '{0}' is not enabled")]
    NotEnabled(String),
    /// A dependency of the module is missing or disabled.
    #[error("module '{0}' requires dependency '{1}' to be enabled first")]
    MissingDependency(String, String),
    /// Another enabled module depends on the module being disabled.
    #[error("module '{0}' is required by enabled module '{1}'")]
    DependentsActive(String, String),
    /// The module's `on_enable` hook failed.
    #[error("module '{0}' failed to enable: {1}")]
    EnableFailed(String, String),
    /// An I/O error while loading or persisting carrot files.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

static GLOBAL: OnceLock<Arc<Carrot>> = OnceLock::new();

/// Installs the process-wide carrot manager.
///
/// Called once during server startup, before [`Carrot::init`].
pub fn set_global(carrot: Arc<Carrot>) {
    let _ = GLOBAL.set(carrot);
}

/// Returns the process-wide carrot manager, if it has been installed.
#[must_use]
pub fn global() -> Option<Arc<Carrot>> {
    GLOBAL.get().cloned()
}
