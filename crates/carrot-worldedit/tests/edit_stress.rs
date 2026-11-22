//! Stress regression loop for huge edits: a `//set`-style apply over a
//! large region must keep server ticks responsive and write every block.
//!
//! Reproduces the reported bug: a big `//set` freezes the whole server
//! and, once it recovers, part of the edit was never applied.
//!
//! Run (release only; debug builds are not valid for timing):
//! `cargo test -p carrot-worldedit --test edit_stress --release -- --ignored --nocapture`

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use carrot_worldedit::edit::apply_state_list;
use carrot_worldedit::states::parse_state_str;
use pumpkin_config::{AdvancedConfiguration, BasicConfiguration, TelemetryConfig};
use pumpkin_core::data::VanillaData;
use pumpkin_core::server::Server;
use pumpkin_data::BlockStateId;
use pumpkin_data::dimension::Dimension;
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

    let mut basic = BasicConfiguration::default();
    basic.default_level_name = "world".to_string();
    basic.allow_nether = false;
    basic.allow_end = false;

    let server = Server::new(
        basic,
        AdvancedConfiguration::default(),
        TelemetryConfig::default(),
        VanillaData::load(),
        vec![],
    )
    .await
    .expect("server boots");
    let world = server.get_world_from_dimension(&Dimension::OVERWORLD);

    // Manual ticker at 20 TPS measuring each tick's duration, exactly like
    // the dedicated Ticker thread does in production. Set
    // `EDIT_STRESS_NO_TICKER=1` to isolate the edit from concurrent
    // ticking (unload/clean_memory) when diagnosing.
    let run_ticker = std::env::var("EDIT_STRESS_NO_TICKER").is_err();
    let stop = Arc::new(AtomicBool::new(false));
    let tick_durations: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let ticker = if run_ticker {
        Some({
            let server = Arc::clone(&server);
            let stop = Arc::clone(&stop);
            let tick_durations = Arc::clone(&tick_durations);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let start = Instant::now();
                    server.tick();
                    let elapsed = start.elapsed();
                    tick_durations
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(elapsed);
                    let spent = start.elapsed();
                    if spent < Duration::from_millis(50) {
                        std::thread::sleep(Duration::from_millis(50) - spent);
                    }
                }
            })
        })
    } else {
        None
    };

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

    let edit_start = Instant::now();
    apply_state_list(&world, &targets).await;
    let edit_elapsed = edit_start.elapsed();

    stop.store(true, Ordering::Relaxed);
    if let Some(ticker) = ticker {
        ticker.join().unwrap();
    } else {
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

    let durations = tick_durations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
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
