//! Sponge Schematic (`.schem`) reading and writing, compatible with the
//! files FAWE 2.x reads and writes.
//!
//! Reads versions 1, 2 and 3; writes version 2 (single `Schematic` root,
//! absolute `Offset`, `WEOffsetX/Y/Z` metadata), which every FAWE/WorldEdit
//! build since 1.13 understands.

use std::collections::HashMap;
use std::io::Cursor;

use pumpkin_data::BlockStateId;
use pumpkin_nbt::NbtCompound;
use pumpkin_nbt::nbt_compress::{read_gzip_compound_tag, write_gzip_compound_tag_to_bytes};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::{CURRENT_MC_DATA_VERSION, CURRENT_MC_VERSION};

use crate::clipboard::Clipboard;
use crate::states::{parse_state_str, state_to_string};

/// Reads a gzipped Sponge schematic into a clipboard.
///
/// # Errors
///
/// Returns a Portuguese error message when the file is not a schematic,
/// is corrupted or uses an unsupported version.
pub fn read_schematic(bytes: &[u8]) -> Result<Clipboard, String> {
    let mut cursor = Cursor::new(bytes);
    let root = read_gzip_compound_tag(&mut cursor)
        .map_err(|err| format!("arquivo .schem inválido: {err}"))?;

    // V1/V2: the root compound itself is named "Schematic". V3: an empty
    // root wraps a "Schematic" child.
    let schematic = root.get_compound("Schematic").map_or(&root, |inner| {
        if root.get_int("Version").is_some() {
            &root
        } else {
            inner
        }
    });

    let version = schematic
        .get_int("Version")
        .ok_or("schematic sem versão (não é um .schem Sponge)")?;
    if !(1..=3).contains(&version) {
        return Err(format!("versão de schematic não suportada: {version}"));
    }

    let size = read_dimensions(schematic)?;
    let (palette_compound, block_data, entities_list, biome_data) = if version == 3 {
        let blocks = schematic.get_compound("Blocks").ok_or("V3 sem Bloco")?;
        (
            blocks.get_compound("Palette"),
            blocks.get_byte_array("Data"),
            blocks.get_list("BlockEntities"),
            schematic
                .get_compound("Biomes")
                .and_then(|biomes| biomes.get_byte_array("Data")),
        )
    } else {
        (
            schematic.get_compound("Palette"),
            schematic.get_byte_array("BlockData"),
            schematic
                .get_list("BlockEntities")
                .or_else(|| schematic.get_list("TileEntities")),
            schematic.get_byte_array("BiomeData"),
        )
    };
    let _ = biome_data;

    let palette = read_palette(palette_compound)?;
    let expected = size.x as usize * size.y as usize * size.z as usize;
    let states = decode_blocks(block_data, &palette, expected)?;

    let (min, origin) = read_placement(schematic, version)?;

    let mut block_entities = HashMap::new();
    if let Some(tags) = entities_list {
        for tag in tags {
            let NbtTag::Compound(entity) = tag else {
                continue;
            };
            if let Some((index, nbt)) = read_block_entity(entity, &size, &min) {
                block_entities.insert(index, nbt);
            }
        }
    }

    Ok(Clipboard::from_parts(
        min,
        origin,
        size,
        states,
        block_entities,
    ))
}

/// Writes a clipboard as a gzipped Sponge schematic version 2.
///
/// # Errors
///
/// Returns a Portuguese error message when serialization fails.
pub fn write_schematic(clipboard: &Clipboard) -> Result<Vec<u8>, String> {
    let size = clipboard.size();
    let volume = clipboard.volume();

    // Palette in first-appearance order, exactly like FAWE writes.
    let mut palette: Vec<String> = Vec::new();
    let mut lookup: HashMap<String, u32> = HashMap::new();
    let mut data = Vec::with_capacity(volume);
    for index in 0..volume {
        let state = clipboard
            .block_at_index(index)
            .ok_or("clipboard truncada antes do fim")?;
        let name = state_to_string(state);
        let next = palette.len() as u32;
        let id = *lookup.entry(name.clone()).or_insert(next);
        if id == next {
            palette.push(name);
        }
        write_varint(&mut data, id);
    }

    let mut schematic = NbtCompound::new();
    schematic.put_int("Version", 2);
    schematic.put_int("DataVersion", CURRENT_MC_DATA_VERSION);
    schematic.put_short("Width", size.x as i16);
    schematic.put_short("Height", size.y as i16);
    schematic.put_short("Length", size.z as i16);

    let min = clipboard.min();
    let origin = clipboard.origin();
    schematic.put("Offset", NbtTag::IntArray(vec![min.0.x, min.0.y, min.0.z]));

    let mut palette_tag = NbtCompound::new();
    for (index, name) in palette.iter().enumerate() {
        palette_tag.put_int(name, index as i32);
    }
    schematic.put_compound("Palette", palette_tag.clone());
    schematic.put_int("PaletteMax", palette.len() as i32);
    schematic.put(
        "BlockData",
        NbtTag::ByteArray(data.into_iter().map(|byte| byte as i8).collect()),
    );

    let mut metadata = NbtCompound::new();
    metadata.put_int("WEOffsetX", min.0.x - origin.0.x);
    metadata.put_int("WEOffsetY", min.0.y - origin.0.y);
    metadata.put_int("WEOffsetZ", min.0.z - origin.0.z);
    schematic.put_compound("Metadata", metadata);

    let entities = clipboard.block_entities();
    if !entities.is_empty() {
        let mut list = Vec::with_capacity(entities.len());
        for (index, nbt) in entities {
            list.push(NbtTag::Compound(write_block_entity(nbt, *index, size)));
        }
        schematic.put_list("BlockEntities", list);
    }

    let mut root = NbtCompound::new();
    root.put_compound("Schematic", schematic);
    write_gzip_compound_tag_to_bytes(root).map_err(|err| format!("falha ao salvar .schem: {err}"))
}

/// The Minecraft version tag written into schematics.
#[must_use]
pub const fn minecraft_version() -> &'static str {
    CURRENT_MC_VERSION
}

fn read_dimensions(schematic: &NbtCompound) -> Result<Vector3<i32>, String> {
    let width = schematic.get_short("Width").ok_or("schematic sem Width")? as u16;
    let height = schematic
        .get_short("Height")
        .ok_or("schematic sem Height")? as u16;
    let length = schematic
        .get_short("Length")
        .ok_or("schematic sem Length")? as u16;
    if width == 0 || height == 0 || length == 0 {
        return Err("schematic com dimensão zero".to_string());
    }
    Ok(Vector3::new(
        i32::from(width),
        i32::from(height),
        i32::from(length),
    ))
}

fn read_palette(palette: Option<&NbtCompound>) -> Result<Vec<BlockStateId>, String> {
    let Some(palette) = palette else {
        return Err("schematic sem Palette".to_string());
    };
    let mut entries: Vec<(i32, String)> = palette
        .clone()
        .into_iter()
        .filter_map(|(name, tag)| match tag {
            NbtTag::Int(id) => Some((id, name.into_string())),
            _ => None,
        })
        .collect();
    entries.sort_by_key(|(id, _)| *id);

    let mut states = Vec::with_capacity(entries.len());
    for (id, name) in entries {
        if id != states.len() as i32 {
            return Err(format!("palette com índice pulado: {id}"));
        }
        states.push(parse_state_str(&name)?);
    }
    Ok(states)
}

fn decode_blocks(
    data: Option<&[i8]>,
    palette: &[BlockStateId],
    expected: usize,
) -> Result<Vec<BlockStateId>, String> {
    let Some(data) = data else {
        return Err("schematic sem BlockData".to_string());
    };
    let mut states = Vec::with_capacity(expected);
    let mut value: u32 = 0;
    let mut shift = 0;
    for &signed in data {
        let byte = signed as u8;
        value |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            let index = value as usize;
            let Some(state) = palette.get(index) else {
                return Err(format!("BlockData com índice fora da palette: {index}"));
            };
            states.push(*state);
            value = 0;
            shift = 0;
        } else {
            shift += 7;
            if shift > 35 {
                return Err("varint de BlockData corrompido".to_string());
            }
        }
    }
    if states.len() != expected {
        return Err(format!(
            "BlockData com {} blocos, esperados {expected}",
            states.len()
        ));
    }
    Ok(states)
}

/// Resolves the region minimum and paste anchor.
///
/// V1/V2 store the absolute world minimum in `Offset` and the anchor in
/// `Metadata.WEOffsetX/Y/Z` (`min - origin`); V3 stores the anchor in
/// `Metadata.WorldEdit.Origin` and a relative `Offset` from it.
fn read_placement(schematic: &NbtCompound, version: i32) -> Result<(BlockPos, BlockPos), String> {
    if version <= 2 {
        let offset = schematic.get_int_array("Offset").unwrap_or(&[0, 0, 0]);
        if offset.len() != 3 {
            return Err("Offset com tamanho errado".to_string());
        }
        let min = BlockPos(Vector3::new(offset[0], offset[1], offset[2]));
        let metadata = schematic.get_compound("Metadata");
        let we = [
            metadata.and_then(|meta| meta.get_int("WEOffsetX")),
            metadata.and_then(|meta| meta.get_int("WEOffsetY")),
            metadata.and_then(|meta| meta.get_int("WEOffsetZ")),
        ];
        let origin = BlockPos(Vector3::new(
            min.0.x - we[0].unwrap_or_default(),
            min.0.y - we[1].unwrap_or_default(),
            min.0.z - we[2].unwrap_or_default(),
        ));
        return Ok((min, origin));
    }

    let origin = schematic
        .get_compound("Metadata")
        .and_then(|meta| meta.get_compound("WorldEdit"))
        .and_then(|we| we.get_int_array("Origin"))
        .map_or(vec![0, 0, 0], std::borrow::ToOwned::to_owned);
    let offset = schematic.get_int_array("Offset").unwrap_or(&[0, 0, 0]);
    if origin.len() != 3 || offset.len() != 3 {
        return Err("origem V3 com tamanho errado".to_string());
    }
    let origin = BlockPos(Vector3::new(origin[0], origin[1], origin[2]));
    let min = BlockPos(Vector3::new(
        origin.0.x + offset[0],
        origin.0.y + offset[1],
        origin.0.z + offset[2],
    ));
    Ok((min, origin))
}
fn read_block_entity(
    entity: &NbtCompound,
    size: &Vector3<i32>,
    min: &BlockPos,
) -> Option<(usize, NbtCompound)> {
    let pos = entity
        .get_int_array("Pos")
        .or_else(|| entity.get_int_array("pos"))?;
    if pos.len() != 3 {
        return None;
    }
    let (x, y, z) = (pos[0], pos[1], pos[2]);
    if x < 0 || y < 0 || z < 0 || x >= size.x || y >= size.y || z >= size.z {
        return None;
    }
    let index = (x + z * size.x + y * size.x * size.z) as usize;

    let mut nbt = if let Some(NbtTag::Compound(data)) = entity.get("Data") {
        data.clone()
    } else {
        entity.clone()
    };
    nbt.put_int("x", min.0.x + x);
    nbt.put_int("y", min.0.y + y);
    nbt.put_int("z", min.0.z + z);
    if nbt.get_string("id").is_none()
        && let Some(id) = entity.get_string("Id")
    {
        let id = id.to_string();
        nbt.put_string("id", id);
    }
    Some((index, nbt))
}

fn write_block_entity(nbt: &NbtCompound, index: usize, size: Vector3<i32>) -> NbtCompound {
    let area = size.x * size.z;
    let y = (index as i32) / area;
    let rest = (index as i32) - y * area;
    let z = rest / size.x;
    let x = rest - z * size.x;

    let id = nbt.get_string("id").unwrap_or_default().to_string();
    let mut entity = NbtCompound::new();
    entity.put_string("Id", id);
    entity.put("Pos", NbtTag::IntArray(vec![x, y, z]));
    for (key, value) in nbt.clone() {
        let name: &str = &key;
        if matches!(name, "x" | "y" | "z" | "id") {
            continue;
        }
        entity.put(name, value);
    }
    entity
}

fn write_varint(buffer: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        buffer.push((value as u8) | 0x80);
        value >>= 7;
    }
    buffer.push(value as u8);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_clipboard() -> Clipboard {
        let stone = parse_state_str("stone").expect("stone");
        let dirt = parse_state_str("dirt").expect("dirt");
        let size = Vector3::new(2, 1, 2);
        let mut blocks = Vec::new();
        for index in 0..4 {
            blocks.push(if index % 2 == 0 { stone } else { dirt });
        }
        Clipboard::from_parts(
            BlockPos(Vector3::new(10, 64, 20)),
            BlockPos(Vector3::new(11, 64, 21)),
            size,
            blocks,
            HashMap::new(),
        )
    }

    #[test]
    fn round_trips_through_v2() {
        let original = sample_clipboard();
        let bytes = write_schematic(&original).expect("writes");
        let loaded = read_schematic(&bytes).expect("reads back");
        assert_eq!(loaded.size(), original.size());
        assert_eq!(loaded.min(), original.min());
        assert_eq!(loaded.origin(), original.origin());
        for (position, state) in original.iter_blocks() {
            assert_eq!(loaded.state_at_absolute(&position), Some(state));
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(read_schematic(b"isto nao e nbt").is_err());
    }
}
