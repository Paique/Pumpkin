//! `axiom:set_buffer` block-buffer decoding: runtime palettes per chunk
//! section plus ZSTD-dictionary-compressed block entities (mirror of
//! `BlockBuffer.load` + `CompressedBlockEntity`).
//!
//! Wire format per section: `long` section position (`BlockPos` packing of
//! section coordinates; the minimum possible long terminates the loop),
//! a vanilla paletted container (byte bits-per-entry; `0` = single value,
//! `1..=8` indirect with a `VarInt`-length palette, `>8` direct), then an
//! optional block-entity table (`VarInt` count capped at 4096 of
//! `short` local offset + compressed NBT).
//!
//! Palette entries resolve through the server's allowed-state filter:
//! void air and disallowed states become `None` ("leave this block alone"),
//! exactly like upstream's `EMPTY_STATE` slot in the player's registry.
//! Unknown state ids in a palette are a decode error (upstream
//! `byIdOrThrow`), which the caller logs and drops; direct-palette indices
//! out of range are skipped, matching upstream's null lookup.

use std::collections::HashMap;
use std::io::Cursor;

use pumpkin_data::BlockStateId;
use pumpkin_nbt::{Nbt, NbtCompound, deserializer::NbtReadHelperJava};

use crate::wire::{Reader, WireError, unpack_block_pos};

/// `AxiomConstants.MIN_POSITION_LONG`: packed `(-33554432, -2048, -33554432)`,
/// the terminator of the section loop (upstream asserts the exact bit
/// pattern: bits 63, 37 and 11 set).
pub const MIN_POSITION_LONG: i64 = crate::wire::pack_block_pos(-33_554_432, -2048, -33_554_432);

/// Entries per chunk section in a block buffer.
const SECTION_ENTRIES: usize = 4096;

/// Upstream `packetCollectionReadLimit` for the block-entity table.
const MAX_BLOCK_ENTITIES_PER_SECTION: usize = 4096;

/// Upstream `maxNbtDecompressLimit`: the largest block-entity NBT payload.
pub const MAX_NBT_DECOMPRESS_LIMIT: usize = 131_072;

/// The ZSTD dictionary shipped with the Axiom client for block-entity NBT.
static BLOCK_ENTITY_DICT: &[u8] =
    include_bytes!("../resources/zstd_dictionaries/block_entities_v1.dict");

/// A block entity as it travels in the buffer (not yet decompressed).
#[derive(Debug, Clone)]
pub struct CompressedBlockEntity {
    /// Declared uncompressed NBT size.
    original_size: u32,
    /// Compression dictionary id (`0` is the only supported one).
    dict: u8,
    /// ZSTD-compressed NBT (named root compound, Java `NbtIo` layout).
    compressed: Vec<u8>,
}

impl CompressedBlockEntity {
    /// Reads one entry: `VarInt` original size · byte dict · byte array.
    fn read(reader: &mut Reader<'_>) -> Result<Self, BufferError> {
        let original_size = reader.read_var_int()?;
        if original_size < 0 || original_size as usize > MAX_NBT_DECOMPRESS_LIMIT {
            return Err(BufferError::BlockEntityTooLarge(
                original_size.unsigned_abs() as usize,
            ));
        }
        let dict = reader.read_byte()?;
        let length = reader.read_var_int()?;
        if length < 0 {
            return Err(BufferError::Wire(WireError::UnexpectedEof));
        }
        let length = length as usize;
        // The compressed payload already fit inside the tunnel's packet cap;
        // keep an allocation guard anyway.
        if length > crate::tunnel::MAX_COMPRESSED_ENTITY_BYTES {
            return Err(BufferError::BlockEntityTooLarge(length));
        }
        let bytes = reader.take_bytes(length)?;
        Ok(Self {
            original_size: original_size as u32,
            dict,
            compressed: bytes.to_vec(),
        })
    }

    /// Decompresses and parses the NBT compound (Java named-root layout).
    ///
    /// Mirrors `CompressedBlockEntity.decompress`: unknown dictionary ids
    /// are rejected, the payload must decompress to exactly the declared
    /// size, and parsing uses the bounded (network-safe) limits.
    pub fn decompress(&self) -> Result<NbtCompound, BufferError> {
        if self.dict != 0 {
            return Err(BufferError::UnknownDictionary(self.dict));
        }
        let mut decompressor = zstd::bulk::Decompressor::with_dictionary(BLOCK_ENTITY_DICT)
            .map_err(BufferError::Zstd)?;
        let decompressed = decompressor
            .decompress(&self.compressed, self.original_size as usize)
            .map_err(BufferError::Zstd)?;
        if decompressed.len() != self.original_size as usize {
            return Err(BufferError::SizeMismatch);
        }
        let mut reader = NbtReadHelperJava::new(Cursor::new(decompressed.as_slice()));
        let nbt = Nbt::read(&mut reader).map_err(BufferError::Nbt)?;
        Ok(nbt.root_tag)
    }
}

/// A decoded runtime palette for one section.
#[derive(Debug, Clone)]
pub enum Palette {
    /// Every entry maps to one state (`None` = leave the block alone).
    Single(Option<BlockStateId>),
    /// Storage indices map through a palette of states.
    Indirect(Vec<Option<BlockStateId>>),
    /// Storage indices are global state ids directly.
    Direct {
        /// Bits per storage entry.
        bits: u8,
    },
}

impl Palette {
    /// Resolves one storage value to its state (`None` = leave alone).
    #[must_use]
    pub fn resolve(&self, raw: u64) -> Option<BlockStateId> {
        match self {
            Self::Single(state) => *state,
            Self::Indirect(palette) => palette.get(raw as usize).copied().flatten(),
            Self::Direct { bits: _ } => filter_state(raw),
        }
    }
}

/// One decoded section: position, palette and its packed storage.
#[derive(Debug)]
pub struct BufferSection {
    /// Section X coordinate (block X >> 4).
    pub sx: i32,
    /// Section Y coordinate (block Y >> 4).
    pub sy: i32,
    /// Section Z coordinate (block Z >> 4).
    pub sz: i32,
    /// Runtime palette.
    pub palette: Palette,
    /// Packed storage longs (big-endian, no spanning across longs).
    pub data: Vec<u64>,
    /// Bits per storage entry (0 for single-value palettes).
    bits: u8,
}

impl BufferSection {
    /// Iterates `(local_x, local_y, local_z, state)` for every position the
    /// buffer wants to change; `None` states (void air / disallowed) and
    /// out-of-range storage indices are skipped.
    pub fn changes(&self) -> impl Iterator<Item = (u8, u8, u8, BlockStateId)> + '_ {
        let bits = self.bits;
        let per_long = if bits == 0 { 0 } else { 64 / bits as usize };
        let mask: u64 = if bits >= 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        let palette = &self.palette;
        let data = &self.data;
        (0..SECTION_ENTRIES).filter_map(move |index| {
            let state = if bits == 0 {
                palette.resolve(0)?
            } else {
                let long_index = index / per_long;
                let shift = (index % per_long) * bits as usize;
                let raw = (data.get(long_index).copied().unwrap_or(0) >> shift) & mask;
                palette.resolve(raw)?
            };
            Some((
                (index & 0xF) as u8,
                (index >> 4 & 0xF) as u8,
                (index >> 8 & 0xF) as u8,
                state,
            ))
        })
    }
}

/// A decoded block buffer.
#[derive(Debug, Default)]
pub struct BlockBuffer {
    /// Decoded sections in wire order.
    pub sections: Vec<BufferSection>,
    /// Block entities keyed by section long then local offset
    /// (`x | y << 4 | z << 8`).
    pub block_entities: HashMap<i64, HashMap<u16, CompressedBlockEntity>>,
}

/// Failures while decoding a buffer. The caller logs and drops the packet
/// (upstream: `logSevere` on the tunnel path, kick on the dedicated path).
#[derive(Debug)]
pub enum BufferError {
    /// A wire-level read failed.
    Wire(WireError),
    /// A palette declared more entries than its bit width can index.
    PaletteTooLarge(usize),
    /// A palette entry or single value is not a valid global state id.
    InvalidState(u32),
    /// A block-entity payload exceeded the decompression limit.
    BlockEntityTooLarge(usize),
    /// An unknown compression dictionary id.
    UnknownDictionary(u8),
    /// ZSTD decompression failed.
    Zstd(std::io::Error),
    /// Decompressed size did not match the declared size.
    SizeMismatch,
    /// The NBT payload failed to parse within the bounded limits.
    Nbt(pumpkin_nbt::Error),
}

impl From<WireError> for BufferError {
    fn from(value: WireError) -> Self {
        Self::Wire(value)
    }
}

impl std::fmt::Display for BufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wire(err) => write!(f, "malformed buffer payload: {err}"),
            Self::PaletteTooLarge(len) => write!(f, "palette length {len} exceeds its bit width"),
            Self::InvalidState(id) => write!(f, "unknown block state id {id}"),
            Self::BlockEntityTooLarge(size) => write!(f, "block entity payload {size} too large"),
            Self::UnknownDictionary(id) => write!(f, "unknown compression dict {id}"),
            Self::Zstd(err) => write!(f, "zstd failure: {err}"),
            Self::SizeMismatch => write!(f, "uncompressed size mismatch"),
            Self::Nbt(err) => write!(f, "invalid block entity nbt: {err}"),
        }
    }
}

impl std::error::Error for BufferError {}

/// Maps a raw global state id to the state the server may place.
///
/// `None` means "leave the block alone": the protocol's void-air sentinel
/// plus every state on the configured disallowed list. Out-of-range ids
/// also map to `None` (direct palettes look them up per position).
#[must_use]
pub fn filter_state(raw: u64) -> Option<BlockStateId> {
    let id = BlockStateId::new(u16::try_from(raw).ok()?)?;
    if id == pumpkin_data::Block::VOID_AIR.default_state.id
        || crate::blocks::is_disallowed(id.to_block_id())
    {
        return None;
    }
    Some(id)
}

/// Strict variant for palette entries: the id must exist in the global
/// registry (upstream `byIdOrThrow`) before the allowed filter runs.
fn strict_state(raw: u32) -> Result<Option<BlockStateId>, BufferError> {
    let id = u16::try_from(raw).map_err(|_| BufferError::InvalidState(raw))?;
    if BlockStateId::new(id).is_none() {
        return Err(BufferError::InvalidState(raw));
    }
    Ok(filter_state(u64::from(id)))
}

/// Reads one vanilla paletted container (see module docs for the layout).
fn read_palette(reader: &mut Reader<'_>) -> Result<(Palette, Vec<u64>, u8), BufferError> {
    let bits = reader.read_byte()?;
    let palette = if bits == 0 {
        let raw = reader.read_var_int()?;
        if raw < 0 {
            return Err(BufferError::InvalidState(raw.unsigned_abs()));
        }
        Palette::Single(strict_state(raw as u32)?)
    } else if bits <= 8 {
        let count = reader.read_var_int()?;
        if count < 0 || count as usize > (1usize << bits) {
            return Err(BufferError::PaletteTooLarge(count.unsigned_abs() as usize));
        }
        let mut palette = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let raw = reader.read_var_int()?;
            if raw < 0 {
                return Err(BufferError::InvalidState(raw.unsigned_abs()));
            }
            palette.push(strict_state(raw as u32)?);
        }
        Palette::Indirect(palette)
    } else if bits <= 16 {
        Palette::Direct { bits }
    } else {
        return Err(BufferError::PaletteTooLarge(bits as usize));
    };

    let longs = if bits == 0 {
        0
    } else {
        SECTION_ENTRIES.div_ceil(64 / bits as usize)
    };
    let mut data = Vec::with_capacity(longs);
    for _ in 0..longs {
        data.push(reader.read_i64_be()? as u64);
    }
    Ok((palette, data, bits))
}

impl BlockBuffer {
    /// Decodes a block buffer from a `set_buffer` payload body (everything
    /// after the buffer type byte; the trailing dispatch-sends varint is
    /// consumed by the caller).
    pub fn load(reader: &mut Reader<'_>) -> Result<Self, BufferError> {
        let mut buffer = Self::default();
        loop {
            let index = reader.read_i64_be()?;
            if index == MIN_POSITION_LONG {
                break;
            }
            let (sx, sy, sz) = unpack_block_pos(index);
            let (palette, data, bits) = read_palette(reader)?;

            // Upstream caps the table read at min(4096, varint); a negative
            // varint reads as the unsigned bit pattern and still clamps.
            let table_size = reader.read_var_int()?.max(0) as usize;
            let table_size = table_size.min(MAX_BLOCK_ENTITIES_PER_SECTION);
            if table_size > 0 {
                let mut table = HashMap::with_capacity(table_size);
                for _ in 0..table_size {
                    let offset = reader.read_i16_be()? as u16;
                    let entity = CompressedBlockEntity::read(reader)?;
                    table.insert(offset, entity);
                }
                buffer.block_entities.insert(index, table);
            }

            buffer.sections.push(BufferSection {
                sx,
                sy,
                sz,
                palette,
                data,
                bits,
            });
        }
        Ok(buffer)
    }

    /// Number of chunk sections the buffer touches (dispatch-sends cost).
    #[must_use]
    pub const fn section_count(&self) -> usize {
        self.sections.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;

    const STONE: u32 = 1;

    fn section_with(index: i64, container: &[u8]) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.write_i64_be(index);
        let mut bytes = writer.into_bytes();
        bytes.extend_from_slice(container);
        // Empty block-entity table.
        bytes.push(0);
        bytes
    }

    /// A single-value stone section at `index`, with one block entity whose
    /// compressed payload was produced with the shipped dictionary.
    fn entity_section(index: i64, offset: u16, serialized: &[u8]) -> Vec<u8> {
        let compressed = zstd::bulk::Compressor::with_dictionary(3, BLOCK_ENTITY_DICT)
            .unwrap()
            .compress(serialized)
            .unwrap();
        let mut writer = Writer::new();
        writer.write_i64_be(index);
        writer.write_byte(0); // bits = 0: single value
        writer.write_var_int(STONE as i32);
        writer.write_var_int(1); // one block entity
        writer.write_i16_be(offset as i16);
        writer.write_var_int(serialized.len() as i32);
        writer.write_byte(0); // dict id 0
        writer.write_var_int(compressed.len() as i32);
        let mut bytes = writer.into_bytes();
        bytes.extend_from_slice(&compressed);
        bytes
    }

    fn container(bits: u8, entries: &[u32], data: &[u64]) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.write_byte(bits);
        if bits == 0 {
            writer.write_var_int(entries[0] as i32);
        } else if bits <= 8 {
            writer.write_var_int(entries.len() as i32);
            for entry in entries {
                writer.write_var_int(*entry as i32);
            }
        }
        for long in data {
            writer.write_i64_be(*long as i64);
        }
        writer.into_bytes()
    }

    fn terminated(section: Vec<u8>) -> Vec<u8> {
        let mut bytes = section;
        bytes.extend_from_slice(&MIN_POSITION_LONG.to_be_bytes());
        bytes
    }

    #[test]
    fn single_value_palette_decodes() {
        let bytes = terminated(section_with(0, &container(0, &[STONE], &[])));
        let mut reader = Reader::new(&bytes);
        let buffer = BlockBuffer::load(&mut reader).unwrap();
        assert_eq!(buffer.section_count(), 1);
        let changes: Vec<_> = buffer.sections[0].changes().collect();
        assert_eq!(changes.len(), 4096);
        assert!(changes.iter().all(|(_, _, _, state)| state.as_u16() == STONE as u16));
        assert_eq!((changes[0].0, changes[0].1, changes[0].2), (0, 0, 0));
        assert_eq!((changes[4095].0, changes[4095].1, changes[4095].2), (15, 15, 15));
    }

    #[test]
    fn indirect_palette_reads_packed_storage() {
        // bits=4: 16 entries per long, 256 longs for 4096 entries. Palette
        // [stone, dirt]: everything stone except entry 5 (palette index 1).
        let dirt = STONE + 1;
        let mut data = vec![0u64; 256];
        data[0] = 1 << 20; // entry 5 sits at bits 20..24
        let bytes = terminated(section_with(0x1000, &container(4, &[STONE, dirt], &data)));
        let mut reader = Reader::new(&bytes);
        let buffer = BlockBuffer::load(&mut reader).unwrap();
        let changes: Vec<_> = buffer.sections[0].changes().collect();
        assert_eq!(changes.len(), 4096);
        assert_eq!(changes[5].3.as_u16(), dirt as u16);
        assert_eq!(changes[6].3.as_u16(), STONE as u16);
    }

    #[test]
    fn void_air_entries_are_untouched() {
        // void_air decodes to None: the section exists but every position
        // is "leave alone".
        let void_air = u32::from(pumpkin_data::Block::VOID_AIR.default_state.id.as_u16());
        let bytes = terminated(section_with(0, &container(0, &[void_air], &[])));
        let mut reader = Reader::new(&bytes);
        let buffer = BlockBuffer::load(&mut reader).unwrap();
        assert_eq!(buffer.sections[0].changes().count(), 0);
    }

    #[test]
    fn unknown_palette_entry_is_decode_error() {
        // 100_000 is far outside the global block-state registry.
        let bytes = terminated(section_with(0, &container(0, &[100_000], &[])));
        let mut reader = Reader::new(&bytes);
        assert!(matches!(
            BlockBuffer::load(&mut reader),
            Err(BufferError::InvalidState(100_000))
        ));
    }

    #[test]
    fn oversized_indirect_palette_is_rejected() {
        // bits=4 can index at most 16 entries; 17 must be refused before
        // allocating.
        let mut writer = Writer::new();
        writer.write_byte(4).write_var_int(17);
        let bytes = terminated(section_with(0, &writer.into_bytes()));
        let mut reader = Reader::new(&bytes);
        assert!(matches!(
            BlockBuffer::load(&mut reader),
            Err(BufferError::PaletteTooLarge(17))
        ));
    }

    #[test]
    fn block_entity_roundtrip() {
        let mut nbt = NbtCompound::new();
        nbt.put_string("Text1", "hello".to_string());
        let mut serialized = Vec::new();
        Nbt::new(String::new(), nbt)
            .write_to_writer(&mut serialized)
            .unwrap();

        let bytes = terminated(entity_section(0, 0x123, &serialized));
        let mut reader = Reader::new(&bytes);
        let buffer = BlockBuffer::load(&mut reader).unwrap();
        let table = buffer.block_entities.get(&0).unwrap();
        let entity = table.get(&0x123).unwrap();
        let decoded = entity.decompress().unwrap();
        assert_eq!(decoded.get_string("Text1").unwrap(), "hello");
    }

    #[test]
    fn truncated_inputs_never_panic() {
        // Every prefix of a valid buffer must decode to an error (never
        // panic); garbage must do the same.
        let void_air = u32::from(pumpkin_data::Block::VOID_AIR.default_state.id.as_u16());
        let bytes = terminated(section_with(0, &container(0, &[void_air], &[])));
        for end in 0..bytes.len() {
            let mut reader = Reader::new(&bytes[..end]);
            let _ = BlockBuffer::load(&mut reader);
        }
        let mut seed: u64 = 0xDEAD_BEEF_CAFE_F00D;
        for _ in 0..256 {
            let len = (seed % 512) as usize + 1;
            seed ^= seed << 13;
            seed ^= seed >> 7;
            let bytes: Vec<u8> = (0..len).map(|i| ((seed >> (i % 8)) & 0xFF) as u8).collect();
            let mut reader = Reader::new(&bytes);
            let _ = BlockBuffer::load(&mut reader);
        }
    }

    #[test]
    fn min_position_long_matches_upstream() {
        // The upstream static block asserts bits 63, 37 and 11 set.
        assert_eq!(
            MIN_POSITION_LONG as u64,
            (1u64 << 63) | (1u64 << 37) | (1u64 << 11)
        );
        // Every packable section long is strictly greater, so the sentinel
        // can never collide with a real section.
        assert!(crate::wire::pack_block_pos(-33_554_431, -2047, -33_554_431) > MIN_POSITION_LONG);
        assert!(crate::wire::pack_block_pos(0, 0, 0) > MIN_POSITION_LONG);
    }
}
