//! Memory budget loop for huge edits: a full `//set`-style edit pipeline
//! (positions -> desired -> diff -> history -> apply) over a large region
//! must keep the process peak RSS within a fixed budget.
//!
//! Reproduces the reported bug: a big `//set` eats every byte of RAM and
//! freezes or kills the server/container.
//!
//! Run (release only, needs /proc — i.e. Linux/WSL):
//! `cargo test -p carrot-worldedit --test edit_memory --release -- --ignored --nocapture`

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::unwrap_used
)]

mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use carrot_worldedit::edit::{ChunkLease, apply_changes, compute_changes, ensure_chunks};
use carrot_worldedit::history::HistoryStore;
use carrot_worldedit::pattern::Pattern;
use carrot_worldedit::region::Cuboid;
use carrot_worldedit::states::parse_state_str;
use pumpkin_data::dimension::Dimension;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

/// Edit footprint: 384x384 columns x 164 layers = ~24.2M blocks over
/// 24x24 chunks, high above terrain so every changed block is air ->
/// stone (worst sky-light case).
const SPAN_XZ: i32 = 384;
const Y_MIN: i32 = 168;
const Y_MAX: i32 = 331;

/// Peak RSS the whole edit pipeline may add to the process. The current
/// fully-materialized pipeline needs multiple GB here; the incremental
/// queue must stay below this budget (page-sized buffers + one slice of
/// resident chunks).
const MEMORY_BUDGET_BYTES: u64 = 700 * 1024 * 1024;

/// A tick longer than this counts as "server stopped responding".
const TICK_FREEZE_LIMIT: Duration = Duration::from_millis(250);

/// Resets the process peak-RSS high-water mark to the current RSS
/// (Linux `clear_refs` 5). The next `peak_rss_bytes` reading therefore
/// measures only the edit pipeline.
fn reset_peak_rss() {
    let written = std::fs::write("/proc/self/clear_refs", b"5").is_ok();
    assert!(
        written,
        "this test needs /proc (run under Linux/WSL, release build)"
    );
}

/// The process peak RSS since the last [`reset_peak_rss`], in bytes.
fn peak_rss_bytes() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("reads /proc/self/status");
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kb: u64 = rest
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse()
                .expect("VmHWM is a kB number");
            return kb * 1024;
        }
    }
    panic!("VmHWM missing from /proc/self/status");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "memory: minutes and needs /proc; run in release under WSL"]
async fn huge_edit_peaks_within_memory_budget() {
    let temp = tempfile::TempDir::new().unwrap();
    // `Server::new` and `VanillaData::load` resolve everything (world,
    // data/) from the current directory; this test binary owns the process.
    std::env::set_current_dir(temp.path()).unwrap();

    let server = common::boot_server().await;
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);

    // Ticker at 20 TPS like production, so chunk unload can reclaim
    // released chunks mid-edit (the incremental pipeline depends on it)
    // and tick responsiveness is verified alongside memory.
    let ticker = common::Ticker::start(&server);

    let stone = parse_state_str("stone").expect("stone parses");
    let region = Cuboid::new(
        BlockPos(Vector3::new(0, Y_MIN, 0)),
        BlockPos(Vector3::new(SPAN_XZ - 1, Y_MAX, SPAN_XZ - 1)),
    );
    let total_blocks = region.volume().expect("volume fits u64");
    let pattern = Pattern::Single(stone);

    let mut history_root = std::env::temp_dir();
    history_root.push(format!("cwh-mem-{}", std::process::id()));
    let mut store = HistoryStore::open(&history_root, uuid::Uuid::new_v4(), 15, 7);

    // --- the production edit pipeline, as `//set` runs it today ---
    reset_peak_rss();
    let edit_start = Instant::now();

    let desired = pattern.apply_all(region.iterate(), None);
    let _lease = ChunkLease::acquire(&world.level, &desired, |item| &item.0);
    ensure_chunks(&world, &desired, |item: &(BlockPos, _)| &item.0).await;
    let changes = compute_changes(&world, &desired, None, None).await;
    drop(desired);
    apply_changes(&world, &changes, &HashMap::new()).await;

    // Correctness: sampled positions must read back as stone right
    // after apply, while the lease still pins the edited chunks (the
    // unload/save cycle after release is a separate concern).
    let mut missing = 0usize;
    let mut sampled = 0usize;
    for i in 0..24_000usize {
        let pos = BlockPos(Vector3::new(
            ((i * 131 + 7) % SPAN_XZ as usize) as i32,
            Y_MIN + ((i * 89 + 3) % (Y_MAX - Y_MIN + 1) as usize) as i32,
            ((i * 197 + 11) % SPAN_XZ as usize) as i32,
        ));
        sampled += 1;
        if world.get_block_state_id_async(&pos).await != stone {
            missing += 1;
        }
    }

    store.record(&changes).expect("history records");
    drop(changes);

    let edit_elapsed = edit_start.elapsed();
    let peak = peak_rss_bytes();
    // ---------------------------------------------------------------

    let durations = ticker.stop();
    let max_tick = durations.iter().max().copied().unwrap_or(Duration::ZERO);

    server.shutdown().await;
    let _ = std::fs::remove_dir_all(&history_root);

    eprintln!(
        "edit: {total_blocks} blocks in {edit_elapsed:?}; peak RSS +{:.0} MiB (budget {} MiB); max tick {max_tick:?}; applied {}/{}",
        peak as f64 / (1024.0 * 1024.0),
        MEMORY_BUDGET_BYTES / (1024 * 1024),
        sampled - missing,
        sampled,
    );

    assert!(
        peak < MEMORY_BUDGET_BYTES,
        "edit of {total_blocks} blocks peaked at +{:.0} MiB (budget {} MiB)",
        peak as f64 / (1024.0 * 1024.0),
        MEMORY_BUDGET_BYTES / (1024 * 1024),
    );
    assert!(
        max_tick < TICK_FREEZE_LIMIT,
        "a server tick froze for {max_tick:?} (limit {TICK_FREEZE_LIMIT:?})",
    );
    assert_eq!(
        missing, 0,
        "{missing}/{sampled} sampled blocks were never applied",
    );
}
