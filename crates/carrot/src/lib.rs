//! Carrot server feature modules.
//!
//! This crate wires the carrot module system into the server: [`init`] is
//! called once during boot (see `crates/pumpkin/src/main.rs`), registers the
//! built-in modules and starts everything the configuration enables. New
//! features are added as modules under `modules/` — never by editing the
//! upstream Pumpkin tree beyond the single `init` call.

mod modules;

use std::{path::Path, sync::Arc};

use carrot_core::Carrot;
use pumpkin_core::server::Server;

pub use modules::BuilderMode;

/// Initializes the carrot module system.
///
/// Call once after the server (and its plugin loaders) have been created and
/// before accepting connections.
pub fn init(server: Arc<Server>, exec_dir: &Path) {
    let carrot = Arc::new(Carrot::new(server, exec_dir));
    carrot_core::set_global(Arc::clone(&carrot));
    carrot.register_module(Arc::new(BuilderMode));
    carrot.register_module(Arc::new(carrot_essentials::Essentials::new()));
    carrot.register_module(Arc::new(carrot_multiverse::Multiverse::new()));
    carrot.register_module(Arc::new(carrot_chat::Chat::new()));
    let worldedit = Arc::new(carrot_worldedit::WorldEdit::new());
    let we_state = Arc::clone(worldedit.state());
    carrot.register_module(worldedit);
    // Terrain rides the worldedit engine (masks, edit, history), so it
    // must register after it and share its state.
    carrot.register_module(Arc::new(carrot_terrain::Terrain::new(we_state)));
    carrot.init();
}
