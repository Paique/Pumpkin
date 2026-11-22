//! Per-player edit history on disk: every recorded edit becomes one
//! gzipped NBT file, so undo/redo survives restarts (like FAWE's disk
//! history) without keeping whole regions in memory.

use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use pumpkin_data::BlockStateId;
use pumpkin_nbt::Nbt;
use pumpkin_nbt::NbtCompound;
use pumpkin_nbt::array_limit;
use pumpkin_nbt::deserializer::NbtReadHelperJava;
use pumpkin_nbt::nbt_compress::write_gzip_compound_tag_to_bytes;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use uuid::Uuid;

use crate::states::{parse_state_str, state_to_string};

/// One recorded edit: position, state before and state after.
pub type Change = (BlockPos, BlockStateId, BlockStateId);

/// Undo/redo stacks backed by files under `history/<uuid>/NNNNNNNN.cwh`.
pub struct HistoryStore {
    dir: PathBuf,
    undo_stack: VecDeque<u64>,
    redo_stack: VecDeque<u64>,
    next_seq: u64,
    capacity: usize,
}

impl HistoryStore {
    /// Opens (and prunes) the history of one player.
    ///
    /// Keeps at most `capacity` edit files and drops files older than
    /// `retention_days` days, mirroring the reference server settings
    /// (15 edits, 7 days).
    #[must_use]
    pub fn open(root: &Path, uuid: Uuid, capacity: usize, retention_days: u32) -> Self {
        let dir = root.join(uuid.to_string());
        let mut seqs: Vec<u64> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let path = entry.path();
                        if path.extension().is_some_and(|ext| ext == "cwh") {
                            path.file_stem()?.to_str()?.parse().ok()
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        seqs.sort_unstable();

        let mut store = Self {
            dir,
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            next_seq: seqs.last().map_or(0, |seq| seq + 1),
            capacity,
        };

        if retention_days > 0 {
            let cutoff = std::time::SystemTime::now()
                - std::time::Duration::from_secs(u64::from(retention_days) * 86_400);
            seqs.retain(|seq| {
                let path = store.path_of(*seq);
                std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .map_or(true, |modified| modified >= cutoff)
            });
        }

        for seq in seqs {
            store.undo_stack.push_back(seq);
        }
        store.trim_old();
        store
    }

    /// Records an edit and clears the redo stack (files included).
    ///
    /// # Errors
    ///
    /// Fails when the history file cannot be written; the edit still
    /// happened, only its undo entry is lost.
    pub fn record(&mut self, changes: &[Change]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let seq = self.next_seq;
        self.next_seq += 1;
        std::fs::write(self.path_of(seq), serialize(changes)?)?;

        let stale: Vec<u64> = self.redo_stack.drain(..).collect();
        for seq in stale {
            let _ = std::fs::remove_file(self.path_of(seq));
        }
        self.undo_stack.push_back(seq);
        self.trim_old();
        Ok(())
    }

    /// The number of available undos.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// The number of available redos.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Pops the newest undo entry and moves it to the redo stack.
    pub fn pop_undo(&mut self) -> Option<Vec<Change>> {
        let seq = self.undo_stack.pop_back()?;
        let changes = self.load(seq);
        self.redo_stack.push_back(seq);
        changes
    }

    /// Pops the newest redo entry and moves it back to the undo stack.
    pub fn pop_redo(&mut self) -> Option<Vec<Change>> {
        let seq = self.redo_stack.pop_back()?;
        let changes = self.load(seq);
        self.undo_stack.push_back(seq);
        changes
    }

    fn load(&self, seq: u64) -> Option<Vec<Change>> {
        let bytes = std::fs::read(self.path_of(seq)).ok()?;
        deserialize(&bytes)
    }

    fn path_of(&self, seq: u64) -> PathBuf {
        self.dir.join(format!("{seq:08}.cwh"))
    }

    fn trim_old(&mut self) {
        while self.undo_stack.len() > self.capacity {
            if let Some(seq) = self.undo_stack.pop_front() {
                let _ = std::fs::remove_file(self.path_of(seq));
            }
        }
    }
}

fn serialize(changes: &[Change]) -> std::io::Result<Vec<u8>> {
    let mut palette: Vec<String> = Vec::new();
    let mut lookup: HashMap<String, i32> = HashMap::new();
    let mut xs = Vec::with_capacity(changes.len());
    let mut ys = Vec::with_capacity(changes.len());
    let mut zs = Vec::with_capacity(changes.len());
    let mut from = Vec::with_capacity(changes.len());
    let mut to = Vec::with_capacity(changes.len());

    for (pos, before, after) in changes {
        let before_name = state_to_string(*before);
        let after_name = state_to_string(*after);
        let before_id = palette_index(&mut palette, &mut lookup, &before_name);
        let after_id = palette_index(&mut palette, &mut lookup, &after_name);
        xs.push(pos.0.x);
        ys.push(pos.0.y);
        zs.push(pos.0.z);
        from.push(before_id);
        to.push(after_id);
    }

    let mut root = NbtCompound::new();
    root.put_int("v", 1);
    root.put_list(
        "palette",
        palette
            .into_iter()
            .map(|name| NbtTag::String(name.into_boxed_str()))
            .collect(),
    );
    root.put("x", NbtTag::IntArray(xs));
    root.put("y", NbtTag::IntArray(ys));
    root.put("z", NbtTag::IntArray(zs));
    root.put("f", NbtTag::IntArray(from));
    root.put("t", NbtTag::IntArray(to));
    write_gzip_compound_tag_to_bytes(root).map_err(|err| std::io::Error::other(err.to_string()))
}

fn palette_index(palette: &mut Vec<String>, lookup: &mut HashMap<String, i32>, name: &str) -> i32 {
    let next = palette.len() as i32;
    let id = *lookup.entry(name.to_string()).or_insert(next);
    if id == next {
        palette.push(name.to_string());
    }
    id
}

fn deserialize(bytes: &[u8]) -> Option<Vec<Change>> {
    // Decodes without `read_gzip_compound_tag`: its 64 MiB take() would
    // truncate a large edit's own history file (a 5M-block confirm edit
    // legitimately decompresses past that), and inside `unbounded` because
    // the coordinate arrays exceed `MAX_ARRAY_LENGTH`. These bytes were
    // written by this module, so no untrusted input reaches either path.
    let mut buf = Vec::new();
    GzDecoder::new(Cursor::new(bytes))
        .read_to_end(&mut buf)
        .ok()?;
    array_limit::unbounded(|| {
        let mut reader = NbtReadHelperJava::new(Cursor::new(buf));
        let root = Nbt::read(&mut reader).ok()?.root_tag;

        let palette: Vec<BlockStateId> = root
            .get_list("palette")?
            .iter()
            .filter_map(|tag| match tag {
                NbtTag::String(name) => parse_state_str(name).ok(),
                _ => None,
            })
            .collect();
        let xs = root.get_int_array("x")?;
        let ys = root.get_int_array("y")?;
        let zs = root.get_int_array("z")?;
        let from = root.get_int_array("f")?;
        let to = root.get_int_array("t")?;

        let mut changes = Vec::with_capacity(xs.len());
        for index in 0..xs.len() {
            let Some(before) = palette.get(from.get(index).copied()? as usize).copied() else {
                continue;
            };
            let Some(after) = palette.get(to.get(index).copied()? as usize).copied() else {
                continue;
            };
            changes.push((
                BlockPos(Vector3::new(
                    *xs.get(index)?,
                    *ys.get(index)?,
                    *zs.get(index)?,
                )),
                before,
                after,
            ));
        }
        Some(changes)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> BlockStateId {
        parse_state_str(name).expect("valid state")
    }

    fn sample_changes() -> Vec<Change> {
        vec![
            (
                BlockPos(Vector3::new(1, 2, 3)),
                BlockStateId::AIR,
                state("stone"),
            ),
            (
                BlockPos(Vector3::new(4, 5, 6)),
                state("dirt"),
                state("stone"),
            ),
        ]
    }

    #[test]
    fn round_trips_through_nbt() {
        let original = sample_changes();
        let bytes = serialize(&original).expect("serializes");
        let loaded = deserialize(&bytes).expect("deserializes");
        assert_eq!(loaded.len(), original.len());
        assert_eq!(loaded[0], original[0]);
        assert_eq!(loaded[1], original[1]);
    }

    #[test]
    fn store_undo_redo_cycle() {
        let root = std::env::temp_dir().join(format!("cwh-test-{}", std::process::id()));
        let uuid = Uuid::new_v4();
        let mut store = HistoryStore::open(&root, uuid, 15, 7);
        store.record(&sample_changes()).expect("records");

        assert_eq!(store.undo_depth(), 1);
        let undo = store.pop_undo().expect("undo entry");
        assert_eq!(undo.len(), 2);
        assert_eq!(store.redo_depth(), 1);

        let redo = store.pop_redo().expect("redo entry");
        assert_eq!(redo.len(), 2);
        assert_eq!(store.undo_depth(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn record_clears_redo() {
        let root = std::env::temp_dir().join(format!("cwh-test-{}-b", std::process::id()));
        let uuid = Uuid::new_v4();
        let mut store = HistoryStore::open(&root, uuid, 15, 7);
        store.record(&sample_changes()).expect("records");
        store.pop_undo().expect("undo");
        store.record(&sample_changes()).expect("records again");
        assert_eq!(store.redo_depth(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reads_flat_v1_files() {
        let original = sample_changes();
        let mut root = NbtCompound::new();
        root.put_int("v", 1);
        root.put_list(
            "palette",
            vec![
                NbtTag::String("minecraft:air".into()),
                NbtTag::String("minecraft:stone".into()),
                NbtTag::String("minecraft:dirt".into()),
            ],
        );
        root.put("x", NbtTag::IntArray(vec![1, 4]));
        root.put("y", NbtTag::IntArray(vec![2, 5]));
        root.put("z", NbtTag::IntArray(vec![3, 6]));
        root.put("f", NbtTag::IntArray(vec![0, 2]));
        root.put("t", NbtTag::IntArray(vec![1, 1]));
        let bytes = write_gzip_compound_tag_to_bytes(root).expect("payload serializes");

        let loaded = deserialize(&bytes).expect("file loads");
        assert_eq!(loaded.len(), original.len());
        assert_eq!(loaded[0], original[0]);
        assert_eq!(loaded[1], original[1]);
    }

    #[test]
    fn round_trips_past_the_array_guard() {
        // 513k entries exceed pumpkin-nbt's MAX_ARRAY_LENGTH (512_000);
        // before the unbounded decode this loaded as nothing and undo
        // answered "Desfeitas 0 edição(ões)".
        let before = parse_state_str("stone").expect("valid state");
        let after = parse_state_str("dirt").expect("valid state");
        let changes: Vec<Change> = (0..513_000)
            .map(|i| {
                (
                    BlockPos(Vector3::new(i % 1000, i / 1000, -(i as i32))),
                    before,
                    after,
                )
            })
            .collect();

        let bytes = serialize(&changes).expect("serializes");
        let loaded = deserialize(&bytes).expect("deserializes past the guard");
        assert_eq!(loaded.len(), changes.len());
        assert_eq!(loaded[0], changes[0]);
        assert_eq!(loaded[512_000], changes[512_000]);
        assert_eq!(loaded[changes.len() - 1], changes[changes.len() - 1]);
    }
}
