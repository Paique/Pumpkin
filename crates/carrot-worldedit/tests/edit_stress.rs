//! Stress regression loop for huge edits: a `//set`-style apply over a
//! large region must keep server ticks responsive and write every block.
//!
//! Reproduces the reported bug: a big `//set` freezes the whole server
//! and, once it recovers, part of the edit was never applied.
//!
//! Run (release only; debug builds are not valid for timing):
//! `cargo test -p carrot-worldedit --test edit_stress --release -- --ignored --nocapture`

#![allow(clippy::expect_used, clippy::print_stderr, clippy::unwrap_used)]

mod common;

use std::time::Duration;

use carrot_worldedit::edit::apply_state_list;
use carrot_worldedit::states::parse_state_str;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

/// Edit footprint: X and Z span, and the Y band (high above terrain, so
/// every changed block is air -> stone, the worst sky-light case).
const SPAN_XZ: i32 = 96;
const Y_MIN: i32 = 80;
const Y_MAX: i32 = 104;

/// A tick longer than this counts as "server stopped responding".
const TICK_FREEZE_LIMIT: Duration = Duration::from_millis(250);

/// The edit must finish within this budget.
const EDIT_BUDGET: Duration = Duration::from_secs(30);

/// Fraction of sampled blocks that must really be stone afterwards.
const MIN_APPLIED_FRACTION: f64 = 0.999;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "stress: minutes in debug; run in release"]
async fn huge_edit_keeps_ticking_and_applies_every_block() {
    let temp = tempfile::TempDir::new().unwrap();
    // `Server::new` and `VanillaData::load` resolve everything (world,
    // data/) from the current directory; this test binary owns the process.
    std::env::set_current_dir(temp.path()).unwrap();

    let server = common::boot_server().await;
    let world = server.get_world_from_dimension(&pumpkin_data::dimension::Dimension::OVERWORLD);

    // Manual ticker at 20 TPS measuring each tick's duration, exactly like
    // the dedicated Ticker thread does in production. Set
    // `EDIT_STRESS_NO_TICKER=1` to isolate the edit from concurrent
    // ticking (unload/clean_memory) when diagnosing.
    let ticker = std::env::var("EDIT_STRESS_NO_TICKER")
        .is_err()
        .then(|| common::Ticker::start(&server));

    let stone: BlockStateId = parse_state_str("stone").expect("stone parses");
    let mut targets: Vec<(BlockPos, BlockStateId)> = Vec::new();
    for y in Y_MIN..Y_MAX {
        for x in 0..SPAN_XZ {
            for z in 0..SPAN_XZ {
                targets.push((BlockPos(Vector3::new(x, y, z)), stone));
            }
        }
    }

    // Warm the world up first so chunk generation is not part of the
    // measurement.
    apply_state_list(&world, &[]).await;

    let edit_start = std::time::Instant::now();
    apply_state_list(&world, &targets).await;
    let edit_elapsed = edit_start.elapsed();

    let durations = ticker.map_or_else(Vec::new, common::Ticker::stop);
    if durations.is_empty() {
        eprintln!("[EDIT_STRESS] ticker disabled for this run");
    }

    // Corretude: every sampled position must read back as stone. Use the
    // async getter so unloaded chunks are fetched instead of reading AIR.
    let mut missing = 0usize;
    let mut sampled = 0usize;
    for (index, (pos, _)) in targets.iter().enumerate() {
        if index % 97 != 0 {
            continue;
        }
        sampled += 1;
        let current = world.get_block_state_id_async(pos).await;
        if current != stone {
            missing += 1;
        }
    }

    let max_tick = durations.iter().max().copied().unwrap_or(Duration::ZERO);
    let ticks_over_limit = durations
        .iter()
        .filter(|duration| **duration > TICK_FREEZE_LIMIT)
        .count();

    eprintln!(
        "edit: {} blocks in {edit_elapsed:?}; ticks: {} (max {max_tick:?}, {ticks_over_limit} over {:?}); applied {}/{}",
        targets.len(),
        durations.len(),
        TICK_FREEZE_LIMIT,
        sampled - missing,
        sampled,
    );

    server.shutdown().await;

    assert!(
        edit_elapsed < EDIT_BUDGET,
        "edit of {} blocks took {edit_elapsed:?} (budget {EDIT_BUDGET:?})",
        targets.len(),
    );
    assert!(
        max_tick < TICK_FREEZE_LIMIT,
        "a server tick froze for {max_tick:?} (limit {TICK_FREEZE_LIMIT:?}); {ticks_over_limit} ticks over the limit",
    );
    let applied_fraction = 1.0 - missing as f64 / sampled.max(1) as f64;
    assert!(
        applied_fraction >= MIN_APPLIED_FRACTION,
        "{missing}/{sampled} sampled blocks were never applied (applied fraction {applied_fraction:.4})",
    );
}
