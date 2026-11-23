//! Per-player edit history on disk: every recorded edit becomes one
//! file of gzipped NBT pages, so undo/redo survives restarts (like
//! FAWE's disk history) without ever keeping a whole edit in memory.
//!
//! Format v2 (written since the incremental queue): `CWH2` magic,
//! version byte, then length-prefixed pages, each a self-contained
//! gzipped NBT compound (`v:1`, palette, x/y/z/f/t arrays). Reading is
//! page-by-page, so RAM stays O(page) no matter how large the edit.
//! Old v1 files (one gzip stream for the whole edit) are still read.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, BufWriter, Cursor, Read, Write};
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

/// One recorded edit entry: position, state before and state after.
pub type Change = (BlockPos, BlockStateId, BlockStateId);

/// Changes per history page. Kept under pumpkin-nbt's
/// `MAX_ARRAY_LENGTH` so page decoding never needs the unbounded
/// escape hatch, and small enough that a page in RAM is a few MB.
const PAGE_CHANGES: usize = 500_000;

/// v2 file header: magic plus format version.
const V2_HEADER: [u8; 5] = [b'C', b'W', b'H', b'2', 2];

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
    /// (15 edits, 7 days). Orphaned `.cwh.tmp` pages from aborted or
    /// crashed edits are removed here.
    #[must_use]
    pub fn open(root: &Path, uuid: Uuid, capacity: usize, retention_days: u32) -> Self {
        let dir = root.join(uuid.to_string());
        let _ = std::fs::create_dir_all(&dir);
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "tmp") {
                let _ = std::fs::remove_file(path);
            }
        }

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
        let mut writer = self.begin_record()?;
        for change in changes {
            writer.push(*change)?;
        }
        self.commit_record(writer)?;
        Ok(())
    }

    /// Starts a streaming record: pages are written as changes arrive,
    /// so a huge edit never lives in memory at once. Call
    /// [`Self::commit_record`] to finalize or [`Self::abort_record`] to
    /// discard.
    ///
    /// # Errors
    ///
    /// Fails when the temporary file cannot be created.
    pub fn begin_record(&mut self) -> std::io::Result<HistoryWriter> {
        HistoryWriter::create(&self.dir, self.next_seq).inspect(|_| self.next_seq += 1)
    }

    /// Finalizes a streaming record: flushes the last partial page,
    /// renames the file into place, clears the redo stack and pushes
    /// the entry onto the undo stack.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be finalized; the temporary file is
    /// removed.
    pub fn commit_record(&mut self, writer: HistoryWriter) -> std::io::Result<u64> {
        let (seq, total) = writer.finish()?;
        std::fs::rename(self.tmp_path_of(seq), self.path_of(seq))?;

        let stale: Vec<u64> = self.redo_stack.drain(..).collect();
        for seq in stale {
            let _ = std::fs::remove_file(self.path_of(seq));
        }
        self.undo_stack.push_back(seq);
        self.trim_old();
        Ok(total)
    }

    /// Discards a streaming record and deletes its temporary file.
    pub fn abort_record(&mut self, writer: HistoryWriter) {
        let seq = writer.seq();
        drop(writer);
        let _ = std::fs::remove_file(self.tmp_path_of(seq));
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

    /// Pops the newest undo entry and moves it to the redo stack,
    /// returning a page-by-page reader for its changes.
    pub fn pop_undo(&mut self) -> Option<HistoryReader> {
        let seq = self.undo_stack.pop_back()?;
        self.redo_stack.push_back(seq);
        HistoryReader::open(&self.path_of(seq)).ok()
    }

    /// Pops the newest redo entry and moves it back to the undo stack,
    /// returning a page-by-page reader for its changes.
    pub fn pop_redo(&mut self) -> Option<HistoryReader> {
        let seq = self.redo_stack.pop_back()?;
        self.undo_stack.push_back(seq);
        HistoryReader::open(&self.path_of(seq)).ok()
    }

    fn path_of(&self, seq: u64) -> PathBuf {
        self.dir.join(format!("{seq:08}.cwh"))
    }

    fn tmp_path_of(&self, seq: u64) -> PathBuf {
        self.dir.join(format!("{seq:08}.cwh.tmp"))
    }

    fn trim_old(&mut self) {
        while self.undo_stack.len() > self.capacity {
            if let Some(seq) = self.undo_stack.pop_front() {
                let _ = std::fs::remove_file(self.path_of(seq));
            }
        }
    }
}

/// Streaming v2 writer behind [`HistoryStore::begin_record`].
pub struct HistoryWriter {
    file: BufWriter<std::fs::File>,
    seq: u64,
    buf: Vec<Change>,
    total: u64,
}

impl HistoryWriter {
    fn create(dir: &Path, seq: u64) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut file = BufWriter::new(std::fs::File::create(
            dir.join(format!("{seq:08}.cwh.tmp")),
        )?);
        file.write_all(&V2_HEADER)?;
        Ok(Self {
            file,
            seq,
            buf: Vec::with_capacity(PAGE_CHANGES),
            total: 0,
        })
    }

    /// The sequence number this record will commit as.
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// Changes recorded so far.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.total
    }

    /// Appends one change, flushing a page whenever the page buffer
    /// fills.
    ///
    /// # Errors
    ///
    /// Fails when the page cannot be written to disk.
    pub fn push(&mut self, change: Change) -> std::io::Result<()> {
        self.buf.push(change);
        self.total += 1;
        if self.buf.len() == PAGE_CHANGES {
            self.flush_page()?;
        }
        Ok(())
    }

    fn flush_page(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let page = serialize_page(&self.buf)?;
        self.file.write_all(&(page.len() as u32).to_le_bytes())?;
        self.file.write_all(&page)?;
        self.buf.clear();
        Ok(())
    }

    /// Flushes the final partial page and closes the file, returning
    /// `(seq, total changes)`.
    ///
    /// # Errors
    ///
    /// Fails when the tail cannot be written.
    pub fn finish(mut self) -> std::io::Result<(u64, u64)> {
        self.flush_page()?;
        self.file.flush()?;
        Ok((self.seq, self.total))
    }
}

/// Page-by-page reader for one history file (v2 paginated or v1
/// legacy, read as a single page).
pub struct HistoryReader {
    v1: Option<Vec<Change>>,
    input: Option<BufReader<std::fs::File>>,
}

impl HistoryReader {
    /// Opens a history file, sniffing v2 pages vs a legacy v1 gzip
    /// stream.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is neither format.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut header = [0u8; 5];
        file.read_exact(&mut header)?;
        if header == V2_HEADER {
            Ok(Self {
                v1: None,
                input: Some(BufReader::new(file)),
            })
        } else if header.starts_with(&[0x1f, 0x8b]) {
            // Legacy v1: one gzip stream for the whole edit, loaded as a
            // single (unbounded) page.
            drop(file);
            let bytes = std::fs::read(path)?;
            let changes = deserialize_page(&bytes)
                .ok_or_else(|| std::io::Error::other("invalid v1 history file"))?;
            Ok(Self {
                v1: Some(changes),
                input: None,
            })
        } else {
            Err(std::io::Error::other("unrecognized history file"))
        }
    }

    /// The next page of changes, or `None` once the file is exhausted.
    /// Each page holds at most [`PAGE_CHANGES`] entries, so callers
    /// never hold a whole edit in memory (legacy v1 files load whole,
    /// as they were written).
    ///
    /// # Errors
    ///
    /// Fails when the next page cannot be read or decoded.
    pub fn next_page(&mut self) -> std::io::Result<Option<Vec<Change>>> {
        if let Some(changes) = self.v1.take() {
            return Ok(Some(changes));
        }
        let Some(input) = self.input.as_mut() else {
            return Ok(None);
        };
        let mut len_bytes = [0u8; 4];
        match read_exact_or_eof(input, &mut len_bytes)? {
            0 => return Ok(None),
            n if n < 4 => {
                return Err(std::io::Error::other(
                    "truncated page length in history file",
                ));
            }
            _ => {}
        }
        let len = u32::from_le_bytes(len_bytes) as usize;
        let mut page = vec![0u8; len];
        input.read_exact(&mut page)?;
        let changes =
            deserialize_page(&page).ok_or_else(|| std::io::Error::other("invalid history page"))?;
        Ok(Some(changes))
    }
}

/// Reads up to `buf.len()` bytes, returning how many were read (0 at a
/// clean EOF).
fn read_exact_or_eof(input: &mut impl BufRead, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = input.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

/// Serializes one page in the per-page NBT layout shared with v1.
fn serialize_page(changes: &[Change]) -> std::io::Result<Vec<u8>> {
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

/// Decodes one gzipped page payload (the compound layout shared with
/// v1). Pages stay under `MAX_ARRAY_LENGTH`, but legacy v1 files load
/// through here too and may exceed it, so the unbounded reader stays;
/// these bytes were written by this module.
fn deserialize_page(bytes: &[u8]) -> Option<Vec<Change>> {
    let mut raw = Vec::new();
    GzDecoder::new(Cursor::new(bytes))
        .read_to_end(&mut raw)
        .ok()?;
    array_limit::unbounded(|| {
        let mut reader = NbtReadHelperJava::new(Cursor::new(raw));
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

    fn changes_of(len: usize) -> Vec<Change> {
        let before = state("stone");
        let after = state("dirt");
        (0..len)
            .map(|i| {
                (
                    BlockPos(Vector3::new(
                        (i % 1000) as i32,
                        (i / 1000) as i32,
                        -(i as i32 / 7),
                    )),
                    before,
                    after,
                )
            })
            .collect()
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cwh-test-{label}-{}", std::process::id()))
    }

    #[test]
    fn streams_multi_page_round_trip() {
        let root = temp_root("pages");
        let uuid = Uuid::new_v4();
        let mut store = HistoryStore::open(&root, uuid, 15, 7);

        // 1.2M changes cross three pages (500k each).
        let changes = changes_of(1_200_000);
        let mut writer = store.begin_record().expect("writer");
        for change in &changes {
            writer.push(*change).expect("push");
        }
        assert_eq!(writer.total(), 1_200_000);
        store.commit_record(writer).expect("commit");

        let mut reader = store.pop_undo().expect("undo entry");
        let mut seen = 0usize;
        let mut pages = 0usize;
        let mut first: Option<Change> = None;
        while let Some(page) = reader.next_page().expect("page") {
            assert!(page.len() <= PAGE_CHANGES, "page exceeds the cap");
            if seen == 0 {
                first = page.first().copied();
            }
            seen += page.len();
            pages += 1;
        }
        assert_eq!(seen, 1_200_000);
        assert_eq!(pages, 3);
        assert_eq!(first, changes.first().copied());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn store_undo_redo_cycle() {
        let root = temp_root("cycle");
        let uuid = Uuid::new_v4();
        let mut store = HistoryStore::open(&root, uuid, 15, 7);
        store.record(&sample_changes()).expect("records");

        assert_eq!(store.undo_depth(), 1);
        let mut undo = store.pop_undo().expect("undo entry");
        let page = undo.next_page().expect("page").expect("non-empty");
        assert_eq!(page, sample_changes());
        assert!(undo.next_page().expect("page").is_none());
        assert_eq!(store.redo_depth(), 1);

        let mut redo = store.pop_redo().expect("redo entry");
        let page = redo.next_page().expect("page").expect("non-empty");
        assert_eq!(page, sample_changes());
        assert_eq!(store.undo_depth(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn record_clears_redo() {
        let root = temp_root("clears");
        let uuid = Uuid::new_v4();
        let mut store = HistoryStore::open(&root, uuid, 15, 7);
        store.record(&sample_changes()).expect("records");
        store.pop_undo().expect("undo");
        store.record(&sample_changes()).expect("records again");
        assert_eq!(store.redo_depth(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn abort_removes_the_temp_file() {
        let root = temp_root("abort");
        let uuid = Uuid::new_v4();
        let mut store = HistoryStore::open(&root, uuid, 15, 7);

        let mut writer = store.begin_record().expect("writer");
        for change in sample_changes() {
            writer.push(change).expect("push");
        }
        store.abort_record(writer);
        assert!(!store.tmp_path_of(0).exists());
        assert_eq!(store.undo_depth(), 0);
        assert!(!store.path_of(0).exists());

        // A reopen must not resurrect anything from the aborted edit.
        let store = HistoryStore::open(&root, uuid, 15, 7);
        assert_eq!(store.undo_depth(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_sweeps_orphan_tmp_files() {
        let root = temp_root("sweep");
        let uuid = Uuid::new_v4();
        let mut store = HistoryStore::open(&root, uuid, 15, 7);
        let writer = store.begin_record().expect("writer");
        drop(writer); // Simulates a crash before commit/rename.
        assert!(store.tmp_path_of(0).exists());

        let store = HistoryStore::open(&root, uuid, 15, 7);
        assert!(!store.tmp_path_of(0).exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reads_legacy_v1_files_as_one_page() {
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

        let path = temp_root("v1").join("legacy.cwh");
        std::fs::create_dir_all(path.parent().expect("parent")).unwrap();
        std::fs::write(&path, bytes).expect("writes v1 file");

        let mut reader = HistoryReader::open(&path).expect("opens v1");
        let page = reader.next_page().expect("page").expect("non-empty");
        assert_eq!(page, original);
        assert!(reader.next_page().expect("page").is_none());
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn pages_past_the_array_guard() {
        // Pages stay under pumpkin-nbt's MAX_ARRAY_LENGTH (512_000), so
        // a 513k edit records as two bounded pages instead of one array
        // the decoder used to silently drop.
        let root = temp_root("guard");
        let uuid = Uuid::new_v4();
        let mut store = HistoryStore::open(&root, uuid, 15, 7);

        let changes = changes_of(513_000);
        let mut writer = store.begin_record().expect("writer");
        for change in &changes {
            writer.push(*change).expect("push");
        }
        store.commit_record(writer).expect("commit");

        let mut reader = store.pop_undo().expect("undo entry");
        let first = reader.next_page().expect("page").expect("non-empty");
        assert_eq!(first.len(), PAGE_CHANGES);
        let second = reader.next_page().expect("page").expect("non-empty");
        assert_eq!(second.len(), 513_000 - PAGE_CHANGES);
        assert_eq!(first.first(), changes.first());
        assert_eq!(second.last(), changes.last());
        let _ = std::fs::remove_dir_all(root);
    }
}
