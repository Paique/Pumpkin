//! Shared runtime state of the chat module.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Mutex, PoisonError, RwLock as StdRwLock},
    time::Instant,
};

use uuid::Uuid;

use crate::config::ChatConfig;

/// Which channel a player talks in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// The player's own world, within the local distance.
    Local,
    /// Every player on the server.
    Global,
}

impl Channel {
    /// The channel name shown to players.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Global => "global",
        }
    }
}

/// Everything the chat module shares between the event handler and the
/// channel commands.
pub struct ChatState {
    config: StdRwLock<ChatConfig>,
    data_folder: StdRwLock<PathBuf>,
    /// Players currently talking globally (absent = local, the reference
    /// default). Channel focus is transient; a restart returns to local.
    global_focus: Mutex<HashSet<Uuid>>,
    /// Last message instant per player, for channel cooldowns.
    last_message: Mutex<HashMap<Uuid, Instant>>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatState {
    /// Creates an empty state; [`ChatState::reload`] fills it on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: StdRwLock::new(ChatConfig::default()),
            data_folder: StdRwLock::new(PathBuf::new()),
            global_focus: Mutex::new(HashSet::new()),
            last_message: Mutex::new(HashMap::new()),
        }
    }

    /// Reloads configuration from the data folder and drops transient state.
    pub fn reload(&self, data_folder: &std::path::Path) {
        let config = ChatConfig::load_or_seed(&data_folder.join("chat.toml"));
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = config;
        *self
            .data_folder
            .write()
            .unwrap_or_else(PoisonError::into_inner) = data_folder.to_path_buf();
        self.clear_transient();
    }

    /// Drops per-player state.
    pub fn clear_transient(&self) {
        self.global_focus
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.last_message
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Forgets a single player (used on leave).
    pub fn forget_player(&self, player: Uuid) {
        self.global_focus
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&player);
        self.last_message
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&player);
    }

    /// Runs a closure with the current config snapshot.
    pub fn with_config<T>(&self, f: impl FnOnce(&ChatConfig) -> T) -> T {
        let guard = self.config.read().unwrap_or_else(PoisonError::into_inner);
        f(&guard)
    }

    /// The channel the player is currently focused on.
    #[must_use]
    pub fn channel_of(&self, player: Uuid) -> Channel {
        if self
            .global_focus
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&player)
        {
            Channel::Global
        } else {
            Channel::Local
        }
    }

    /// Sets the channel the player is focused on.
    pub fn set_channel(&self, player: Uuid, channel: Channel) {
        let mut focus = self
            .global_focus
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match channel {
            Channel::Global => {
                focus.insert(player);
            }
            Channel::Local => {
                focus.remove(&player);
            }
        }
    }

    /// Checks the channel cooldown, returning the seconds left when the
    /// player must wait. Passing messages refresh the timer.
    pub fn check_cooldown(&self, player: Uuid, cooldown_secs: f64) -> Option<f64> {
        if cooldown_secs <= 0.0 {
            return None;
        }
        let now = Instant::now();
        let mut last = self
            .last_message
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(instant) = last.get(&player) {
            let elapsed = instant.elapsed().as_secs_f64();
            if elapsed < cooldown_secs {
                return Some(cooldown_secs - elapsed);
            }
        }
        last.insert(player, now);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_focus_round_trips() {
        let state = ChatState::new();
        let player = Uuid::nil();
        assert_eq!(state.channel_of(player), Channel::Local);
        state.set_channel(player, Channel::Global);
        assert_eq!(state.channel_of(player), Channel::Global);
        state.set_channel(player, Channel::Local);
        assert_eq!(state.channel_of(player), Channel::Local);
    }

    #[test]
    fn cooldown_elapses_instantly_when_disabled() {
        let state = ChatState::new();
        let player = Uuid::nil();
        assert!(state.check_cooldown(player, 0.0).is_none());
        assert!(state.check_cooldown(player, 0.0).is_none());
    }
}
