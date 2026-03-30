# Venturi Core Runtime Architecture

This document explains how Venturi's runtime is structured so contributors can reason about behavior changes quickly.

## Core Idea

Venturi is a message-driven system with two threads:

1. **Core thread** — PipeWire/system integration and side effects.
2. **GUI thread** — GTK view-model and widgets.

Communication uses typed channels:

- GUI → Core: `CoreCommand`
- Core → GUI: `CoreEvent`

## Startup Sequence

1. `main.rs` creates `AppRunner` and chooses daemon vs GUI mode.
2. `AppRunner` creates command/event channels and spawns `PipeWireManager`.
3. `PipeWireManager::spawn` sends `CoreEvent::Ready` and initializes `CoreRuntimeState`.
4. `CoreRuntimeState::initialize` performs:
   - XDG path resolution and config load
   - hotkey backend selection/registration
   - virtual device provisioning (`Venturi-Output`, `Venturi-VirtualMic`)
   - initial virtual-mic rewiring to selected/default physical input
5. Non-daemon mode starts GUI and optional tray.

## Module Boundaries

### `src/core/pipewire_manager.rs`

Owns runtime orchestration. It should contain **loop control and state transitions**, not low-level process helpers.

Key items:

- `CoreRuntimeState` — mutable state for one manager thread.
- `handle_core_command` — command dispatch.
- `handle_hotkey_tick` — maps hotkey events to commands.
- `refresh_snapshot` — poll + diff + emit events.

### `src/core/pipewire_discovery.rs`

Pure-ish discovery/parsing layer.

- `Snapshot`
- `StreamInfo`
- `parse_pw_dump(...)`
- `poll_snapshot(...)`

Responsibilities:

- Parse `pw-dump` JSON into normalized state.
- Hide internal virtual nodes from user selectors when requested.
- Normalize stream display naming (e.g., prefer `application.process.binary` when app name is generic WebRTC text).

### `src/core/pipewire_backend.rs`

Side-effect backend for process invocations.

- `run_wpctl`, `run_pactl`, `run_pw_metadata`, `run_pw_link`
- virtual module management (`ensure_virtual_devices`, `rewire_virtual_mic_source`, loopback load/unload)

Responsibilities:

- Execute external commands.
- Return explicit `Result` errors so manager can emit `CoreEvent::Error`.

### `src/core/pipewire_channel_control.rs`

Channel-level volume/mute logic.

- `apply_channel_volume(...)`
- `apply_channel_mute(...)`
- `ChannelControlTargets`

Responsibilities:

- Map semantic channels (`Main`, `Mic`, `Game`, `Media`, `Chat`, `Aux`) to concrete control targets.
- Apply scoping rules consistently.

## CoreRuntimeState Data Structure

`CoreRuntimeState` tracks thread-local mutable state:

- Routing mode (`metadata-first` / fallback links)
- Loaded config and categorizer overrides
- Selected input device and active module IDs
- Last snapshot for diff-based event emission
- Per-target dedupe maps for volume/mute writes
- Hotkey binding state and backend adapter

Rule: no state is shared across threads; only commands/events cross thread boundaries.

## Event Loop Flow

Each tick in `PipeWireManager`:

1. `recv_timeout(POLL_INTERVAL)` for `CoreCommand`
2. `handle_core_command(...)`
3. `handle_hotkey_tick(...)`
4. `refresh_snapshot(...)`

This preserves a stable cadence where user actions and periodic discovery both make progress.

## Device and Stream Behavior

- OS-visible output target in Venturi is `Venturi-Output` (internal sink).
- Physical output selector controls a loopback from `Venturi-Output.monitor` to selected device.
- Virtual input is `Venturi-VirtualMic`; selected physical input is rewired into this source.
- Internal monitor sources and virtual internal nodes are filtered out of GUI selectors.

## Failure Handling Pattern

Manager methods return `Result<_, String>` for side-effect failures.

On error, the manager emits `CoreEvent::Error(...)` and continues loop operation when possible.

## Contributor Guidelines

When adding behavior:

1. Add/update command/event contract first (`messages.rs`).
2. Put parsing/discovery changes in `pipewire_discovery.rs`.
3. Put system command logic in `pipewire_backend.rs`.
4. Keep orchestration/state transitions in `pipewire_manager.rs`.
5. Add targeted tests closest to the changed module.

This keeps manager readable and prevents new side-effect code from creeping into orchestration paths.
