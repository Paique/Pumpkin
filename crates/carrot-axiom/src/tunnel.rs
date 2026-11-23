//! `axiom:tunnel` fragmentation, reassembly and internal packet decoding
//! (mirror of `AxiomPaper.handleTunnelBuffer`/`handleTunnelPacket`).
//!
//! Big client payloads travel as fragments under the vanilla 32 KiB
//! custom-payload cap: each fragment starts with a flags byte (`bit0` FIRST,
//! `bit1` LAST) followed by payload bytes. A reassembled (or single-fragment)
//! internal packet is `Utf8` destination channel · `Int` uncompressed size ·
//! byte packet flags (`bit0` = ZSTD) · payload, decompressed to exactly the
//! declared size when the ZSTD flag is set.
//!
//! Every guard mirrors upstream 1:1, including the quirk that fragments are
//! accounted with their flags byte and that a tunneled `axiom:tunnel`
//! destination is a protocol violation (nested tunnels are not allowed).

use crate::{channels, config::AxiomConfig, wire::Reader};

/// `AxiomConstants.TUNNEL_BUFFER_FLAG_FIRST`.
pub const FLAG_FIRST: u8 = 1;
/// `AxiomConstants.TUNNEL_BUFFER_FLAG_LAST`.
pub const FLAG_LAST: u8 = 2;
/// `AxiomConstants.TUNNEL_PACKET_FLAG_ZSTD_COMPRESSED`.
pub const PACKET_FLAG_ZSTD: u8 = 1;

/// Allocation guard for a single compressed block-entity payload: the
/// tunnel's default 2 MiB packet cap is the upstream ceiling.
pub const MAX_COMPRESSED_ENTITY_BYTES: usize = 2_097_152;

/// Reassembly state for one player (upstream `AxiomPlayerState` fields).
#[derive(Debug, Default, Clone)]
pub struct TunnelBuffer {
    /// Fragments of the packet in flight, flags bytes included.
    fragments: Vec<Vec<u8>>,
    /// Total bytes buffered, flags bytes included.
    total_bytes: usize,
    /// Whether the packet in flight is discarded until the next FIRST.
    skip: bool,
}

/// What the tunnel decided to do with a received fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelAction {
    /// A reassembled internal packet, ready for the channel dispatcher.
    Dispatch(Vec<u8>),
    /// Fragment buffered or silently dropped (orphan/skip path).
    Nothing,
    /// Protocol violation: disconnect the player with this reason.
    Violation(&'static str),
}

/// Maximum size the internal-packet decoder accepts for the `ZSTD` output
/// before delegating to the configuration cap.
///
/// Guards negative `Int`s that upstream would turn into failed allocations.
fn sane_uncompressed(value: i32) -> Option<usize> {
    usize::try_from(value).ok()
}

/// Feeds one `axiom:tunnel` fragment. Mirrors `handleTunnelBuffer`.
///
/// An orphan continuation is dropped silently, oversized inputs kick, and
/// the reassembly buffer is bounded by both total bytes and fragment count.
pub fn process_fragment(
    config: &AxiomConfig,
    tunnel: &mut TunnelBuffer,
    bytes: &[u8],
) -> TunnelAction {
    let split = usize::try_from(config.tunnel_split_size.max(1)).unwrap_or(usize::MAX);
    if bytes.len() > split {
        return TunnelAction::Violation("Axiom: Sent split buffer that was too large");
    }

    let flags = bytes.first().copied().unwrap_or(0);
    let first = flags & FLAG_FIRST != 0;
    let last = flags & FLAG_LAST != 0;

    if first && last {
        return TunnelAction::Dispatch(bytes[1..].to_vec());
    }

    if first {
        tunnel.skip = false;
        tunnel.fragments.clear();
        tunnel.total_bytes = 0;
    } else if tunnel.skip {
        return TunnelAction::Nothing;
    }

    if !first && tunnel.fragments.is_empty() {
        // A continuation without a FIRST in flight: discard until the next
        // FIRST exactly like upstream.
        tunnel.skip = true;
        return TunnelAction::Nothing;
    }

    tunnel.fragments.push(bytes.to_vec());
    tunnel.total_bytes += bytes.len();

    // Upstream bound: maxTotal = maximum + split, and one fragment more
    // than the whole packet would need.
    let maximum = config.maximum_tunnel_packet_size.max(0) as usize;
    let max_total = maximum + split;
    let max_fragments = max_total / split + 1;
    if tunnel.total_bytes > max_total || tunnel.fragments.len() > max_fragments {
        tunnel.fragments.clear();
        tunnel.total_bytes = 0;
        tunnel.skip = true;
        return TunnelAction::Violation("Axiom: Sent too many split buffers");
    }

    if last {
        let total: usize = tunnel
            .fragments
            .iter()
            .map(|fragment| fragment.len() - 1)
            .sum();
        let mut combined = Vec::with_capacity(total);
        for fragment in &tunnel.fragments {
            combined.extend_from_slice(&fragment[1..]);
        }
        tunnel.fragments.clear();
        tunnel.total_bytes = 0;
        TunnelAction::Dispatch(combined)
    } else {
        TunnelAction::Nothing
    }
}

/// Decodes a reassembled internal packet into `(channel, payload)`.
///
/// Returns the destination channel and the (decompressed) payload on
/// success; errors carry the exact upstream kick reason. The caller enforces
/// the fully-read invariant on the payload after the channel handler runs.
pub fn decode_packet(config: &AxiomConfig, bytes: &[u8]) -> Result<(String, Vec<u8>), String> {
    let mut reader = Reader::new(bytes);
    let channel = reader
        .read_string()
        .map_err(|_| "Axiom: Failed to read tunnel packet channel".to_string())?;
    let uncompressed_size = reader
        .read_i32_be()
        .map_err(|_| "Axiom: Failed to read tunnel packet header".to_string())?;
    let packet_flags = reader
        .read_byte()
        .map_err(|_| "Axiom: Failed to read tunnel packet header".to_string())?;

    let maximum = config.maximum_tunnel_packet_size;
    let Some(capacity) = sane_uncompressed(uncompressed_size) else {
        return Err("Axiom: Sent packet was too large".to_string());
    };
    if uncompressed_size > maximum {
        return Err("Axiom: Sent packet was too large".to_string());
    }

    let rest = &bytes[reader.position()..];
    if packet_flags & PACKET_FLAG_ZSTD != 0 {
        let decompressed = zstd::bulk::decompress(rest, capacity)
            .map_err(|err| format!("Axiom: Zstd failed to decompress: {err}"))?;
        if decompressed.len() != capacity {
            return Err("Axiom: Uncompressed size didn't match real size".to_string());
        }
        Ok((channel, decompressed))
    } else {
        Ok((channel, rest.to_vec()))
    }
}

/// Whether the destination of a tunneled packet is a channel this phase
/// accepts serverbound.
///
/// `axiom:tunnel` itself is advertised but nested tunnels are rejected
/// (upstream registers the handler slot as null).
#[must_use]
pub fn dispatchable(channel: &str) -> bool {
    matches!(
        channel,
        channels::HELLO
            | channels::SET_BLOCK
            | channels::SET_BUFFER
            | channels::TICK_BLOCKS
    ) || channel == channels::TUNNEL
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AxiomConfig {
        AxiomConfig::default()
    }

    /// Builds an internal packet exactly like the Axiom client does.
    fn internal_packet(channel: &str, payload: &[u8], zstd_flag: bool, declared: Option<i32>) -> Vec<u8> {
        let mut writer = crate::wire::Writer::new();
        writer.write_string(channel);
        let mut bytes = writer.into_bytes();
        let size = declared.unwrap_or(payload.len() as i32);
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.push(u8::from(zstd_flag));
        bytes.extend_from_slice(payload);
        bytes
    }

    fn fragment(flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![flags];
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn single_fragment_packet_decodes() {
        let packet = internal_packet("axiom:set_block", &[1, 2, 3], false, None);
        let mut tunnel = TunnelBuffer::default();
        let action = process_fragment(&config(), &mut tunnel, &fragment(FLAG_FIRST | FLAG_LAST, &packet));
        let TunnelAction::Dispatch(bytes) = action else {
            panic!("expected dispatch, got {action:?}");
        };
        let (channel, payload) = decode_packet(&config(), &bytes).unwrap();
        assert_eq!(channel, "axiom:set_block");
        assert_eq!(payload, vec![1, 2, 3]);
    }

    #[test]
    fn multi_fragment_reassembly_strips_flags() {
        let packet = internal_packet("axiom:set_buffer", &[7; 50], false, None);
        let mid = packet.len() / 2;
        let mut tunnel = TunnelBuffer::default();
        assert_eq!(
            process_fragment(&config(), &mut tunnel, &fragment(FLAG_FIRST, &packet[..mid])),
            TunnelAction::Nothing
        );
        let action = process_fragment(&config(), &mut tunnel, &fragment(FLAG_LAST, &packet[mid..]));
        let TunnelAction::Dispatch(bytes) = action else {
            panic!("expected dispatch, got {action:?}");
        };
        assert_eq!(bytes, packet);
    }

    #[test]
    fn orphan_fragment_is_silently_skipped() {
        let mut tunnel = TunnelBuffer::default();
        assert_eq!(
            process_fragment(&config(), &mut tunnel, &fragment(0, &[1, 2])),
            TunnelAction::Nothing
        );
        // Now skipping: even a LAST fragment is dropped.
        assert_eq!(
            process_fragment(&config(), &mut tunnel, &fragment(FLAG_LAST, &[3])),
            TunnelAction::Nothing
        );
        // A fresh FIRST recovers.
        let packet = internal_packet("axiom:set_block", &[9], false, None);
        let action =
            process_fragment(&config(), &mut tunnel, &fragment(FLAG_FIRST | FLAG_LAST, &packet));
        assert!(matches!(action, TunnelAction::Dispatch(_)));
    }

    #[test]
    fn oversized_fragment_kicks() {
        let mut tunnel = TunnelBuffer::default();
        let huge = vec![FLAG_FIRST | FLAG_LAST; 31_001];
        assert_eq!(
            process_fragment(&config(), &mut tunnel, &huge),
            TunnelAction::Violation("Axiom: Sent split buffer that was too large")
        );
    }

    #[test]
    fn too_many_fragments_kicks() {
        // Defaults: max_total = 2MiB + 31000 = 2128152, max_fragments = 69.
        let mut tunnel = TunnelBuffer::default();
        let mut kicked = false;
        let payload = vec![0u8; 30_999];
        for index in 0..71 {
            let flags = if index == 0 { FLAG_FIRST } else { 0 };
            if let TunnelAction::Violation(reason) =
                process_fragment(&config(), &mut tunnel, &fragment(flags, &payload))
            {
                assert_eq!(reason, "Axiom: Sent too many split buffers");
                kicked = true;
                break;
            }
        }
        assert!(kicked, "expected the fragment-count guard to fire");
    }

    #[test]
    fn zstd_roundtrip_and_size_mismatch() {
        let payload = vec![42u8; 1000];
        let compressed = zstd::bulk::compress(&payload, 3).unwrap();
        let packet = internal_packet("axiom:set_buffer", &compressed, true, Some(1000));
        let (channel, decompressed) = decode_packet(&config(), &packet).unwrap();
        assert_eq!(channel, "axiom:set_buffer");
        assert_eq!(decompressed, payload);

        // Declared size larger than the real content: decompression
        // succeeds but the size check fails.
        let packet = internal_packet("axiom:set_buffer", &compressed, true, Some(1001));
        assert_eq!(
            decode_packet(&config(), &packet).unwrap_err(),
            "Axiom: Uncompressed size didn't match real size"
        );
        // Declared size smaller than the real content fails inside zstd.
        let packet = internal_packet("axiom:set_buffer", &compressed, true, Some(999));
        let err = decode_packet(&config(), &packet).unwrap_err();
        assert!(err.starts_with("Axiom: Zstd failed to decompress"), "{err}");
    }

    #[test]
    fn declared_size_above_cap_kicks() {
        let packet = internal_packet("axiom:set_buffer", &[0; 10], false, Some(2_097_153));
        assert_eq!(
            decode_packet(&config(), &packet).unwrap_err(),
            "Axiom: Sent packet was too large"
        );
        // Negative sizes are violations too (upstream would fail allocating).
        let packet = internal_packet("axiom:set_buffer", &[0; 10], false, Some(-1));
        assert_eq!(
            decode_packet(&config(), &packet).unwrap_err(),
            "Axiom: Sent packet was too large"
        );
    }

    #[test]
    fn garbage_never_panics_the_decoder() {
        // Deterministic PRNG: pseudo-random buffers must never panic the
        // reassembler or the internal-packet decoder (only kick or drop).
        let mut seed: u64 = 0x1234_5678_9ABC_DEF0;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let config = config();
        for _ in 0..512 {
            let len = (next() % 4096) as usize + 1;
            let bytes: Vec<u8> = (0..len).map(|_| (next() & 0xFF) as u8).collect();
            let mut tunnel = TunnelBuffer::default();
            let action = process_fragment(&config, &mut tunnel, &bytes);
            if let TunnelAction::Dispatch(packet) = action {
                let _ = decode_packet(&config, &packet);
            }
        }
    }

    #[test]
    fn lost_first_then_recovery() {
        // FIRST lost: the next continuation skips; a new FIRST + LAST pair
        // still delivers exactly its own payload.
        let mut tunnel = TunnelBuffer::default();
        assert_eq!(
            process_fragment(&config(), &mut tunnel, &fragment(0, &[9; 64])),
            TunnelAction::Nothing
        );
        let packet = internal_packet("axiom:tick_blocks", &[7; 32], false, None);
        let action = process_fragment(
            &config(),
            &mut tunnel,
            &fragment(FLAG_FIRST | FLAG_LAST, &packet),
        );
        let TunnelAction::Dispatch(bytes) = action else {
            panic!("expected dispatch, got {action:?}");
        };
        assert_eq!(bytes, packet);
    }

    #[test]
    fn dispatch_table_matches_phase() {
        assert!(dispatchable("axiom:set_block"));
        assert!(dispatchable("axiom:hello"));
        assert!(dispatchable("axiom:tunnel"));
        assert!(!dispatchable("axiom:set_gamemode"));
        assert!(!dispatchable("axiom:teleport"));
    }
}
