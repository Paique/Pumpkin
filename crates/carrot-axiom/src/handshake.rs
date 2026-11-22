//! Axiom handshake: probe, `axiom:hello` exchange, `axiom:enable`,
//! `axiom:goodbye` and post-enable initial packets (phase AX-0).
//!
//! The sequence mirrors `AxiomPaper.java` / `HelloPacketListener.java`
//! (API version 10): a 1-second probe per online player (20 ticks) starts
//! handshakes for permission holders, deactivates players who lost
//! permission, resends changed restrictions, and sends a one-shot goodbye to
//! players without `axiom.use`.

use std::sync::Arc;

use pumpkin_core::{entity::player::Player, net::DisconnectReason, server::Server};
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_util::text::TextComponent;

use crate::{
    channels,
    config::{AxiomConfig, VersionResolution},
    restrictions::Restrictions,
    state::AxiomState,
    wire::{Reader, Writer},
};

/// Axiom client API version negotiated by the handshake (Axiom 6.x).
pub const API_VERSION: i32 = 10;

/// Blueprint packet version announced in `axiom:enable` (constant upstream).
const BLUEPRINT_VERSION: i32 = 2;

/// Server config version announced in `axiom:enable` (constant upstream).
const SERVER_CONFIG_VERSION: u8 = 0;

/// Goodbye sent once to players who will never be offered a handshake.
const NO_PERMISSION_MESSAGE: &str = "You don't have permission to use Axiom. Give yourself /op or axiom.default (if using a permission plugin)";

/// Serverbound channels this phase accepts. AX-0 only exchanges the
/// handshake; later phases extend this list (and the client will not send
/// anything else meanwhile, which keeps the protocol honest).
const SUPPORTED_SERVERBOUND: &[&str] = &[channels::HELLO];

/// Serializes the `axiom:enable` payload.
///
/// Layout (upstream `AxiomPaper.java`): bool enabled · byte server-config
/// version · `VarInt` blueprint version · `Int` `tunnelSplitSize` · `Int`
/// `maximumTunnelPacketSize` · `VarInt` `blockWithCustomData` count (0) ·
/// `VarInt` `ignoreRotationSet` count (0) · `VarInt` count + channel
/// identifiers.
#[must_use]
pub fn build_enable_payload(config: &AxiomConfig) -> Vec<u8> {
    let mut writer = Writer::new();
    writer
        .write_bool(true)
        .write_byte(SERVER_CONFIG_VERSION)
        .write_var_int(BLUEPRINT_VERSION)
        .write_i32_be(config.tunnel_split_size)
        .write_i32_be(config.maximum_tunnel_packet_size)
        .write_var_int(0)
        .write_var_int(0)
        .write_var_int(SUPPORTED_SERVERBOUND.len() as i32);
    for channel in SUPPORTED_SERVERBOUND {
        writer.write_string(channel);
    }
    writer.into_bytes()
}

/// Sends a custom payload to a Java client (Axiom is Java-only; Bedrock
/// players are silently skipped).
async fn send_payload(player: &Arc<Player>, channel: &str, data: &[u8]) {
    if let Some(java) = player.as_java() {
        java.send_custom_payload(channel, data).await;
    }
}

/// Sends `axiom:goodbye` with a textual reason.
async fn send_goodbye(player: &Arc<Player>, reason: &str) {
    let mut writer = Writer::new();
    writer.write_string(reason);
    send_payload(player, channels::GOODBYE, &writer.into_bytes()).await;
}

/// Disconnects a player with a message (protocol-level kick).
fn kick(player: &Arc<Player>, message: &str) {
    player.kick(
        DisconnectReason::Unknown,
        &TextComponent::text(message.to_string()),
    );
}

/// Generates a non-zero handshake id (0 is the "no pending handshake"
/// sentinel upstream).
fn next_handshake_id() -> i64 {
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    if let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) {
        hasher.write_u64(elapsed.as_nanos() as u64);
    }
    let id = hasher.finish() as i64;
    if id == 0 {
        1
    } else {
        id
    }
}

/// One probe tick for one player (upstream `AxiomPaper.probePlayer`).
pub async fn probe_player(state: &AxiomState, server: &Arc<Server>, player: &Arc<Player>) {
    let uuid = player.gameprofile.id;
    let allowed = crate::has_use(server, player);

    if state.with_player(uuid, |player| player.active) {
        if allowed {
            update_restrictions(state, server, player).await;
        } else {
            state.with_player(uuid, |player| {
                player.active = false;
                player.restrictions = None;
            });
            // A minimal `false` payload is the whole disable packet.
            send_payload(player, channels::ENABLE, &[0]).await;
        }
    } else if allowed {
        let pending = state.with_player(uuid, |player| player.pending_handshake_id);
        // Never resend while an id is outstanding; a late client reply must
        // still match the id it was given.
        if pending == 0 {
            let id = next_handshake_id();
            let mut writer = Writer::new();
            writer.write_i64_be(id);
            send_payload(player, channels::HELLO, &writer.into_bytes()).await;
            state.with_player(uuid, |player| {
                player.pending_handshake_id = id;
                player.sent_disable_reason = false;
            });
        }
    } else {
        let sent = state.with_player(uuid, |player| player.sent_disable_reason);
        if !sent {
            send_goodbye(player, NO_PERMISSION_MESSAGE).await;
            state.with_player(uuid, |player| player.sent_disable_reason = true);
        }
    }
}

/// Handles `axiom:hello` (client → server): validates versions, handshake id
/// and permission, then activates the client (upstream
/// `HelloPacketListener.onReceive`).
pub async fn handle_hello(
    state: &AxiomState,
    server: &Arc<Server>,
    player: &Arc<Player>,
    data: &[u8],
) {
    let config = state.config();
    let uuid = player.gameprofile.id;
    let mut reader = Reader::new(data);

    let api_version = match reader.read_var_int() {
        Ok(value) => value,
        Err(err) => {
            kick(player, &format!("Error while processing packet axiom:hello: {err}"));
            return;
        }
    };

    if api_version != API_VERSION {
        let reason = if api_version < API_VERSION {
            format!("Unable to use Axiom, you're on an outdated version! Please update to the latest version of Axiom to use it on this server. (C={api_version} S={API_VERSION})")
        } else {
            format!("Unable to use Axiom, server hasn't updated Axiom yet. (C={api_version} S={API_VERSION})")
        };
        send_goodbye(player, &reason).await;
        match config.unsupported_axiom_version_resolution {
            VersionResolution::Warn => {
                player.send_system_message(&TextComponent::text(reason));
                return;
            }
            VersionResolution::Kick => {
                kick(player, &reason);
                return;
            }
            VersionResolution::Ignore => {}
        }
    }

    let data_version = match reader.read_var_int() {
        Ok(value) => value,
        Err(err) => {
            kick(player, &format!("Error while processing packet axiom:hello: {err}"));
            return;
        }
    };
    let protocol_version = match reader.read_var_int() {
        Ok(value) => value,
        Err(err) => {
            kick(player, &format!("Error while processing packet axiom:hello: {err}"));
            return;
        }
    };
    let handshake_id = match reader.read_i64_be() {
        Ok(value) => value,
        Err(err) => {
            kick(player, &format!("Error while processing packet axiom:hello: {err}"));
            return;
        }
    };

    let pending = state.with_player(uuid, |player| player.pending_handshake_id);
    if pending == 0 || handshake_id != pending {
        send_goodbye(player, "Invalid handshake ID").await;
        return;
    }
    state.with_player(uuid, |player| player.pending_handshake_id = 0);

    if !crate::has_use(server, player) {
        send_goodbye(player, "Missing axiom.use permission").await;
        return;
    }

    // Without ViaVersion the client must run the server's protocol; the
    // message keeps the upstream "data version" wording.
    let current_protocol = CURRENT_MC_VERSION.protocol_version();
    if protocol_version != current_protocol {
        let reason = format!(
            "Axiom: Incompatible data version detected (client {data_version}, server {current_protocol})"
        );
        send_goodbye(player, &reason).await;
        match config.incompatible_data_version_resolution {
            VersionResolution::Warn => {
                player.send_system_message(&TextComponent::text(reason));
                return;
            }
            VersionResolution::Kick => {
                kick(player, &reason);
                return;
            }
            VersionResolution::Ignore => {}
        }
    }

    state.with_player(uuid, |player| {
        player.client_data_version = Some(data_version);
    });
    on_axiom_active(state, server, player).await;
}

/// Activates the client: enable, restrictions, and the post-enable initial
/// packets (upstream `AxiomPaper.onAxiomActive` + `sendInitialPackets`).
async fn on_axiom_active(state: &AxiomState, server: &Arc<Server>, player: &Arc<Player>) {
    let enable = state.enable_payload();
    send_payload(player, channels::ENABLE, &enable).await;
    state.with_player(player.gameprofile.id, |player| {
        player.active = true;
        player.sent_disable_reason = false;
        // Force a restrictions resend on (re)activation.
        player.restrictions = None;
    });
    update_restrictions(state, server, player).await;
    // Empty world-property registry (VarInt 0 categories) and custom-block
    // clear (VarInt 0) are always sent upstream.
    send_payload(player, channels::REGISTER_WORLD_PROPERTIES, &[0]).await;
    send_payload(player, channels::CUSTOM_BLOCKS, &[0]).await;
}

/// Recomputes restrictions and resends them only when they changed.
pub async fn update_restrictions(
    state: &AxiomState,
    server: &Arc<Server>,
    player: &Arc<Player>,
) {
    let fresh = Restrictions::calculate(server, player, &state.config());
    let changed = state.with_player(player.gameprofile.id, |entry| {
        if entry.restrictions.as_ref() == Some(&fresh) {
            false
        } else {
            entry.restrictions = Some(fresh.clone());
            true
        }
    });
    if changed {
        send_payload(player, channels::RESTRICTIONS, &fresh.encode()).await;
    }
}

/// The 20-tick probe loop (one second at full TPS).
pub async fn probe_task(server: Arc<Server>, state: Arc<AxiomState>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        for player in server.get_all_players() {
            probe_player(&state, &server, &player).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_payload_golden() {
        let bytes = build_enable_payload(&AxiomConfig::default());
        let expected: Vec<u8> = [
            // bool true, server config version 0, blueprint version 2
            &[0x01, 0x00, 0x02][..],
            // tunnel split size 31000 (BE int)
            &31_000i32.to_be_bytes(),
            // maximum tunnel packet size 2 MiB (BE int)
            &2_097_152i32.to_be_bytes(),
            // empty blockWithCustomData + ignoreRotationSet + count 1
            &[0x00, 0x00, 0x01],
            // Utf8 "axiom:hello"
            &[0x0B],
            b"axiom:hello",
        ]
        .concat();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn hello_request_roundtrip() {
        // varint 10, varint 4420, varint 771, long 0x1122334455667788
        let mut writer = Writer::new();
        writer.write_var_int(10).write_var_int(4420).write_var_int(771);
        writer.write_i64_be(0x1122_3344_5566_7788);
        let bytes = writer.into_bytes();
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.read_var_int().unwrap(), 10);
        assert_eq!(reader.read_var_int().unwrap(), 4420);
        assert_eq!(reader.read_var_int().unwrap(), 771);
        assert_eq!(reader.read_i64_be().unwrap(), 0x1122_3344_5566_7788);
    }

    #[test]
    fn goodbye_payload_is_single_string() {
        let mut writer = Writer::new();
        writer.write_string("Invalid handshake ID");
        let bytes = writer.into_bytes();
        let mut reader = Reader::new(&bytes);
        assert_eq!(
            reader.read_string().unwrap(),
            "Invalid handshake ID".to_string()
        );
    }

    #[test]
    fn handshake_ids_are_never_zero() {
        let mut all_nonzero = true;
        for _ in 0..64 {
            if next_handshake_id() == 0 {
                all_nonzero = false;
            }
        }
        assert!(all_nonzero);
    }
}
