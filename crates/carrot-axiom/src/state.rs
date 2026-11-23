//! Per-player handshake state and module-wide shared state (the Rust
//! equivalent of upstream `AxiomPlayerState`).

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use uuid::Uuid;

use crate::{
    config::AxiomConfig,
    restrictions::Restrictions,
    tunnel::TunnelBuffer,
    wire::{Reader, Writer},
};

/// Handshake state for one player, created on join and dropped on quit.
#[derive(Debug, Default, Clone)]
pub struct PlayerState {
    /// The client completed the handshake and the editor is active.
    pub active: bool,
    /// Outstanding handshake id (0 = none pending).
    pub pending_handshake_id: i64,
    /// Whether the no-permission goodbye was already sent.
    pub sent_disable_reason: bool,
    /// Last restrictions payload sent (resent only when it changes).
    pub restrictions: Option<Restrictions>,
    /// Data version reported by the client handshake.
    pub client_data_version: Option<i32>,
    /// Tunnel reassembly in flight (phase AX-1).
    pub tunnel: TunnelBuffer,
    /// Chunk sections queued for application; shared with apply tasks
    /// (backpressure counter).
    pub pending_sections: Arc<AtomicUsize>,
    /// Dispatch token bucket, in twentieths of a send per tick.
    pub dispatch: DispatchBucket,
    /// When `pending_sections` first exceeded the soft cap (watchdog).
    pub pending_since: Option<Instant>,
    /// Whether the watchdog already warned during this excursion.
    pub pending_warned: bool,
}

/// Per-player dispatch budget (mirror of `DispatchBudget.java`; units are
/// twentieths of a send so one send drains per tick at the limit).
#[derive(Debug, Default, Clone)]
pub struct DispatchBucket {
    /// Bucket balance in 1/20-send units; can go negative when the client
    /// oversends.
    pub available_20ths: i64,
    /// When the lazy refill last ran.
    pub last_refill: Option<Instant>,
    /// Last whole-sends balance announced to the client.
    pub last_notified: i32,
}

impl DispatchBucket {
    /// Seeds a full bucket on activation and reports it as notified.
    pub fn seed(&mut self, allowed_per_second: u32) {
        self.available_20ths = i64::from(allowed_per_second) * 20;
        self.last_refill = None;
        self.last_notified = allowed_per_second as i32;
    }

    /// Lazily refills from elapsed time while below the cap and not
    /// suspended by backpressure.
    fn lazy_refill(&mut self, config: &AxiomConfig, pending: usize) {
        let allowed = config.block_buffer_rate_limit.max(1) as i64;
        let max20 = allowed * 20;
        if let Some(last) = self.last_refill
            && self.available_20ths < max20
            && pending <= config.max_pending_sections_per_player as usize
        {
            let refill = (last.elapsed().as_nanos() as i64).saturating_mul(max20) / 1_000_000_000;
            self.available_20ths = self.available_20ths.saturating_add(refill).min(max20);
        }
        self.last_refill = Some(Instant::now());
    }

    /// Consumes `sends`, refilling lazily from elapsed time while below the
    /// soft backpressure cap. Returns the kick reason when the client is
    /// sending far too fast (balance below minus one full bucket).
    pub fn consume(
        &mut self,
        config: &AxiomConfig,
        pending: usize,
        sends: usize,
        client_available: i32,
    ) -> Result<(), &'static str> {
        let allowed = config.block_buffer_rate_limit.max(1) as i64;
        let max20 = allowed * 20;
        self.lazy_refill(config, pending);

        // Debit, clamped to what the client claims it can afford.
        let client_20ths = i64::from(client_available.max(0)) * 20;
        let balance = (self.available_20ths - sends as i64 * 20).min(client_20ths);
        self.available_20ths = balance;
        if balance < -max20 {
            return Err("You are sending updates too fast!");
        }
        Ok(())
    }

    /// Probe-side recovery pass: refills without debiting and returns
    /// `(add, max)` when the whole-sends balance grew since the last
    /// announcement (upstream's periodic refill notification).
    pub fn recover(&mut self, config: &AxiomConfig, pending: usize) -> Option<(i32, i32)> {
        self.lazy_refill(config, pending);
        let allowed = config.block_buffer_rate_limit.max(1) as i32;
        let whole = self.whole_sends();
        (whole > self.last_notified).then(|| {
            let add = whole - self.last_notified;
            self.last_notified = whole;
            (add, allowed)
        })
    }

    /// The whole-sends balance (truncating, like Java's int division).
    #[must_use]
    pub const fn whole_sends(&self) -> i32 {
        (self.available_20ths / 20) as i32
    }
}

/// Serializes `axiom:update_available_dispatch_sends`.
#[must_use]
pub fn encode_dispatch_update(add: i32, max: i32) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.write_var_int(add).write_var_int(max);
    writer.into_bytes()
}

/// Reads `axiom:set_buffer`'s trailing client dispatch declaration.
#[must_use]
pub fn read_client_dispatch(reader: &mut Reader<'_>) -> i32 {
    reader.read_var_int().unwrap_or(0).max(0)
}

/// Module state shared between handlers and the probe task.
pub struct AxiomState {
    config: RwLock<AxiomConfig>,
    players: RwLock<HashMap<Uuid, PlayerState>>,
    enable_payload: RwLock<Vec<u8>>,
    probe_task: Mutex<Option<tokio::task::AbortHandle>>,
}

impl AxiomState {
    /// Creates empty state; configuration loads on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: RwLock::new(AxiomConfig::default()),
            players: RwLock::new(HashMap::new()),
            enable_payload: RwLock::new(Vec::new()),
            probe_task: Mutex::new(None),
        }
    }

    /// Reloads the configuration and rebuilds the cached `axiom:enable`
    /// payload. The payload is rebuilt so later phases can extend the
    /// supported-serverbound channel list from configuration.
    pub fn reload(&self, data_folder: &Path) {
        let config = AxiomConfig::load_or_seed(&data_folder.join("axiom.toml"));
        crate::blocks::configure(&config.disallowed_blocks);
        let enable = crate::handshake::build_enable_payload(&config);
        *self
            .config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
        *self
            .enable_payload
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = enable;
    }

    /// Returns a snapshot of the configuration.
    #[must_use]
    pub fn config(&self) -> AxiomConfig {
        self.config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns a copy of the cached `axiom:enable` payload.
    #[must_use]
    pub fn enable_payload(&self) -> Vec<u8> {
        self.enable_payload
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Runs `update` against one player's state; the player is created on
    /// first touch. Returns the update's value.
    pub fn with_player<T>(
        &self,
        uuid: Uuid,
        update: impl FnOnce(&mut PlayerState) -> T,
    ) -> T {
        let mut players = self
            .players
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        update(players.entry(uuid).or_default())
    }

    /// Removes a player's state (quit).
    pub fn forget_player(&self, uuid: Uuid) {
        self.players
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&uuid);
    }

    /// Returns the player's pending-sections counter handle (creating the
    /// player entry when absent).
    #[must_use]
    pub fn pending_sections(&self, uuid: Uuid) -> Arc<AtomicUsize> {
        self.with_player(uuid, |player| Arc::clone(&player.pending_sections))
    }

    /// One probe-side watchdog pass: tracks how long a player has been
    /// above the soft pending cap and logs once per excursion. Spawned
    /// apply tasks always release their sections (RAII), so this only
    /// observes; it never force-completes work.
    pub fn watchdog_tick(&self, uuid: Uuid, soft_cap: u32, timeout_secs: u32) {
        let pending = self.pending_sections(uuid).load(Ordering::Relaxed);
        let above = pending > soft_cap as usize;
        let due = self.with_player(uuid, |player| {
            if above {
                if player.pending_since.is_none() {
                    player.pending_since = Some(Instant::now());
                    player.pending_warned = false;
                }
                let due = !player.pending_warned
                    && player
                        .pending_since
                        .is_some_and(|detected| detected.elapsed().as_secs() >= u64::from(timeout_secs));
                if due {
                    player.pending_warned = true;
                }
                due
            } else {
                player.pending_since = None;
                player.pending_warned = false;
                false
            }
        });
        if due {
            tracing::warn!(
                "carrot-axiom: player {uuid} has {pending} sections pending for over \
                 {timeout_secs}s (watchdog)"
            );
        }
    }

    /// Drops all player state (module disable).
    pub fn clear(&self) {
        self.players
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let handle = self
            .probe_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            handle.abort();
        }
    }

    /// Stores the probe task handle so disable can abort it.
    pub fn set_probe_task(&self, handle: tokio::task::AbortHandle) {
        *self
            .probe_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
    }
}

impl Default for AxiomState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_state_defaults_to_inactive() {
        let state = AxiomState::new();
        let pending = state.with_player(Uuid::new_v4(), |player| {
            let pending = player.pending_handshake_id;
            player.pending_handshake_id = 42;
            pending
        });
        assert_eq!(pending, 0);
        let pending = state.with_player(Uuid::new_v4(), |player| player.pending_handshake_id);
        assert_eq!(pending, 0);
    }
}
