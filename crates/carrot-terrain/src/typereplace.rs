//! `//tr` (typereplace): swaps whole block families — stairs, slabs,
//! walls, fences, trapdoors, buttons and all — preserving state
//! properties. Literal port of Arceon's `TypeReplaceHandler`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use carrot_core::ModuleContext;
use carrot_worldedit::edit::{EditRequest, ensure_chunks, execute};
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_core::entity::player::Player;
use pumpkin_data::{Block, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;

use crate::commands::{ONLY_PLAYERS, PERM_BRUSH, player_of, reply};
use crate::state::TerrainState;

/// Family role slots, in table order.
const ROLE_COUNT: usize = 17;

/// One family: lookup names (first is canonical) and the block per role.
struct FamilyDef {
    names: &'static [&'static str],
    roles: [Option<&'static str>; ROLE_COUNT],
}

// Slot indices of the roles (must match the extraction order).
const BLOCK: usize = 0;
const NON_WAXED_BLOCK: usize = 1;
const STAIRS: usize = 2;
const NON_WAXED_STAIRS: usize = 3;
const SLAB: usize = 4;
const NON_WAXED_SLAB: usize = 5;
const FENCE: usize = 6;
const FENCE_GATE: usize = 7;
const WALL: usize = 8;
// slot 9: pressure plates (no special logic)
const BUTTON: usize = 10; // slot 9: pressure plates
const TRAPDOOR: usize = 11;
const NON_WAXED_TRAPDOOR: usize = 12;
const CRACKED: usize = 13;
const NON_WAXED_CRACKED: usize = 14;
const CHISELED: usize = 15;
const NON_WAXED_CHISELED: usize = 16;

/// The non-waxed form of a role maps back onto the primary slot when
/// building the replacement (Arceon reads `trTo.getStairs()` etc.).
const fn primary_of(role: usize) -> usize {
    match role {
        NON_WAXED_BLOCK => BLOCK,
        NON_WAXED_STAIRS => STAIRS,
        NON_WAXED_SLAB => SLAB,
        NON_WAXED_TRAPDOOR => TRAPDOOR,
        NON_WAXED_CRACKED => CRACKED,
        NON_WAXED_CHISELED => CHISELED,
        other => other,
    }
}

/// Roles whose replacement copies the source state properties onto the
/// target default state (Arceon `blockState.with(key, ...)` loop).
const fn copies_properties(role: usize) -> bool {
    matches!(
        role,
        STAIRS
            | NON_WAXED_STAIRS
            | SLAB
            | NON_WAXED_SLAB
            | FENCE
            | FENCE_GATE
            | WALL
            | BUTTON
            | TRAPDOOR
            | NON_WAXED_TRAPDOOR
    )
}

/// `//tr` flags: `-f` spares walls/fences/fence gates, `-b` plain
/// blocks, `-s` slabs (Arceon switch semantics).
#[derive(Clone, Copy, Default)]
pub struct Flags {
    prevent_fence: bool,
    prevent_block: bool,
    prevent_slab: bool,
}

/// Families extracted 1:1 from Arceon's `TypeReplaceHandler`.
const FAMILIES: &[FamilyDef] = &[
    FamilyDef {
        names: &["oak_planks", "oakplanks", "oak", "5", "5:0"],
        roles: [
            Some("oak_planks"),
            None,
            Some("oak_stairs"),
            None,
            Some("oak_slab"),
            None,
            Some("oak_fence"),
            Some("oak_fence_gate"),
            None,
            Some("oak_pressure_plate"),
            Some("oak_button"),
            Some("oak_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["spruce_planks", "spruceplanks", "spruce", "5:1"],
        roles: [
            Some("spruce_planks"),
            None,
            Some("spruce_stairs"),
            None,
            Some("spruce_slab"),
            None,
            Some("spruce_fence"),
            Some("spruce_fence_gate"),
            None,
            Some("spruce_pressure_plate"),
            Some("spruce_button"),
            Some("spruce_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["birch_planks", "birchplanks", "birch", "5:2"],
        roles: [
            Some("birch_planks"),
            None,
            Some("birch_stairs"),
            None,
            Some("birch_slab"),
            None,
            Some("birch_fence"),
            Some("birch_fence_gate"),
            None,
            Some("birch_pressure_plate"),
            Some("birch_button"),
            Some("birch_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["jungle_planks", "jungleplanks", "jungle", "5:3"],
        roles: [
            Some("jungle_planks"),
            None,
            Some("jungle_stairs"),
            None,
            Some("jungle_slab"),
            None,
            Some("jungle_fence"),
            Some("jungle_fence_gate"),
            None,
            Some("jungle_pressure_plate"),
            Some("jungle_button"),
            Some("jungle_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["acacia_planks", "acaciaplanks", "acacia", "5:4"],
        roles: [
            Some("acacia_planks"),
            None,
            Some("acacia_stairs"),
            None,
            Some("acacia_slab"),
            None,
            Some("acacia_fence"),
            Some("acacia_fence_gate"),
            None,
            Some("acacia_pressure_plate"),
            Some("acacia_button"),
            Some("acacia_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &[
            "dark_oak_planks",
            "darkoak_planks",
            "darkoakplanks",
            "dark_oak",
            "darkoak",
            "5:5",
        ],
        roles: [
            Some("dark_oak_planks"),
            None,
            Some("dark_oak_stairs"),
            None,
            Some("dark_oak_slab"),
            None,
            Some("dark_oak_fence"),
            Some("dark_oak_fence_gate"),
            None,
            Some("dark_oak_pressure_plate"),
            Some("dark_oak_button"),
            Some("dark_oak_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["mangrove_planks", "mangroveplanks", "mangrove", "5:6"],
        roles: [
            Some("mangrove_planks"),
            None,
            Some("mangrove_stairs"),
            None,
            Some("mangrove_slab"),
            None,
            Some("mangrove_fence"),
            Some("mangrove_fence_gate"),
            None,
            Some("mangrove_pressure_plate"),
            Some("mangrove_button"),
            Some("mangrove_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["bamboo_planks", "bambooplanks", "bamboo", "5:8"],
        roles: [
            Some("bamboo_planks"),
            None,
            Some("bamboo_stairs"),
            None,
            Some("bamboo_slab"),
            None,
            Some("bamboo_fence"),
            Some("bamboo_fence_gate"),
            None,
            Some("bamboo_pressure_plate"),
            Some("bamboo_button"),
            Some("bamboo_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["bamboo_mosaic", "bamboomosaic", "mosaicbamboo", "5:9"],
        roles: [
            Some("bamboo_mosaic"),
            None,
            Some("bamboo_mosaic_stairs"),
            None,
            Some("bamboo_mosaic_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["cherry_planks", "cherryplanks", "cherry", "5:7"],
        roles: [
            Some("cherry_planks"),
            None,
            Some("cherry_stairs"),
            None,
            Some("cherry_slab"),
            None,
            Some("cherry_fence"),
            Some("cherry_fence_gate"),
            None,
            Some("cherry_pressure_plate"),
            Some("cherry_button"),
            Some("cherry_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["stone", "1"],
        roles: [
            Some("stone"),
            None,
            Some("stone_stairs"),
            None,
            Some("stone_slab"),
            None,
            None,
            None,
            None,
            Some("stone_pressure_plate"),
            Some("stone_button"),
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["smooth_stone", "smoothstone", "43:8"],
        roles: [
            Some("smooth_stone"),
            None,
            None,
            None,
            Some("smooth_stone_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["granite", "1:1"],
        roles: [
            Some("granite"),
            None,
            Some("granite_stairs"),
            None,
            Some("granite_slab"),
            None,
            None,
            None,
            Some("granite_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["polished_granite", "1:2"],
        roles: [
            Some("polished_granite"),
            None,
            Some("polished_granite_stairs"),
            None,
            Some("polished_granite_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["diorite", "1:3"],
        roles: [
            Some("diorite"),
            None,
            Some("diorite_stairs"),
            None,
            Some("diorite_slab"),
            None,
            None,
            None,
            Some("diorite_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["polished_diorite", "1:4"],
        roles: [
            Some("polished_diorite"),
            None,
            Some("polished_diorite_stairs"),
            None,
            Some("polished_diorite_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["andesite", "1:5"],
        roles: [
            Some("andesite"),
            None,
            Some("andesite_stairs"),
            None,
            Some("andesite_slab"),
            None,
            None,
            None,
            Some("andesite_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["polished_andesite", "1:6"],
        roles: [
            Some("polished_andesite"),
            None,
            Some("polished_andesite_stairs"),
            None,
            Some("polished_andesite_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["cobblestone", "cobble", "4"],
        roles: [
            Some("cobblestone"),
            None,
            Some("cobblestone_stairs"),
            None,
            Some("cobblestone_slab"),
            None,
            None,
            None,
            Some("cobblestone_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &[
            "mossy_cobblestone",
            "mossy_cobble",
            "mossy",
            "mossycobble",
            "mossycobblestone",
            "48",
        ],
        roles: [
            Some("mossy_cobblestone"),
            None,
            Some("mossy_cobblestone_stairs"),
            None,
            Some("mossy_cobblestone_slab"),
            None,
            None,
            None,
            Some("mossy_cobblestone_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["mossy_stonebrick", "mossystonebrick", "98:1"],
        roles: [
            Some("mossy_stone_bricks"),
            None,
            Some("mossy_stone_brick_stairs"),
            None,
            Some("mossy_stone_brick_slab"),
            None,
            None,
            None,
            Some("mossy_stone_brick_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["sandstone", "24"],
        roles: [
            Some("sandstone"),
            None,
            Some("sandstone_stairs"),
            None,
            Some("sandstone_slab"),
            None,
            None,
            None,
            Some("sandstone_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_sandstone"),
            None,
        ],
    },
    FamilyDef {
        names: &["cut_sandstone", "cutsandstone", "24:2"],
        roles: [
            Some("cut_sandstone"),
            None,
            None,
            None,
            Some("cut_sandstone_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_sandstone"),
            None,
        ],
    },
    FamilyDef {
        names: &["smooth_sandstone", "smoothsandstone"],
        roles: [
            Some("smooth_sandstone"),
            None,
            Some("smooth_sandstone_stairs"),
            None,
            Some("smooth_sandstone_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["brick", "bricks", "45"],
        roles: [
            Some("bricks"),
            None,
            Some("brick_stairs"),
            None,
            Some("brick_slab"),
            None,
            None,
            None,
            Some("brick_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["red_sandstone", "redsandstone", "179"],
        roles: [
            Some("red_sandstone"),
            None,
            Some("red_sandstone_stairs"),
            None,
            Some("red_sandstone_slab"),
            None,
            None,
            None,
            Some("red_sandstone_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_red_sandstone"),
            None,
        ],
    },
    FamilyDef {
        names: &["cut_red_sandstone", "cutredsandstone"],
        roles: [
            Some("cut_red_sandstone"),
            None,
            None,
            None,
            Some("cut_red_sandstone_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_red_sandstone"),
            None,
        ],
    },
    FamilyDef {
        names: &["smooth_red_sandstone", "smoothredsandstone"],
        roles: [
            Some("smooth_red_sandstone"),
            None,
            Some("smooth_red_sandstone_stairs"),
            None,
            Some("smooth_red_sandstone_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["stonebrick", "stonebricks", "98"],
        roles: [
            Some("stone_bricks"),
            None,
            Some("stone_brick_stairs"),
            None,
            Some("stone_brick_slab"),
            None,
            None,
            None,
            Some("stone_brick_wall"),
            None,
            None,
            None,
            None,
            Some("cracked_stone_bricks"),
            None,
            Some("chiseled_stone_bricks"),
            None,
        ],
    },
    FamilyDef {
        names: &["netherbrick", "nether", "netherbricks", "112"],
        roles: [
            Some("nether_bricks"),
            None,
            Some("nether_brick_stairs"),
            None,
            Some("nether_brick_slab"),
            None,
            Some("nether_brick_fence"),
            None,
            Some("nether_brick_wall"),
            None,
            None,
            None,
            None,
            Some("cracked_nether_bricks"),
            None,
            Some("chiseled_nether_bricks"),
            None,
        ],
    },
    FamilyDef {
        names: &[
            "red_netherbrick",
            "rednetherbrick",
            "rednether",
            "red_nether",
            "red_netherbricks",
            "rednetherbricks",
            "215",
        ],
        roles: [
            Some("red_nether_bricks"),
            None,
            Some("red_nether_brick_stairs"),
            None,
            Some("red_nether_brick_slab"),
            None,
            None,
            None,
            Some("red_nether_brick_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["quartz", "155"],
        roles: [
            Some("quartz_block"),
            None,
            Some("quartz_stairs"),
            None,
            Some("quartz_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_quartz_block"),
            None,
        ],
    },
    FamilyDef {
        names: &["smooth_quartz", "smoothquartz"],
        roles: [
            Some("smooth_quartz"),
            None,
            Some("smooth_quartz_stairs"),
            None,
            Some("smooth_quartz_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_quartz_block"),
            None,
        ],
    },
    FamilyDef {
        names: &["purpur", "201"],
        roles: [
            Some("purpur_block"),
            None,
            Some("purpur_stairs"),
            None,
            Some("purpur_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["prismarine", "168"],
        roles: [
            Some("prismarine"),
            None,
            Some("prismarine_stairs"),
            None,
            Some("prismarine_slab"),
            None,
            None,
            None,
            Some("prismarine_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &[
            "prismarine_brick",
            "prismarinebrick",
            "prismarine_bricks",
            "prismarinebricks",
            "168:1",
        ],
        roles: [
            Some("prismarine_bricks"),
            None,
            Some("prismarine_brick_stairs"),
            None,
            Some("prismarine_brick_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["dark_prismarine", "darkprismarine", "168:2"],
        roles: [
            Some("dark_prismarine"),
            None,
            Some("dark_prismarine_stairs"),
            None,
            Some("dark_prismarine_slab"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &[
            "end_stone_bricks",
            "endbricks",
            "endstonebricks",
            "end_bricks",
        ],
        roles: [
            Some("end_stone_bricks"),
            None,
            Some("end_stone_brick_stairs"),
            None,
            Some("end_stone_brick_slab"),
            None,
            None,
            None,
            Some("end_stone_brick_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["iron", "irontrapdoor", "iron_trapdoor", "167"],
        roles: [
            None,
            None,
            None,
            None,
            None,
            None,
            Some("iron_bars"),
            None,
            None,
            Some("heavy_weighted_pressure_plate"),
            None,
            Some("iron_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["blackstone"],
        roles: [
            Some("blackstone"),
            None,
            Some("blackstone_stairs"),
            None,
            Some("blackstone_slab"),
            None,
            None,
            None,
            Some("blackstone_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &[
            "blackstone_bricks",
            "blackstone_brick",
            "blackstonebrick",
            "blackstonebricks",
        ],
        roles: [
            Some("polished_blackstone_bricks"),
            None,
            Some("polished_blackstone_brick_stairs"),
            None,
            Some("polished_blackstone_brick_slab"),
            None,
            None,
            None,
            Some("polished_blackstone_brick_wall"),
            None,
            None,
            None,
            None,
            Some("cracked_polished_blackstone_bricks"),
            None,
            Some("chiseled_polished_blackstone"),
            None,
        ],
    },
    FamilyDef {
        names: &["polished_blackstone", "polishedblackstone"],
        roles: [
            Some("polished_blackstone"),
            None,
            Some("polished_blackstone_stairs"),
            None,
            Some("polished_blackstone_slab"),
            None,
            None,
            None,
            Some("polished_blackstone_wall"),
            Some("polished_blackstone_pressure_plate"),
            Some("polished_blackstone_button"),
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["crimson_planks", "crimsonplanks", "crimson"],
        roles: [
            Some("crimson_planks"),
            None,
            Some("crimson_stairs"),
            None,
            Some("crimson_slab"),
            None,
            Some("crimson_fence"),
            Some("crimson_fence_gate"),
            None,
            Some("crimson_pressure_plate"),
            Some("crimson_button"),
            Some("crimson_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["warped_planks", "warpedplanks", "warped"],
        roles: [
            Some("warped_planks"),
            None,
            Some("warped_stairs"),
            None,
            Some("warped_slab"),
            None,
            Some("warped_fence"),
            Some("warped_fence_gate"),
            None,
            Some("warped_pressure_plate"),
            Some("warped_button"),
            Some("warped_trapdoor"),
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["deepslate_bricks", "deepslate_brick", "deepslate"],
        roles: [
            Some("deepslate_bricks"),
            None,
            Some("deepslate_brick_stairs"),
            None,
            Some("deepslate_brick_slab"),
            None,
            None,
            None,
            Some("deepslate_brick_wall"),
            None,
            None,
            None,
            None,
            Some("cracked_deepslate_bricks"),
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["cobbled_deepslate", "cobbleddeepslate"],
        roles: [
            Some("cobbled_deepslate"),
            None,
            Some("cobbled_deepslate_stairs"),
            None,
            Some("cobbled_deepslate_slab"),
            None,
            None,
            None,
            Some("cobbled_deepslate_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["polished_deepslate", "polisheddeepslate"],
        roles: [
            Some("polished_deepslate"),
            None,
            Some("polished_deepslate_stairs"),
            None,
            Some("polished_deepslate_slab"),
            None,
            None,
            None,
            Some("polished_deepslate_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["deepslate_tiles", "deepslatetiles"],
        roles: [
            Some("deepslate_tiles"),
            None,
            Some("deepslate_tile_stairs"),
            None,
            Some("deepslate_tile_slab"),
            None,
            None,
            None,
            Some("deepslate_tile_wall"),
            None,
            None,
            None,
            None,
            Some("cracked_deepslate_tiles"),
            None,
            Some("chiseled_deepslate"),
            None,
        ],
    },
    FamilyDef {
        names: &["cut_copper", "waxed_cut_copper", "cutcopper", "copper"],
        roles: [
            Some("waxed_cut_copper"),
            Some("cut_copper"),
            Some("waxed_cut_copper_stairs"),
            Some("cut_copper_stairs"),
            Some("waxed_cut_copper_slab"),
            Some("cut_copper_slab"),
            None,
            None,
            None,
            None,
            None,
            Some("waxed_copper_trapdoor"),
            Some("copper_trapdoor"),
            None,
            None,
            Some("waxed_chiseled_copper"),
            Some("chiseled_copper"),
        ],
    },
    FamilyDef {
        names: &[
            "exposed_cut_copper",
            "waxed_exposed_cut_copper",
            "exposedcutcopper",
            "copper",
            "exposedcopper",
        ],
        roles: [
            Some("waxed_exposed_cut_copper"),
            Some("exposed_cut_copper"),
            Some("waxed_exposed_cut_copper_stairs"),
            Some("exposed_cut_copper_stairs"),
            Some("waxed_exposed_cut_copper_slab"),
            Some("exposed_cut_copper_slab"),
            None,
            None,
            None,
            None,
            None,
            Some("waxed_exposed_copper_trapdoor"),
            Some("exposed_copper_trapdoor"),
            None,
            None,
            Some("waxed_exposed_chiseled_copper"),
            Some("exposed_chiseled_copper"),
        ],
    },
    FamilyDef {
        names: &[
            "weathered_cut_copper",
            "waxed_weathered_cut_copper",
            "weatheredcutcopper",
            "copper",
            "weatheredcopper",
        ],
        roles: [
            Some("waxed_weathered_cut_copper"),
            Some("weathered_cut_copper"),
            Some("waxed_weathered_cut_copper_stairs"),
            Some("weathered_cut_copper_stairs"),
            Some("waxed_weathered_cut_copper_slab"),
            Some("weathered_cut_copper_slab"),
            None,
            None,
            None,
            None,
            None,
            Some("waxed_weathered_copper_trapdoor"),
            Some("weathered_copper_trapdoor"),
            None,
            None,
            Some("waxed_weathered_chiseled_copper"),
            Some("weathered_chiseled_copper"),
        ],
    },
    FamilyDef {
        names: &[
            "oxidized_cut_copper",
            "waxed_oxidized_cut_copper",
            "oxidizedcutcopper",
            "copper",
            "oxidizedcopper",
        ],
        roles: [
            Some("waxed_oxidized_cut_copper"),
            Some("oxidized_cut_copper"),
            Some("waxed_oxidized_cut_copper_stairs"),
            Some("oxidized_cut_copper_stairs"),
            Some("waxed_oxidized_cut_copper_slab"),
            Some("oxidized_cut_copper_slab"),
            None,
            None,
            None,
            None,
            None,
            Some("waxed_oxidized_copper_trapdoor"),
            Some("oxidized_copper_trapdoor"),
            None,
            None,
            Some("waxed_oxidized_chiseled_copper"),
            Some("oxidized_chiseled_copper"),
        ],
    },
    FamilyDef {
        names: &["mud_bricks", "mudbricks", "mud"],
        roles: [
            Some("mud_bricks"),
            None,
            Some("mud_brick_stairs"),
            None,
            Some("mud_brick_slab"),
            None,
            None,
            None,
            Some("mud_brick_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    FamilyDef {
        names: &["tuff"],
        roles: [
            Some("tuff"),
            None,
            Some("tuff_stairs"),
            None,
            Some("tuff_slab"),
            None,
            None,
            None,
            Some("tuff_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_tuff"),
            None,
        ],
    },
    FamilyDef {
        names: &["tuff_bricks", "tuffbricks"],
        roles: [
            Some("tuff_bricks"),
            None,
            Some("tuff_brick_stairs"),
            None,
            Some("tuff_brick_slab"),
            None,
            None,
            None,
            Some("tuff_brick_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_tuff_bricks"),
            None,
        ],
    },
    FamilyDef {
        names: &["polished_tuff", "polishedtuff"],
        roles: [
            Some("polished_tuff"),
            None,
            Some("polished_tuff_stairs"),
            None,
            Some("polished_tuff_slab"),
            None,
            None,
            None,
            Some("polished_tuff_wall"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("chiseled_tuff"),
            None,
        ],
    },
];

/// Families with block names resolved against the running registry.
struct Registry {
    roles: Vec<[Option<&'static Block>; ROLE_COUNT]>,
    by_name: HashMap<&'static str, usize>,
}

impl Registry {
    fn build() -> Self {
        let mut roles = Vec::with_capacity(FAMILIES.len());
        let mut by_name = HashMap::new();
        for (index, family) in FAMILIES.iter().enumerate() {
            let mut resolved = [None; ROLE_COUNT];
            for (slot, name) in family.roles.iter().enumerate() {
                if let Some(name) = name
                    && let Some(block) = Block::from_registry_key(name)
                {
                    resolved[slot] = Some(block);
                }
            }
            for name in family.names {
                by_name.insert(*name, index);
            }
            roles.push(resolved);
        }
        Self { roles, by_name }
    }

    fn shared() -> &'static Self {
        static REGISTRY: OnceLock<Registry> = OnceLock::new();
        REGISTRY.get_or_init(Self::build)
    }
}

/// Looks a family up by any of its aliases (case-insensitive).
#[must_use]
pub fn family_index(name: &str) -> Option<usize> {
    let lower = name.to_ascii_lowercase();
    Registry::shared().by_name.get(lower.as_str()).copied()
}

/// The canonical names of every family, for the bare `//tr` listing.
#[must_use]
pub fn family_names() -> Vec<&'static str> {
    FAMILIES.iter().map(|family| family.names[0]).collect()
}

/// Converts one block state from family `from` to family `to`, mirroring
/// the Arceon ladder: plain roles set the default state, stateful roles
/// copy every source property the target also has.
#[must_use]
pub fn convert(source: BlockStateId, from: usize, to: usize, flags: Flags) -> Option<BlockStateId> {
    let registry = Registry::shared();
    let source_block = source.to_block();
    let from_roles = registry.roles.get(from)?;
    let to_roles = registry.roles.get(to)?;

    for (slot, candidate) in from_roles.iter().enumerate() {
        let Some(candidate) = candidate else { continue };
        if candidate != &source_block {
            continue;
        }
        if flags.prevent_block && matches!(slot, BLOCK | NON_WAXED_BLOCK)
            || flags.prevent_slab && matches!(slot, SLAB | NON_WAXED_SLAB)
            || flags.prevent_fence && matches!(slot, FENCE | FENCE_GATE | WALL)
        {
            return None;
        }
        let target = to_roles[primary_of(slot)]?;
        let mut state = target.default_state.id;
        if copies_properties(slot)
            && let Some(properties) = source_block.properties(source)
        {
            let mut parsed = Vec::new();
            for property in properties.to_props() {
                if let Ok(matching) =
                    target.parse_property(property.key_str(), property.value_str())
                {
                    parsed.push(matching);
                }
            }
            state = target.apply_properties(state, &parsed);
        }
        return Some(state);
    }
    None
}

/// Runs `//tr` over the player's selection.
pub async fn run(
    terrain: &Arc<TerrainState>,
    player: &Arc<Player>,
    from: usize,
    to: usize,
    flags: Flags,
) {
    let world = player.world();
    let selection = {
        let session = terrain.we.session(player.gameprofile.id);
        let guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.selection
    };
    let Some([corner_a, corner_b]) = selection else {
        player.send_system_message(&TextComponent::text(
            "Faça uma seleção primeiro (wand, //pos1 e //pos2).".to_string(),
        ));
        return;
    };
    let min = BlockPos::min(corner_a, corner_b);
    let max = BlockPos::max(corner_a, corner_b);

    let corners = vec![min, max];
    ensure_chunks(&world, &corners, |pos: &BlockPos| pos).await;

    let mut desired = Vec::new();
    for y in min.0.y..=max.0.y {
        for z in min.0.z..=max.0.z {
            for x in min.0.x..=max.0.x {
                let pos = BlockPos::new(x, y, z);
                let before = world.get_block_state_id(&pos);
                if let Some(after) = convert(before, from, to, flags) {
                    desired.push((pos, after));
                }
            }
        }
    }

    let description = format!("//tr {} {}", FAMILIES[from].names[0], FAMILIES[to].names[0]);
    execute(&terrain.we, player, EditRequest::new(desired, description)).await;
}

/// Registers `//tr`, `//typer` and `//typereplace`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    for name in ["//tr", "//typer", "//typereplace"] {
        let executor = TypeReplaceExecutor {
            state: Arc::clone(state),
        };
        context.register_command(
            name,
            command(name, "Replace any block type family with another.")
                .executes(executor.clone())
                .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
            PERM_BRUSH,
        );
    }
}

/// `//tr [from] [to] [-f|-b|-s]`; bare `//tr` lists the families.
#[derive(Clone)]
struct TypeReplaceExecutor {
    state: Arc<TerrainState>,
}

impl CommandExecutor for TypeReplaceExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "args").unwrap_or_default();
        let mut tokens = raw.split_whitespace();

        let (Some(from_token), Some(to_token)) = (tokens.next(), tokens.next()) else {
            let names = family_names().join(", ");
            reply(
                context,
                format!(
                    "Familias: {names}\nuse: //tr <de> <para> [-f paredes/cercas] [-b blocos] [-s lajes]"
                ),
            );
            return Ok(0);
        };
        let Some(from) = family_index(from_token) else {
            reply(context, format!("Familia desconhecida: {from_token}"));
            return Ok(0);
        };
        let Some(to) = family_index(to_token) else {
            reply(context, format!("Familia desconhecida: {to_token}"));
            return Ok(0);
        };

        let mut flags = Flags::default();
        for token in tokens {
            match token {
                "-f" => flags.prevent_fence = true,
                "-b" => flags.prevent_block = true,
                "-s" => flags.prevent_slab = true,
                other => {
                    reply(context, format!("flag desconhecida: {other}"));
                    return Ok(0);
                }
            }
        }

        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        tokio::spawn(async move {
            run(&state, &player, from, to, flags).await;
        });
        reply(context, "Aplicando troca de familia...".to_string());
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_resolve_and_lookup() {
        let stone = family_index("stone").expect("stone family");
        let deepslate = family_index("deepslate_bricks").expect("deepslate bricks");
        // Registry lookup accepts aliases and case.
        assert_eq!(family_index("STONE"), Some(stone));
        assert_eq!(family_index("Deepslate"), Some(deepslate));
        assert!(family_index("familia_inexistente").is_none());
    }

    #[test]
    fn plain_block_swaps_default_state() {
        let from = family_index("stone").expect("stone");
        let to = family_index("granite").expect("granite");
        let source = Block::from_registry_key("stone")
            .expect("stone block")
            .default_state
            .id;
        let converted = convert(source, from, to, Flags::default()).expect("swaps");
        assert_eq!(converted.to_block().name, "granite");
    }

    #[test]
    fn stairs_keep_properties() {
        let from = family_index("oak").expect("oak");
        let to = family_index("spruce").expect("spruce");
        let source = carrot_worldedit::states::parse_state_str(
            "minecraft:oak_stairs[facing=north,half=top,waterlogged=true]",
        )
        .expect("stairs state");
        let converted = convert(source, from, to, Flags::default()).expect("swaps");
        let text = carrot_worldedit::states::state_to_string(converted);
        assert!(
            text.starts_with("minecraft:spruce_stairs[facing=north"),
            "got {text}"
        );
        assert!(text.contains("half=top"), "got {text}");
        assert!(text.contains("waterlogged=true"), "got {text}");
    }

    #[test]
    fn flags_block_their_roles() {
        let from = family_index("oak").expect("oak");
        let to = family_index("spruce").expect("spruce");
        let slab = carrot_worldedit::states::parse_state_str("minecraft:oak_slab[type=top]")
            .expect("slab");
        assert!(convert(slab, from, to, Flags::default()).is_some());
        assert!(
            convert(
                slab,
                from,
                to,
                Flags {
                    prevent_slab: true,
                    ..Flags::default()
                }
            )
            .is_none()
        );
    }

    #[test]
    fn unrelated_block_is_untouched() {
        let from = family_index("stone").expect("stone");
        let to = family_index("granite").expect("granite");
        let dirt = Block::from_registry_key("dirt")
            .expect("dirt")
            .default_state
            .id;
        assert!(convert(dirt, from, to, Flags::default()).is_none());
    }
}
