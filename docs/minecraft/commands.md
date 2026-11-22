# Commands — carrot module command surface

> **Status:** Implemented — verified against the `register_command` call sites in each crate's source, commits `27da4e5af`–`a4fcb98b7` (2026-11-22). carrot has no `plugin.yml`-style manifest: commands are registered natively through each module's synthetic plugin context, so this document plus the registering code is the source of truth.

Aliases below are separate full registrations (one dispatcher entry per literal), not Brigadier `.alias` links. Permissions use the `carrot:` namespace with LuckPerms-style groups from `carrot/groups.toml` (see [flows §8](../architecture/flows.md#8-carrot-module-lifecycle-this-fork) and [ADR-0006](../plans/decisions/ADR-0006-carrot-module-system.md)).

## Root infrastructure (always registered — `carrot-core`)

Permission: `carrot:admin` (default Op level 4).

| Command | Purpose |
| --- | --- |
| `/carrot module list` | List modules and their enabled state. |
| `/carrot module enable <id>` | Enable a module at runtime (persists to `carrot.toml`). |
| `/carrot module disable <id>` | Disable a module at runtime; removes its commands and handlers, like plugin unload. |
| `/carrot groups reload` | Reload `groups.toml` and re-apply groups to all online players. |

> **Note:** [ADR-0006](../plans/decisions/ADR-0006-carrot-module-system.md) names `/carrotgroups reload`; the code registers it as the `/carrot groups` subcommand. The discrepancy is tracked in [GAP-0003](../plans/gaps/GAP-0003-adr0006-carrotgroups-command-name.md); this document follows the code.

## Module ids

`builder_mode` (in the `carrot` aggregator; no commands), `essentials`, `multiverse`, `chat`, `worldedit`, `terrain`.

## `essentials` (crates/carrot-essentials)

| Command | Purpose | Permission default |
| --- | --- | --- |
| `/fly` | Toggle flight for yourself. | Allow |
| `/speed <value> [walk\|fly]` | Set walk or fly speed (EssentialsX scale, 1–8). | Allow |
| `/gm <mode> [player]` | Change gamemode; mode accepts `c/s/a/sp` or `0–3`. | Op 2 (`carrot:essentials.gamemode`) |
| `/tpa <target>` | Ask a player to let you teleport to them. | Allow |
| `/tpaccept [sender]` | Accept a pending teleport request. | Allow |
| `/tpdeny [sender]` | Deny a pending teleport request. | Allow |
| `/home [name]` | Teleport to one of your homes (default `home`). | Allow |
| `/sethome [name]` | Set one of your homes (`homes.toml`). | Allow |
| `/delhome [name]` | Delete one of your homes. | Allow |
| `/warp [name]` | Teleport to a warp, or list all warps. | Allow |
| `/setwarp <name>` | Set a warp at your position (`warps.toml`). | Op 2 |
| `/delwarp <name>` | Delete a warp. | Op 2 |
| `/back` | Return to your location before the last teleport or death. | Allow |
| `/spawn` | Teleport to the global spawn (configured world, default `byv`). | Allow |
| `/n`, `/nv` | Toggle night vision (Builders-Utilities behavior). | Allow |

Teleport/world-change/death event handlers record `/back` positions (not commands).

## `chat` (crates/carrot-chat)

Command names come from `chat.toml` (`[local].command`, `[global].command`; empty string skips registration). Defaults:

| Command | Purpose | Permission default |
| --- | --- | --- |
| `/l [message]` | Focus the local channel (no args) or talk in it; 50-block radius, filters and cooldowns apply. | Allow (`carrot:chat.channel.local`) |
| `/g [message]` | Same for the global channel. | Allow (`carrot:chat.channel.global`) |

While enabled, the module cancels vanilla chat broadcasts and re-routes every message through the focused channel.

## `multiverse` (crates/carrot-multiverse)

| Command | Purpose | Permission default |
| --- | --- | --- |
| `/mv [list]` | World management: `list` (or bare), `info <world>`, `load <world>`, `unload <world>`. | Op 2 (`carrot:multiverse.admin`) |
| `/goto <world>` | Teleport to a world, loading it on demand. | Allow (`carrot:multiverse.goto`) |

## `worldedit` (crates/carrot-worldedit)

All permissions default Op 2, namespaced under `carrot:worldedit.*` (group listed in the Permission column).

Selection (`carrot:worldedit.selection`):

| Command | Aliases | Purpose |
| --- | --- | --- |
| `//wand` | — | Gives the selection wand (config item, default `wooden_axe`). Left/right click sets pos1/pos2. |
| `//pos1`, `//pos2` | `//1`, `//2` | Set a selection corner (optional position argument). |
| `//sel` | — | Show the current selection (size/volume). |
| `//expand [amount] [direction]` | `//exp` | Grow the selection. |
| `//contract [amount] [direction]` | `//co` | Shrink the selection. |
| `//shift <amount>` | — | Move the selection (direction follows look). |
| `//vert` | — | Expand the selection to the world's vertical bounds. |
| `//farwand` | `//fw` | Toggle long-range (ray) wand selection. |

Region (`carrot:worldedit.region`):

| Command | Aliases | Purpose |
| --- | --- | --- |
| `//set <pattern>` | `//s` | Fill the selection with a pattern (Builders-Utilities shorthand). |
| `//replace [from] <to>` | `//rep`, `//r`, `//re` | Replace blocks in the selection. |
| `//walls <pattern>` | — | Build the region walls. |
| `//outline <pattern>` | — | Build the region shell. |

Clipboard (`carrot:worldedit.clipboard`):

| Command | Aliases | Purpose |
| --- | --- | --- |
| `//copy` | `//c` | Copy the selection. |
| `//cut` | — | Copy and clear the selection. |
| `//paste [flags]` | `//p`, `//pa` | Paste the clipboard. |
| `//rotate [angle]` | `//rt` | Rotate the clipboard. |
| `//flip [direction]` | `//f` | Mirror the clipboard. |
| `//stack [count] [direction]` | `//st` | Repeat the selection along a direction. |
| `//move [count] [direction]` | `//m` | Move the selection contents. |
| `//fp`, `//fp-a` | — | DoTrO macros: flip + paste (skip air). |
| `//rt90x4`, `//rt90x4-a` | — | DoTrO macros: rotate-paste ×4 (skip air). |
| `//crt90x4`, `//crt90x4-a` | — | DoTrO macros: copy + rotate-paste ×4 (skip air). |

History (`carrot:worldedit.history`): `//undo [times]`, `//redo [times]`, alias `//u` (= undo) — on-disk gzipped-NBT history under `carrot/modules/worldedit/history/`.

Schematics (`carrot:worldedit.schematic`): `//schem` / `//schematic` with `save <name>`, `load <name>`, `list`, `delete <name>` — Sponge `.schem` (reads v1/2/3, writes v2) on the shared `carrot/schematics/` folder.

Mask & confirm: `//gmask [mask]` (`carrot:worldedit.mask`) sets/clears the session-wide mask; `//confirm` (`carrot:worldedit.confirm`) applies a pending large edit.

Generation (`carrot:worldedit.generation`): `//cyl <pattern> <radius[,radiusZ]> [height] [-h]`, `//hcyl`, `//sphere <pattern> <radius[,radiusY[,radiusZ]]> [-h]`, `//hsphere` — FAWE-geometry solid/hollow shapes.

Utility (shares `carrot:worldedit.region`): `//fill` (downward flood of connected air), `//center` (mark center), `//line`, `//curve` (draw between selection corners).

Navigation (`carrot:worldedit.navigation`): `/up` and `//up` — go upwards, placing glass below (`-f` skip, `-g` force).

## `terrain` (crates/carrot-terrain)

Rides the worldedit engine: gmask, `//confirm` and undo history apply to terrain edits. Permissions default Op 2; brush tool permissions: `carrot:terrain.brush`, `carrot:terrain.sniper`, `carrot:terrain.gobrush`.

Brushes (`commands/br.rs`):

| Command | Purpose |
| --- | --- |
| `//br <type> [args]` | Bind a terrain brush to the held item: `none`, `sphere`, `cylinder`, `smooth`, `erode`, `blendball`, `rock`/`blob`, `boulder`, `shatter`; flags `-h/-f/-a`. |
| `//ab cube\|boulder [args]` | Arceon cube / boulder brushes. |
| `//terragen <type> [args]` | Noise terrain over the selection (simplex, fractal, billowy, ridged, turbulence, electric, euclidean, manhattan, natural, poly). Alias: `//tgen`. |

VoxelSniper tools (`commands/sniper_cmd.rs`):

| Command | Purpose |
| --- | --- |
| `/b [args]` | Sniper tool (arrow/gunpowder actions). Bare number sets size; brushes `ball`/`b`, `cylinder`/`c`, `disc`/`d`, `erode`/`e`, `erodeblend`/`eb`, `blendball`/`bb`, `blenddisc`/`bd`. |
| `/v [pattern]` | Sniper brush pattern (no args = targeted block). |
| `/vr [pattern]` | Sniper replace-restriction pattern. |

goBrush (`commands/gobrush_cmd.rs`): `/gobrush`, alias `/gb` — heightmap painting tool (flint item). Subcommands: `size|s N`, `intensity|i N`, `brush|b <name>`, `toggle|t`, `up`, `down`, `flat`, `3d`, `rotate`, `list`. Left-click toggles, right-click paints. Default brushes are 18 compiled-in PNG heightmaps seeded to `carrot/gobrush/brushes/` on first enable; runtime brushes are read from disk, so users can add their own PNGs.

Builder shortcuts (`commands/aliases.rs`, from the reference server's `worldedit_abreviacoes.sk`):

| Command | Purpose | Permission |
| --- | --- | --- |
| `/brb <pattern> <size> <smoothness> <height>` | Boulder brush preset. | `carrot:terrain.brush` |
| `/brblob <pattern> <radius> <roundness> <frequency> <amplitude>` | Blob brush preset. | `carrot:terrain.brush` |
| `/brshatter <radius> <count>` | Shatter brush preset. | `carrot:terrain.brush` |
| `/abcube <pattern> [size] [snap] [height] [width]` | Arceon cube preset. | `carrot:terrain.brush` |
| `/abcubeh` | Arceon cube with `-h` forced. | `carrot:terrain.brush` |
| `/abb` | Boulder + width. | `carrot:terrain.brush` |
| `/abba` | Boulder with `-a` forced. | `carrot:terrain.brush` |
| `/abac` | Cube with `-a -c`. | `carrot:terrain.brush` |
| `/bang` | Cliff heightmap preset (enabled). | `carrot:terrain.gobrush` |
| `/bmelt` | Erode melt. | `carrot:terrain.sniper` |
| `/bbb` | Blend ball. | `carrot:terrain.sniper` |
| `/bbd` | Blend disc. | `carrot:terrain.sniper` |
| `/befill` | Erode-blend fill. | `carrot:terrain.sniper` |
| `/brandom` | Ball with random radius 1–100. | `carrot:terrain.sniper` |

Arceon/ezEdits suite:

| Command | Aliases | Purpose |
| --- | --- | --- |
| `//tr [from] [to] [-f\|-b\|-s]` | `//typer`, `//typereplace` | Replace block-type families (stairs/slabs/walls/fences/…, 56 families × 17 roles) preserving state properties; bare `//tr` lists families. |
| `//revolve <amount> [start] [end] [heightDiff] [-p] [-s] [-r]` | `//rev` | Revolve the selection around the player; `-p` polygon, `-s` spoke line. |
| `//loft [args]` | — | Multi-frame bezier lofts: `set|s`, `frame|f`, `point|pos|p`, `clear|c`, `remove|r`; paper item marks frames/points. |
| `//eztexture <mode> <mask> <palette> [args]` | `//ezt` | Palette texturing (ambient, sunlight, pointlight, curvature, axisgradient, shift). |
| `//ezpalette <sub> [args]` | `//ezp` | Palette management: `fetch <name> [count] [-d dir]`, `save <name> <blocks...>`, `list`, `delete\|remove <name>` (stored as `carrot/ezpalettes/<name>.txt`). |

Terrain mask tokens (registered into the worldedit mask parser — pattern tokens, not commands): `#arcangle[min][max][size][-o]`, `#vor[zoom][jitter]`, `#noise`, `#fractal`, `#turbulence`, `#crack`, `#electric`.

## `builder_mode` (in crates/carrot)

No commands. Registers the `carrot:buildermode` permission (default Deny) and blocking join/respawn handlers: holders join/respawn in Creative, everyone else in Survival (parity with the reference server's `criativo.sk`).
