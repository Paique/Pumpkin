//! Axiom plugin-messaging channel names (mirror of upstream
//! `AxiomChannels.java`, Axiom API version 10).

/// Handshake: server sends a random non-zero id, client echoes it with its
/// version numbers. Always a dedicated channel, never tunneled.
pub const HELLO: &str = "axiom:hello";
/// Activates (`true`) or deactivates (`false`) the client editor.
pub const ENABLE: &str = "axiom:enable";
/// Textual rejection sent on any handshake failure.
pub const GOODBYE: &str = "axiom:goodbye";
/// Capability restrictions derived from the player's permission set.
pub const RESTRICTIONS: &str = "axiom:restrictions";
/// World-property registry announcement (empty registry sends a single
/// `VarInt` `0`).
pub const REGISTER_WORLD_PROPERTIES: &str = "axiom:register_world_properties";
/// Custom block registry clear (single `VarInt` `0` when the server has none).
pub const CUSTOM_BLOCKS: &str = "axiom:custom_blocks";
/// The tunnel multiplexer: fragmented payloads under the 32 KiB
/// custom-payload cap, reassembled into internal packets (phase AX-1).
pub const TUNNEL: &str = "axiom:tunnel";
/// Unitary/small block edits from the client editor (phase AX-1).
pub const SET_BLOCK: &str = "axiom:set_block";
/// Bulk section buffers (runtime palettes) from the client editor
/// (phase AX-1).
pub const SET_BUFFER: &str = "axiom:set_buffer";
/// Forced ticking of blocks/fluids in a region (phase AX-1).
pub const TICK_BLOCKS: &str = "axiom:tick_blocks";
/// Dispatch-budget recovery notification (`VarInt add`, `VarInt max`).
pub const UPDATE_AVAILABLE_DISPATCH_SENDS: &str = "axiom:update_available_dispatch_sends";
