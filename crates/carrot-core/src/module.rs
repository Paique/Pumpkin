//! The module trait and the registration context handed to modules.

use std::sync::Arc;

use pumpkin_core::{
    command::node::detached::CommandDetachedNode,
    plugin::{Context, EventHandler, EventPriority, Payload},
    server::Server,
};
use pumpkin_util::permission::{Permission, PermissionDefault};

use crate::CarrotError;

/// A carrot feature module.
///
/// Modules are registered at startup (always compiled in) and enabled or
/// disabled at runtime. Everything a module registers through the
/// [`ModuleContext`] in [`CarrotModule::on_enable`] is tracked and removed
/// again when the module is disabled, so a module can be toggled repeatedly
/// without leaking commands, handlers or permissions.
///
/// Hooks must not block: long-running work should be spawned onto the tokio
/// runtime. See the upstream `AGENTS.md` rules on async work.
pub trait CarrotModule: Send + Sync {
    /// The stable module id used in `carrot.toml` and the `/carrot` command.
    fn id(&self) -> &'static str;

    /// Ids of modules that must be enabled before this one can enable.
    fn depends(&self) -> &'static [&'static str] {
        &[]
    }

    /// The default enabled state used when `carrot.toml` does not mention
    /// this module yet.
    fn default_enabled(&self) -> bool {
        true
    }

    /// Registers everything the module needs. Returning an error aborts the
    /// enable and is reported to the caller.
    ///
    /// # Errors
    ///
    /// Returns a [`CarrotError`] describing why the module could not start.
    fn on_enable(&self, context: &mut ModuleContext<'_>) -> Result<(), CarrotError>;

    /// Optional cleanup hook run before the module's commands, handlers and
    /// permissions are removed by the manager. Flush state here; do not
    /// unregister anything, the manager already does that.
    fn on_disable(&self, _context: &mut ModuleContext<'_>) {}
}

/// Registration handle passed to [`CarrotModule`] hooks.
///
/// Wraps the module's plugin [`Context`] (so commands and handlers reuse the
/// exact same machinery as regular plugins, including command tree
/// re-broadcasts) and tracks command names so the manager can unregister
/// them on disable.
pub struct ModuleContext<'a> {
    context: &'a Arc<Context>,
    data_folder: std::path::PathBuf,
    commands: Vec<String>,
}

impl<'a> ModuleContext<'a> {
    /// Creates a context for the given module plugin context and data folder.
    pub(crate) const fn new(context: &'a Arc<Context>, data_folder: std::path::PathBuf) -> Self {
        Self {
            context,
            data_folder,
            commands: Vec::new(),
        }
    }

    /// The server instance.
    #[must_use]
    pub fn server(&self) -> &Arc<Server> {
        &self.context.server
    }

    /// The module's private data folder (`carrot/modules/<id>`), guaranteed to
    /// exist.
    #[must_use]
    pub fn data_folder(&self) -> &std::path::Path {
        &self.data_folder
    }

    /// Registers a command under the module's source.
    ///
    /// `name` is the command literal, used to unregister the command again
    /// when the module is disabled. `permission` should be namespaced under
    /// `carrot:` (e.g. `carrot:essentials.fly`) so group wildcards resolve.
    pub fn register_command(
        &mut self,
        name: &str,
        node: impl Into<CommandDetachedNode>,
        permission: &str,
    ) {
        self.context.register_command(node, permission);
        self.commands.push(name.to_string());
    }

    /// Registers an event handler for the module.
    pub fn register_event<E, H>(&self, handler: Arc<H>, priority: EventPriority, blocking: bool)
    where
        E: Payload + 'static,
        H: EventHandler<E> + 'static,
    {
        self.context.register_event(handler, priority, blocking);
    }

    /// Registers a permission node (bypassing the plugin namespace check, so
    /// nodes can live under the shared `carrot:` namespace).
    pub fn register_permission(&self, node: &str, description: &str, default: PermissionDefault) {
        if let Err(err) = self
            .context
            .permission_manager
            .registry
            .register_permission(Permission::new(node, description, default))
        {
            tracing::debug!("carrot: {err}");
        }
    }

    /// Drains the command names registered through this context.
    pub(crate) fn take_commands(&mut self) -> Vec<String> {
        std::mem::take(&mut self.commands)
    }
}
