//! Shared harness for the server-backed integration tests: boots an
//! in-process server and runs a production-like 20 TPS ticker thread.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pumpkin_config::{AdvancedConfiguration, BasicConfiguration, TelemetryConfig};
use pumpkin_core::data::VanillaData;
use pumpkin_core::server::Server;
use pumpkin_data::dimension::Dimension;

/// Boots a single-world overworld server rooted at the process's
/// current directory.
pub async fn boot_server() -> Arc<Server> {
    let basic = BasicConfiguration {
        default_level_name: "world".to_string(),
        allow_nether: false,
        allow_end: false,
        ..BasicConfiguration::default()
    };

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
    let _ = world; // forces world creation before the ticker starts
    server
}

/// A 20 TPS ticker measuring each tick's duration, exactly like the
/// dedicated `Ticker` thread in production. Call [`Ticker::stop`] to
/// join and collect the measurements.
pub struct Ticker {
    handle: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    durations: Arc<Mutex<Vec<Duration>>>,
}

impl Ticker {
    /// Starts ticking `server` on its own thread.
    pub fn start(server: &Arc<Server>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let durations: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
        let handle = {
            let server = Arc::clone(server);
            let stop = Arc::clone(&stop);
            let durations = Arc::clone(&durations);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let start = Instant::now();
                    server.tick();
                    let elapsed = start.elapsed();
                    durations
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(elapsed);
                    let spent = start.elapsed();
                    if let Some(spare) = Duration::from_millis(50).checked_sub(spent) {
                        std::thread::sleep(spare);
                    }
                }
            })
        };
        Self {
            handle: Some(handle),
            stop,
            durations,
        }
    }

    /// Stops the ticker and returns every measured tick duration.
    pub fn stop(mut self) -> Vec<Duration> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
        self.durations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
