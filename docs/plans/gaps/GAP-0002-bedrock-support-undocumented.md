# GAP-0002: Bedrock Edition support is work-in-progress — behaviour parity undocumented

- **Status:** Open
- **Found by:** ddd-setup
- **Date:** 2026-11-21

## Description

The README marks Bedrock Edition support as "W.I.P", and the Bedrock protocol implementation lives in `crates/pumpkin-protocol/src/bedrock/` alongside the Java one. Unlike the Java side (which pins `CURRENT_MC_VERSION` in generated `pumpkin-data` and has a vanilla-parity discipline documented in AGENTS.md), this pass found no documented statement of which Bedrock protocol versions are supported, what subset of gameplay works over Bedrock, or what parity target applies to it.

## Impact

Any work touching Bedrock paths has no documented baseline to verify against; docs claiming "vanilla parity" cannot be extended to Bedrock behaviour, and the Bedrock listener's default-enabled state (port 19132) may surprise operators of Java-only servers.

## Proposed action

When Bedrock support stabilizes enough to define a target, document in `docs/architecture/flows.md` (connection flow) which Bedrock protocol version(s) are supported and what the parity claim is. Until then, any Bedrock-behaviour claim in these docs should be treated as unverified.
