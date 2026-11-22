//! Custom chunk generators for managed worlds.
//!
//! Void worlds are chunks of pure air in the `the_void` biome: nothing ever
//! spawns in them and there is no surface to spawn on, matching the
//! reference server's `VoidWorldGenerator`.

use std::sync::Arc;

use pumpkin_data::chunk::Biome;
use pumpkin_data::dimension::Dimension;
use pumpkin_util::world_seed::Seed;
use pumpkin_world::chunk_system::StagedChunkEnum;
use pumpkin_world::generation::generator::{
    CustomChunkGenerator, FlatLayer, WorldGenerator, flat::FlatGenerator,
};
use pumpkin_world::generation::proto_chunk::ProtoChunk;

use crate::config::GeneratorKind;

/// An empty-world generator: air chunks in `the_void`.
pub struct VoidGenerator {
    dimension: Dimension,
    seed: u64,
}

impl VoidGenerator {
    /// Creates the generator for the given dimension and seed.
    #[must_use]
    pub const fn new(dimension: Dimension, seed: u64) -> Self {
        Self { dimension, seed }
    }
}

impl CustomChunkGenerator for VoidGenerator {
    fn dimension(&self) -> &Dimension {
        &self.dimension
    }

    fn seed(&self) -> u64 {
        self.seed
    }

    fn step_to_biomes(&self, chunk: &mut ProtoChunk) {
        let biome_id = Biome::from_name("the_void").map_or(Biome::PLAINS.id, |biome| biome.id);
        chunk.flat_biome_map.fill(biome_id);
        chunk.stage = StagedChunkEnum::Biomes;
    }

    // `step_to_noise` keeps its default: no blocks are ever placed.
}

/// Builds the flat generator of the reference server's flat worlds.
///
/// Bedrock, stone, dirt and grass sum up to the overworld's 128 blocks
/// above `min_y`, so the grass surface sits at `y = 63` and players stand
/// on `y = 64` (where the reference `flat` warp is).
#[must_use]
pub fn flat_generator(dimension: Dimension, seed: u64) -> WorldGenerator {
    WorldGenerator::Flat(Box::new(FlatGenerator::new(
        Seed(seed),
        dimension,
        vec![
            FlatLayer {
                block: "minecraft:bedrock".to_string(),
                height: 1,
            },
            FlatLayer {
                block: "minecraft:stone".to_string(),
                height: 125,
            },
            FlatLayer {
                block: "minecraft:dirt".to_string(),
                height: 1,
            },
            FlatLayer {
                block: "minecraft:grass_block".to_string(),
                height: 1,
            },
        ],
        "minecraft:plains".to_string(),
    )))
}

/// Builds the world generator for a managed world definition; `None` keeps
/// the vanilla noise generator.
#[must_use]
pub fn world_generator(
    kind: GeneratorKind,
    dimension: Dimension,
    seed: u64,
) -> Option<WorldGenerator> {
    match kind {
        GeneratorKind::Vanilla => None,
        GeneratorKind::Void => Some(WorldGenerator::Custom(Arc::new(VoidGenerator::new(
            dimension, seed,
        )))),
        GeneratorKind::Flat => Some(flat_generator(dimension, seed)),
    }
}
