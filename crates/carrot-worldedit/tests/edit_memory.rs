//! Memory budget loop for huge edits: the incremental edit queue
//! (diff to history pages slice by slice, apply page by page) must
//! keep the process peak RSS within a fixed budget.
//!
//! Reproduces the reported bug: a big `//set` eats every byte of RAM
//! and freezes or kills the server/container. The pre-queue pipeline
//! peaked at +1105 MiB on this exact footprint.
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

use std::sync::Arc;
use std::time::{Duration, Instant};

use carrot_worldedit::pattern::Pattern;
use carrot_worldedit::queue::run_region_edit_probe;
use carrot_worldedit::region::Cuboid;
use carrot_worldedit::session::WeState;
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

/// Peak RSS the whole edit pipeline may add to the process (page-sized
/// buffers, one slice of diff work, one page of resident chunks).
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

    // Ticker at 20 TPS like production, so chunk unload reclaims the
    // slices the queue releases mid-edit (the incremental pipeline
    // depends on it) and tick responsiveness is verified with memory.
    let ticker = common::Ticker::start(&server);

    // A real module state with a config that skips the //confirm gate
    // (the probe measures the full diff+apply run in one go).
    let data_folder = temp.path().join("modules").join("worldedit");
    std::fs::create_dir_all(&data_folder).unwrap();
    std::fs::write(
        data_folder.join("worldedit.toml"),
        "confirm_above = 9223372036854775807\nqueue_slice_blocks = 524288\n",
    )
    .unwrap();
    let state = Arc::new(WeState::new());
    state.configure(&data_folder);
    let uuid = uuid::Uuid::new_v4();

    let stone = parse_state_str("stone").expect("stone parses");
    let region = Cuboid::new(
        BlockPos(Vector3::new(0, Y_MIN, 0)),
        BlockPos(Vector3::new(SPAN_XZ - 1, Y_MAX, SPAN_XZ - 1)),
    );
    let total_blocks = region.volume().expect("volume fits u64");
    let pattern = Pattern::Single(stone);

    // --- the production edit pipeline, as `//set` runs it today ---
    reset_peak_rss();
    let edit_start = Instant::now();
    let changed = run_region_edit_probe(&state, uuid, &world, region, pattern)
        .await
        .expect("edit completes");
    let edit_elapsed = edit_start.elapsed();
    let peak = peak_rss_bytes();
    // ---------------------------------------------------------------

    // Correctness: sampled positions must read back as stone (async
    // getter refetches any chunk the ticker unloaded meanwhile).
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

    let durations = ticker.stop();
    let max_tick = durations.iter().max().copied().unwrap_or(Duration::ZERO);

    // The recorded history must cover the whole edit.
    let undo_pages: usize = {
        let session = state.session(uuid);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut reader = guard.history.pop_undo().expect("undo entry exists");
        let mut total = 0usize;
        while let Some(page) = reader.next_page().expect("page decodes") {
            total += page.len();
        }
        total
    };

    server.shutdown().await;

    eprintln!(
        "edit: {total_blocks} blocks changed {changed} in {edit_elapsed:?}; peak RSS +{:.0} MiB (budget {} MiB); max tick {max_tick:?}; applied {}/{}; undo pages {undo_pages}",
        peak as f64 / (1024.0 * 1024.0),
        MEMORY_BUDGET_BYTES / (1024 * 1024),
        sampled - missing,
        sampled,
    );

    assert_eq!(changed, total_blocks, "the queue applied every block");
    assert_eq!(
        undo_pages, total_blocks as usize,
        "history pages cover the edit"
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
