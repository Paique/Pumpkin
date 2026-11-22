//! The carrot manager: module lifecycle, group application and boot wiring.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock as StdRwLock},
};

use pumpkin_core::{
    LOGGER_IMPL,
    entity::player::Player,
    plugin::{
        Context, EventHandler, EventPriority, PluginMetadata, events::player::PlayerJoinEvent,
    },
    server::Server,
};
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault};
use tracing::{error, info, warn};

use crate::{
    CarrotConfig, CarrotError, CarrotModule, GroupsConfig, ModuleContext,
    command::register_carrot_command,
};

/// Directory (relative to the working directory) holding all carrot state.
const CARROT_DIR: &str = "carrot";

/// Source prefix used for module plugin contexts (`carrot-<id>`).
fn module_source(id: &str) -> String {
    format!("carrot-{id}")
}

/// Snapshot of one module's state for listing.
#[derive(Debug, Clone)]
pub struct ModuleStateInfo {
    /// The module id.
    pub id: &'static str,
    /// Whether the module is currently enabled.
    pub enabled: bool,
}

/// A module registered with the manager.
struct RegisteredModule {
    module: Arc<dyn CarrotModule>,
    context: Arc<Context>,
    data_folder: PathBuf,
    enabled: bool,
    commands: Vec<String>,
}

/// Manages carrot modules and permission groups.
///
/// Created once at server startup by the `carrot` aggregator crate; commands
/// and the group-join handler reach it through [`crate::global`].
pub struct Carrot {
    server: Arc<Server>,
    root: PathBuf,
    config_path: PathBuf,
    groups_path: PathBuf,
    config: StdRwLock<CarrotConfig>,
    groups: StdRwLock<GroupsConfig>,
    modules: StdRwLock<Vec<RegisteredModule>>,
    root_context: Arc<Context>,
}

impl Carrot {
    /// Creates the manager. Call [`Carrot::register_module`] next and
    /// [`Carrot::init`] last.
    #[must_use]
    pub fn new(server: Arc<Server>, exec_dir: &Path) -> Self {
        let root = exec_dir.join(CARROT_DIR);
        let root_context = Arc::new(Context::new(
            root_metadata(),
            server.clone(),
            server.plugin_manager.handlers.clone(),
            server.plugin_manager.clone(),
            LOGGER_IMPL.clone(),
        ));
        Self {
            server,
            config_path: root.join("carrot.toml"),
            groups_path: root.join("groups.toml"),
            root,
            config: StdRwLock::new(CarrotConfig::default()),
            groups: StdRwLock::new(GroupsConfig::default()),
            modules: StdRwLock::new(Vec::new()),
            root_context,
        }
    }

    /// Adds a module to the registry (disabled until [`Carrot::init`]).
    pub fn register_module(&self, module: Arc<dyn CarrotModule>) {
        let id = module.id();
        let context = Arc::new(Context::new(
            module_metadata(id),
            self.server.clone(),
            self.server.plugin_manager.handlers.clone(),
            self.server.plugin_manager.clone(),
            LOGGER_IMPL.clone(),
        ));
        let data_folder = self.root.join("modules").join(id);
        self.modules
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RegisteredModule {
                module,
                context,
                data_folder,
                enabled: false,
                commands: Vec::new(),
            });
    }

    /// Loads configuration and groups, installs the always-on pieces
    /// (`/carrot` command, admin permission, group join handler) and enables
    /// every module the configuration asks for.
    pub fn init(&self) {
        if let Err(err) = fs::create_dir_all(self.root.join("modules")) {
            error!("carrot: failed to create module folders: {err}");
        }

        *self.config.write().unwrap_or_else(PoisonError::into_inner) =
            CarrotConfig::load(&self.config_path);
        *self.groups.write().unwrap_or_else(PoisonError::into_inner) =
            GroupsConfig::load(&self.groups_path);

        self.register_root();

        // Materialize every module in carrot.toml (with its default state),
        // then enable modules in dependency order until no progress is made.
        let mut defaults: Vec<(&'static str, bool)> = Vec::new();
        {
            let modules = self.modules.read().unwrap_or_else(PoisonError::into_inner);
            for registered in modules.iter() {
                defaults.push((registered.module.id(), registered.module.default_enabled()));
            }
        }
        {
            let mut config = self.config.write().unwrap_or_else(PoisonError::into_inner);
            for (id, default) in &defaults {
                config.modules.entry((*id).to_string()).or_insert(*default);
            }
        }
        self.persist_config();

        let mut progress = true;
        while progress {
            progress = false;
            let candidates: Vec<String> = {
                let modules = self.modules.read().unwrap_or_else(PoisonError::into_inner);
                modules
                    .iter()
                    .filter(|registered| !registered.enabled)
                    .map(|registered| registered.module.id().to_string())
                    .collect()
            };
            for id in candidates {
                let wants_enable = self
                    .config
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .modules
                    .get(&id)
                    .copied()
                    .unwrap_or(false);
                if !wants_enable {
                    continue;
                }
                match self.enable_module(&id) {
                    Ok(()) => progress = true,
                    Err(CarrotError::MissingDependency(_, _)) => {}
                    Err(err) => warn!("carrot: {err}"),
                }
            }
        }

        let (total, enabled) = {
            let modules = self.modules.read().unwrap_or_else(PoisonError::into_inner);
            (modules.len(), modules.iter().filter(|r| r.enabled).count())
        };
        info!("carrot: {enabled}/{total} modules enabled");
    }

    /// Enables a module at runtime.
    ///
    /// # Errors
    ///
    /// Returns a [`CarrotError`] if the module is unknown, already enabled,
    /// has a disabled dependency, or its `on_enable` hook failed.
    pub fn enable_module(&self, id: &str) -> Result<(), CarrotError> {
        let mut modules = self.modules.write().unwrap_or_else(PoisonError::into_inner);
        let registered = modules
            .iter()
            .find(|registered| registered.module.id() == id)
            .ok_or_else(|| CarrotError::UnknownModule(id.to_string()))?;
        if registered.enabled {
            return Err(CarrotError::AlreadyEnabled(id.to_string()));
        }
        for dep in registered.module.depends() {
            let dep_enabled = modules
                .iter()
                .any(|other| other.module.id() == *dep && other.enabled);
            if !dep_enabled {
                return Err(CarrotError::MissingDependency(
                    id.to_string(),
                    dep.to_string(),
                ));
            }
        }

        fs::create_dir_all(&registered.data_folder).map_err(CarrotError::Io)?;
        let mut context = ModuleContext::new(&registered.context, registered.data_folder.clone());
        if let Err(err) = registered.module.on_enable(&mut context) {
            return Err(CarrotError::EnableFailed(id.to_string(), err.to_string()));
        }
        let commands = context.take_commands();

        let registered = modules
            .iter_mut()
            .find(|registered| registered.module.id() == id)
            .ok_or_else(|| CarrotError::UnknownModule(id.to_string()))?;
        registered.commands = commands;
        registered.enabled = true;
        drop(modules);

        self.set_module_config(id, true);
        info!("carrot: enabled module '{id}'");
        Ok(())
    }

    /// Disables a module at runtime, removing its commands, event handlers
    /// and leaving its permission nodes registered (harmless without holders).
    ///
    /// # Errors
    ///
    /// Returns a [`CarrotError`] if the module is unknown, not enabled, or
    /// another enabled module depends on it.
    pub fn disable_module(&self, id: &str) -> Result<(), CarrotError> {
        let (module, context, data_folder, commands) = {
            let modules = self.modules.read().unwrap_or_else(PoisonError::into_inner);
            let registered = modules
                .iter()
                .find(|registered| registered.module.id() == id)
                .ok_or_else(|| CarrotError::UnknownModule(id.to_string()))?;
            if !registered.enabled {
                return Err(CarrotError::NotEnabled(id.to_string()));
            }
            for other in modules.iter() {
                if other.enabled && other.module.depends().contains(&id) {
                    return Err(CarrotError::DependentsActive(
                        id.to_string(),
                        other.module.id().to_string(),
                    ));
                }
            }
            (
                Arc::clone(&registered.module),
                Arc::clone(&registered.context),
                registered.data_folder.clone(),
                registered.commands.clone(),
            )
        };

        let source = context.get_metadata().name.clone();
        let mut module_context = ModuleContext::new(&context, data_folder);
        module.on_disable(&mut module_context);

        self.remove_handlers(&source);
        for name in &commands {
            context.unregister_command(name);
        }

        {
            let mut modules = self.modules.write().unwrap_or_else(PoisonError::into_inner);
            if let Some(registered) = modules
                .iter_mut()
                .find(|registered| registered.module.id() == id)
            {
                registered.commands.clear();
                registered.enabled = false;
            }
        }

        self.set_module_config(id, false);
        info!("carrot: disabled module '{id}'");
        Ok(())
    }

    /// Returns the state of every registered module.
    #[must_use]
    pub fn module_states(&self) -> Vec<ModuleStateInfo> {
        self.modules
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|registered| ModuleStateInfo {
                id: registered.module.id(),
                enabled: registered.enabled,
            })
            .collect()
    }

    /// Applies the permission groups of a player to their attachment and
    /// re-sends their command tree so gating takes effect immediately.
    pub fn apply_groups(&self, player: &Arc<Player>) {
        let permissions = self
            .groups
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .permissions_of(&player.gameprofile.name);
        for (node, value) in permissions {
            self.server
                .permission_manager
                .set_permission(player.gameprofile.id, node, value);
        }
        self.root_context.reload_commands_for(player);
    }

    /// Reloads `groups.toml` and re-applies groups to every online player.
    ///
    /// # Errors
    ///
    /// Returns a [`CarrotError`] if the groups file cannot be written when
    /// seeding.
    pub fn reload_groups(&self) -> Result<usize, CarrotError> {
        *self.groups.write().unwrap_or_else(PoisonError::into_inner) =
            GroupsConfig::load(&self.groups_path);
        let mut applied = 0;
        for world in self.server.worlds.load().iter() {
            for player in world.players.load().iter() {
                self.apply_groups(player);
                applied += 1;
            }
        }
        Ok(applied)
    }

    fn register_root(&self) {
        self.register_carrot_admin_permission();
        self.root_context
            .register_event(Arc::new(GroupJoinHandler), EventPriority::Normal, true);
        register_carrot_command(&self.root_context);
    }

    fn register_carrot_admin_permission(&self) {
        if let Err(err) =
            self.server
                .permission_manager
                .registry
                .register_permission(Permission::new(
                    "carrot:admin",
                    "Manage carrot modules and groups.",
                    PermissionDefault::Op(PermissionLvl::Four),
                ))
        {
            tracing::debug!("carrot: {err}");
        }
    }

    fn remove_handlers(&self, source: &str) {
        self.server.plugin_manager.handlers.rcu(|handlers| {
            let mut new_handlers = (**handlers).clone();
            new_handlers.retain(|_, handlers| {
                handlers.retain(|handler| handler.source() != Some(source));
                !handlers.is_empty()
            });
            Arc::new(new_handlers)
        });
    }

    fn set_module_config(&self, id: &str, enabled: bool) {
        self.config
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .modules
            .insert(id.to_string(), enabled);
        self.persist_config();
    }

    fn persist_config(&self) {
        let snapshot = self
            .config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Err(err) = snapshot.save(&self.config_path) {
            warn!(
                "carrot: failed to save {}: {err}",
                self.config_path.display()
            );
        }
    }
}

/// Applies permission groups when a player joins.
///
/// Registered as blocking so it completes before module join handlers (such
/// as `builder_mode`) check the resulting permissions; the root context is
/// registered before any module handler.
struct GroupJoinHandler;

impl EventHandler<PlayerJoinEvent> for GroupJoinHandler {
    fn handle<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a PlayerJoinEvent,
    ) -> pumpkin_core::plugin::BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Some(carrot) = crate::global() {
                carrot.apply_groups(&event.player);
            }
        })
    }
}

fn root_metadata() -> PluginMetadata {
    PluginMetadata {
        name: "carrot".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        authors: vec!["Latte Studios".to_string()],
        description: "Carrot server modules.".to_string(),
        dependencies: Vec::new(),
        permissions: Vec::new(),
    }
}

fn module_metadata(id: &str) -> PluginMetadata {
    PluginMetadata {
        name: module_source(id),
        version: env!("CARGO_PKG_VERSION").to_string(),
        authors: vec!["Latte Studios".to_string()],
        description: format!("Carrot module '{id}'."),
        dependencies: Vec::new(),
        permissions: Vec::new(),
    }
}
