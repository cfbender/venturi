# Venturi Core Runtime Architecture

This document captures the current runtime architecture and hard-earned lessons from recent PipeWire monitor + metering work.

## Runtime Model

Venturi is a message-driven system with these active runtimes:

1. **Core manager thread** (`PipeWireManager`) — owns orchestration and side effects.
2. **GUI thread** (GTK main loop) — owns `MixerTab` model + widgets.
3. **PwMonitor reader thread** (`pw-dump --monitor`) — feeds structural/volume deltas to core.
4. **Meter worker thread** — samples `pw-record` targets and emits `LevelsUpdate`.

Channel contracts:

- GUI → Core: `CoreCommand`
- Core → GUI: `CoreEvent`

## Startup Handshake (Important)

1. `AppRunner` creates channels and spawns `PipeWireManager`.
2. Core emits `CoreEvent::Ready`.
3. App sends `CoreCommand::Ping`; core responds with `CoreEvent::Pong`.
4. GUI launches and starts receiving events.
5. After `window.present()`, GUI sends:
   - `CoreCommand::SetMeteringEnabled(true)`
   - `CoreCommand::RequestSnapshot`

### Why `RequestSnapshot` exists

`wait_for_event` in `app.rs` only waits for specific events (`Ready` / `Pong`) and drops unrelated ones during bootstrap. That means early `DevicesChanged`/`StreamAppeared` events can be lost before GUI listeners are live.

`RequestSnapshot` triggers `resend_initial_state()` in core to re-emit current devices/streams once the GUI is ready.

## Core Loop

`PipeWireManager::spawn` runs a `crossbeam_channel::select!` loop across:

- `command_rx` (GUI commands)
- `monitor_rx` (`PwMonitorEvent`s)
- timeout tick (`LOOP_TICK_INTERVAL`)

Loop responsibilities:

- Coalesce `SetVolume` bursts (`coalesce_commands`)
- Handle command side effects (`handle_core_command`)
- Apply monitor updates (`handle_monitor_event`)
- Handle monitor restart backoff/circuit breaker
- Flush persisted state and tick hotkeys

## Snapshot Ownership & Structural Deltas

`CoreRuntimeState.last_snapshot` is the authoritative map for:

- device selectors (`devices`)
- control routing maps (`output_ids`, `input_ids`)
- meter target maps (`output_meter_targets`, `input_meter_targets`)
- app streams (`streams`)
- per-node volumes (`volumes`)

`merge_changed_objects` + `apply_structural_monitor_delta` must keep this coherent under PipeWire ID churn.

### Guardrail

When IDs are reassigned/reused, prune stale mappings/streams/volumes first, then upsert new structures. Otherwise you can issue commands to dead node IDs and see errors like:

- `Object '<id>' not found`

## Metering Architecture

### Target semantics that matter

`pw-record --target` is reliable with **object serial or node name**. Numeric node ID behavior can be inconsistent in practice across object classes.

Current strategy:

- **Main/Mic meters:** targets from `output_meter_targets` / `input_meter_targets`, with sink/source values parsed as `serial.or(id)`.
- **Per-app meters (Game/Media/Chat/Aux):** stream `node_name` targets.
- **Corked streams:** ignored for per-app stream-name sampling (`pulse.corked == true`).
- Numeric stream sampling is not used for per-app meters to avoid cross-channel coupling and dead samples.

Meter worker internals:

- refreshes snapshot from shared cache (`Arc<Mutex<Snapshot>>`)
- builds channel targets from snapshot + categorizer overrides
- maintains sampler caches per target
- emits `CoreEvent::LevelsUpdate`

## Shared Snapshot Boundary

Core and meter worker share `Arc<Mutex<Snapshot>>` only for read-copy metering use. Core writes; meter worker clones.

This is the only intentional shared state between runtime threads.

## Staged Typed Service Seams

Tray and hotkey lifecycle paths now cross adapter boundaries through typed enums instead of string command IDs.

- `venturi-platform-adapter` tray actions map to typed tray commands (`ToggleWindow`, `Shutdown`).
- Core tray callsites convert typed tray commands directly into `CoreCommand` values.
- Hotkey adapter actions are typed press/release variants, so event conversion no longer depends on string parsing fallbacks.

This keeps boundary migration incremental while preserving existing runtime behavior.

## Lifecycle Parity Tests

`venturi-runtime::composition::TestSnapshot` includes lifecycle request flags for toggle-window and shutdown paths, with read access in `SnapshotView`.

Integration coverage now verifies parity across tray + hotkey lifecycle dispatch:

- tray `Show/Hide` and `Quit` actions emit expected `CoreCommand`s
- hotkey toggle emits `CoreCommand::ToggleWindow`
- applying these commands into runtime composition lifecycle methods sets parity snapshot flags

Primary verification entry point: `cargo test --test parity_tray_hotkeys`.

## Routing & Channel Control Boundaries

### `Main`

- Control target: `Venturi-Output` sink
- Physical device selection is separate and handled by loopback reconciliation from `Venturi-Output.monitor` to selected output

### `Mic`

- Control target: `Venturi-VirtualMic` source
- Selected physical input is rewired into this virtual mic source

### App channels (`Game`, `Media`, `Chat`, `Aux`)

- Control target: Venturi channel mix sinks (`Venturi-Game`, `Venturi-Media`, `Venturi-Chat`, `Venturi-Aux`)
- Category slider semantics: controls each channel bus contribution into `Venturi-Output` (app-local source volume remains app/OS controlled)
- Classification source: `classify_with_priority(overrides, binary, app_name, media_role)`

## Failure Handling

Monitor process failures are handled with backoff and a circuit breaker:

- restart delay
- consecutive failure tracking in a time window
- give-up path emits `CoreEvent::Error`
- successful `InitialSnapshot` resets failure counters

## Operational Gotchas (from real regressions)

1. **Do not re-enqueue events in `wait_for_event`.**
   Re-sending non-matching events to the same channel can create an event feedback loop and starve command processing.

2. **Keep bootstrap replay explicit.**
   Use `RequestSnapshot` after GUI activation instead of clever channel replay hacks.

3. **Treat PipeWire IDs as volatile.**
   Structural monitor delta merge must aggressively prune stale references.

4. **Meter target type is part of correctness.**
   Serial/name vs id selection can make the difference between real levels and silence/cross-coupling.

5. **Stale GUI process can fake regressions.**
   GTK `GApplication` uniqueness means an old instance can receive activation while new `cargo run` exits immediately.
   If behavior looks impossible, verify no stale Venturi process is running.

## Contributor Checklist for Runtime Changes

When changing runtime behavior:

1. Update command/event contract first (`messages.rs`).
2. Keep parsing/discovery in `pipewire_discovery.rs`.
3. Keep command/process side effects in `pipewire_backend.rs`.
4. Keep loop/state transitions in `pipewire_manager.rs`.
5. Add regression tests near touched logic (startup replay, structural delta pruning, target precedence).
6. Verify with:
   - `cargo check`
   - `cargo test --lib`
   - `cargo clippy -- -D warnings`
