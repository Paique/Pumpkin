//! The Axiom permission tree (mirror of upstream `AxiomPermission.java`,
//! ~95 nodes) plus the `axiom.default`/`axiom.all` child expansion from the
//! upstream `plugin.yml`.
//!
//! Nodes are registered under their literal `axiom.*` names so the reference
//! build server's `LuckPerms` seed and the public Axiom documentation keep
//! working, and so `axiom:restrictions` can serialize internal names without
//! a translation layer. `axiom.default` serializes on the wire as `axiom.*`
//! (upstream `getInternalName`).

use std::collections::HashMap;

use carrot_core::ModuleContext;
use pumpkin_util::permission::{Permission, PermissionDefault};

/// One node of the Axiom permission enum.
#[derive(Debug, Clone, Copy)]
pub struct AxiomPerm {
    /// The literal permission node (e.g. `axiom.build.place`).
    pub node: &'static str,
    /// The enum parent (wildcard group), mirroring upstream hierarchy.
    pub parent: Option<&'static str>,
    /// Whether the node is part of the `axiom.default` recommendation
    /// (children of `axiom.default` in the upstream `plugin.yml`).
    pub default_group: bool,
}

/// `axiom.default` — wire name is `axiom.*`.
pub const DEFAULT_NODE: &str = "axiom.default";
/// `axiom.all` — bypasses every other node, same as op.
pub const ALL_NODE: &str = "axiom.all";
/// The gate checked by the probe and handshake.
pub const USE_NODE: &str = "axiom.use";
/// Gate for `axiom:set_block` (phase AX-1).
pub const BUILD_PLACE_NODE: &str = "axiom.build.place";
/// Gate for `axiom:set_buffer` (phase AX-1).
pub const BUILD_SECTION_NODE: &str = "axiom.build.section";
/// Gate for block-entity NBT inside buffers (phase AX-1).
pub const BUILD_NBT_NODE: &str = "axiom.build.nbt";
/// Gate for `axiom:tick_blocks` (phase AX-1).
pub const BUILD_DANGEROUS_TICK_NODE: &str = "axiom.build_dangerous.tick";

const fn root(node: &'static str, default_group: bool) -> AxiomPerm {
    AxiomPerm {
        node,
        parent: None,
        default_group,
    }
}

const fn child(
    node: &'static str,
    parent: &'static str,
    default_group: bool,
) -> AxiomPerm {
    AxiomPerm {
        node,
        parent: Some(parent),
        default_group,
    }
}

/// All nodes, in enum order. Wire name of `axiom.default` is `axiom.*`.
#[rustfmt::skip]
pub const PERMISSIONS: &[AxiomPerm] = &[
    root(ALL_NODE, false),
    // entity (non-default)
    root("axiom.entity.*", false),
    child("axiom.entity.spawn", "axiom.entity.*", false),
    child("axiom.entity.manipulate", "axiom.entity.*", false),
    child("axiom.entity.delete", "axiom.entity.*", false),
    child("axiom.entity.request_data", "axiom.entity.*", false),
    // blueprint (non-default)
    root("axiom.blueprint.*", false),
    child("axiom.blueprint.upload", "axiom.blueprint.*", false),
    child("axiom.blueprint.request", "axiom.blueprint.*", false),
    child("axiom.blueprint.manifest", "axiom.blueprint.*", false),
    // annotation (non-default)
    root("axiom.annotation.*", false),
    child("axiom.annotation.create", "axiom.annotation.*", false),
    child("axiom.annotation.clear_all", "axiom.annotation.*", false),
    // build dangerous (non-default)
    root("axiom.build_dangerous.*", false),
    child("axiom.build_dangerous.tick", "axiom.build_dangerous.*", false),
    // default roots
    root(DEFAULT_NODE, true),
    child(USE_NODE, DEFAULT_NODE, true),
    child("axiom.allow_copying_other_plots", DEFAULT_NODE, true),
    child("axiom.can_import_blocks", DEFAULT_NODE, true),
    child("axiom.can_export_blocks", DEFAULT_NODE, true),
    // chunk
    root("axiom.chunk.*", true),
    child("axiom.chunk.request", "axiom.chunk.*", true),
    child("axiom.chunk.request_block_entity", "axiom.chunk.*", true),
    // build
    root("axiom.build.*", true),
    child("axiom.build.place", "axiom.build.*", true),
    child("axiom.build.section", "axiom.build.*", true),
    child("axiom.build.nbt", "axiom.build.*", true),
    // editor
    root("axiom.editor.*", true),
    child("axiom.editor.use", "axiom.editor.*", true),
    child("axiom.editor.views", "axiom.editor.*", true),
    // player
    root("axiom.player.*", true),
    child("axiom.player.bypass_movement_restrictions", "axiom.player.*", true),
    child("axiom.player.speed", "axiom.player.*", true),
    child("axiom.player.teleport", "axiom.player.*", true),
    child("axiom.player.gamemode.*", "axiom.player.*", true),
    child("axiom.player.gamemode.survival", "axiom.player.gamemode.*", true),
    child("axiom.player.gamemode.creative", "axiom.player.gamemode.*", true),
    child("axiom.player.gamemode.spectator", "axiom.player.gamemode.*", true),
    child("axiom.player.gamemode.adventure", "axiom.player.gamemode.*", true),
    child("axiom.player.hotbar", "axiom.player.*", true),
    child("axiom.player.set_no_physical_trigger", "axiom.player.*", true),
    // world
    root("axiom.world.*", true),
    child("axiom.world.time", "axiom.world.*", true),
    child("axiom.world.property", "axiom.world.*", true),
    // capability
    root("axiom.capability.*", true),
    child("axiom.capability.bulldozer", "axiom.capability.*", true),
    child("axiom.capability.replace_mode", "axiom.capability.*", true),
    child("axiom.capability.force_place", "axiom.capability.*", true),
    child("axiom.capability.no_updates", "axiom.capability.*", true),
    child("axiom.capability.tinker", "axiom.capability.*", true),
    child("axiom.capability.infinite_reach", "axiom.capability.*", true),
    child("axiom.capability.fast_place", "axiom.capability.*", true),
    child("axiom.capability.angel_placement", "axiom.capability.*", true),
    child("axiom.capability.no_clip", "axiom.capability.*", true),
    child("axiom.capability.phantom", "axiom.capability.*", true),
    // tools
    root("axiom.tool.*", true),
    child("axiom.tool.magic_select", "axiom.tool.*", true),
    child("axiom.tool.box_select", "axiom.tool.*", true),
    child("axiom.tool.freehand_select", "axiom.tool.*", true),
    child("axiom.tool.lasso_select", "axiom.tool.*", true),
    child("axiom.tool.ruler", "axiom.tool.*", true),
    child("axiom.tool.annotation", "axiom.tool.*", true),
    child("axiom.tool.painter", "axiom.tool.*", true),
    child("axiom.tool.noise_painter", "axiom.tool.*", true),
    child("axiom.tool.biome_painter", "axiom.tool.*", true),
    child("axiom.tool.gradient_painter", "axiom.tool.*", true),
    child("axiom.tool.script_brush", "axiom.tool.*", true),
    child("axiom.tool.freehand_draw", "axiom.tool.*", true),
    child("axiom.tool.sculpt_draw", "axiom.tool.*", true),
    child("axiom.tool.rock", "axiom.tool.*", true),
    child("axiom.tool.weld", "axiom.tool.*", true),
    child("axiom.tool.melt", "axiom.tool.*", true),
    child("axiom.tool.stamp", "axiom.tool.*", true),
    child("axiom.tool.text", "axiom.tool.*", true),
    child("axiom.tool.shape", "axiom.tool.*", true),
    child("axiom.tool.path", "axiom.tool.*", true),
    child("axiom.tool.modelling", "axiom.tool.*", true),
    child("axiom.tool.floodfill", "axiom.tool.*", true),
    child("axiom.tool.fluidball", "axiom.tool.*", true),
    child("axiom.tool.elevation", "axiom.tool.*", true),
    child("axiom.tool.slope", "axiom.tool.*", true),
    child("axiom.tool.smooth", "axiom.tool.*", true),
    child("axiom.tool.distort", "axiom.tool.*", true),
    child("axiom.tool.roughen", "axiom.tool.*", true),
    child("axiom.tool.shatter", "axiom.tool.*", true),
    child("axiom.tool.extrude", "axiom.tool.*", true),
    child("axiom.tool.modify", "axiom.tool.*", true),
    // builder tools
    root("axiom.builder_tool.*", true),
    child("axiom.builder_tool.move", "axiom.builder_tool.*", true),
    child("axiom.builder_tool.clone", "axiom.builder_tool.*", true),
    child("axiom.builder_tool.stack", "axiom.builder_tool.*", true),
    child("axiom.builder_tool.smear", "axiom.builder_tool.*", true),
    child("axiom.builder_tool.extrude", "axiom.builder_tool.*", true),
    child("axiom.builder_tool.erase", "axiom.builder_tool.*", true),
    child("axiom.builder_tool.setup_symmetry", "axiom.builder_tool.*", true),
];

/// The wire (internal) name of a node: `axiom.default` serializes as
/// `axiom.*`, everything else as the node itself.
#[must_use]
pub fn wire_name(node: &str) -> &str {
    if node == DEFAULT_NODE {
        "axiom.*"
    } else {
        node
    }
}

/// Looks up a node's enum entry.
#[must_use]
pub fn find(node: &str) -> Option<&'static AxiomPerm> {
    PERMISSIONS.iter().find(|perm| perm.node == node)
}

/// Registers every node with defaults and flattened children so group grants
/// like `axiom.default` or `axiom.build.*` resolve through the permission
/// manager's child/wildcard lookup.
///
/// Defaults follow the reference server: ops implicitly hold everything
/// (enforced by the restrictions short-circuit), `axiom.use`/`axiom.all` are
/// granted to ops, and nothing else is granted by default.
pub fn register(context: &ModuleContext<'_>) {
    use tracing::debug;

    fn children_of(predicate: impl Fn(&AxiomPerm) -> bool) -> HashMap<String, bool> {
        PERMISSIONS
            .iter()
            .filter(|perm| predicate(perm))
            .map(|perm| (perm.node.to_string(), true))
            .collect()
    }

    let registry = &context.server().permission_manager.registry;

    // axiom.all: every other node.
    let mut all = Permission::new(
        ALL_NODE,
        "Allows use of ALL Axiom features. Same as /op.",
        PermissionDefault::Op(pumpkin_util::permission::PermissionLvl::One),
    );
    for (node, value) in children_of(|perm| perm.node != ALL_NODE) {
        all.add_child(&node, value);
    }
    if let Err(err) = registry.register_permission(all) {
        debug!("carrot-axiom: {err}");
    }

    // axiom.use: ops only by default.
    if let Err(err) = registry.register_permission(Permission::new(
        USE_NODE,
        "Allows basic use of Axiom.",
        PermissionDefault::Op(pumpkin_util::permission::PermissionLvl::One),
    )) {
        debug!("carrot-axiom: {err}");
    }

    // axiom.default: flattened children of the recommended set.
    let mut default = Permission::new(
        DEFAULT_NODE,
        "Allows the use of all Axiom features recommended for public servers.",
        PermissionDefault::Deny,
    );
    for (node, value) in
        children_of(|perm| perm.default_group && perm.node != DEFAULT_NODE)
    {
        default.add_child(&node, value);
    }
    if let Err(err) = registry.register_permission(default) {
        debug!("carrot-axiom: {err}");
    }

    // Every remaining node, with direct children attached to group roots.
    for perm in PERMISSIONS {
        if perm.node == ALL_NODE || perm.node == USE_NODE || perm.node == DEFAULT_NODE {
            continue;
        }
        let mut permission =
            Permission::new(perm.node, "Axiom capability node.", PermissionDefault::Deny);
        for (node, value) in
            children_of(|other| other.parent == Some(perm.node))
        {
            permission.add_child(&node, value);
        }
        if let Err(err) = registry.register_permission(permission) {
            debug!("carrot-axiom: {err}");
        }
    }
}
