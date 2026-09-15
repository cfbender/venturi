# Venturi Core Runtime Architecture

## Runtime Model

Venturi is a message-driven system with these active runtimes:

1. **Core manager thread** (`PipeWireManager`) — owns orchestration and side effects.
2. **GUI thread** (GTK main loop) — owns `MixerTab` model + widgets.
3. **PwMonitor reader thread** (`pw-dump --monitor`) — feeds structural/volume deltas to core.
4. **Meter worker thread** — samples the 6 fixed Venturi bus nodes and emits `LevelsUpdate`.

Channel contracts:

- GUI → Core: `CoreCommand`
- Core → GUI: `CoreEvent`

## Startup Sequence

1. `AppRunner` creates channels and spawns `PipeWireManager`.
2. Core emits `CoreEvent::Ready`.
3. GUI launches and starts receiving events.
4. After `window.present()`, GUI sends:
   - `CoreCommand::SetMeteringEnabled(true)`
   - `CoreCommand::RequestSnapshot`

### Why `RequestSnapshot` exists

`wait_for_event` in `app.rs` only waits for `Ready` and drops unrelated events during bootstrap. That means early `DevicesChanged`/`StreamAppeared` events can be lost before GUI listeners are live.

`RequestSnapshot` triggers `resend_initial_state()` in core to re-emit current devices/streams once the GUI is ready.

## Core Loop

`PipeWireManager::spawn` runs a `crossbeam_channel::select!` loop across:

- `command_rx` (GUI commands)
- `monitor_rx` (`PwMonitorEvent`s)
- timeout tick (`LOOP_TICK_INTERVAL`)

## Volume & Mute Sync

PipeWire is the source of truth for bus volumes and mutes. Core keeps no
per-channel volume/mute caches of its own; the only cached state is
`last_snapshot` (`volumes`, `mutes` keyed by node id), which mirrors what
`pw-dump --monitor` last reported. The sync flow is:

- **UI → PipeWire:** User drags slider / toggles mute → `CoreCommand::SetVolume` /
  `CoreCommand::SetMute` → `wpctl set-volume` / `wpctl set-mute` on the bus node.
  Targets are resolved strictly from `last_snapshot` (`channel_node_id`); if the
  Venturi node is missing the command fails with `CoreEvent::Error` rather than
  falling back to `@DEFAULT_AUDIO_SINK@`/`@DEFAULT_AUDIO_SOURCE@`.
- **PipeWire → UI:** `pw-dump --monitor` reports a `Props` change → core updates
  `last_snapshot` and emits `CoreEvent::VolumeChanged` / `CoreEvent::MuteChanged`.
  This covers changes made by Venturi itself, hotkeys, `wpctl`, pavucontrol, or
  hardware keys, so hotkey state and the GUI never drift from PipeWire.
- **GUI model:** `MixerTab` (shared `Arc<Mutex<_>>`) is the single GUI-side copy
  of channel state. Widgets write user changes into it directly and the 33 ms
  slider tick syncs widgets *from* it; a short grace window after user input
  (`USER_INPUT_GRACE`) plus `suppress_signal`/`is_dragging` flags prevent the
  tick from snapping a slider or mute button back before PipeWire confirms.
- **Hotkeys:** key autorepeat is filtered (`HeldKeys`) so a held mute key toggles
  once per press.

## Default Device Following

`last_snapshot.default_sink` / `default_source` mirror the `default` metadata
object (`default.audio.sink` / `default.audio.source`). In monitor mode pw-dump
only re-emits changed keys, so they are merged per key. When the user's selection
is "Default" and the corresponding default changes, core re-runs the matching
route reconcile.

The virtual mic master is resolved by `resolve_virtual_mic_master`: a specific
selection wins; otherwise the system default source is used unless it is a
Venturi node (e.g. the user set `Venturi-VirtualMic` as their system default),
in which case the first non-Venturi hardware input is used. This prevents the
remap module from being pointed at itself, which produced a silent mic.

## Routing

All stream routing uses `pw-metadata target.object` (with `target.node` legacy fallback). There is no dual routing mode — metadata routing is the only mechanism.

## Metering

The meter worker samples 6 fixed Venturi-owned bus targets:

- `Venturi-Output` (Main)
- `Venturi-Game`, `Venturi-Media`, `Venturi-Chat`, `Venturi-Aux`
- `Venturi-VirtualMic` (Mic)

Meters are only active while the window is visible. Per-app stream metering is not performed — app volume remains app/OS-controlled.

## Routing & Channel Control Boundaries

### `Main`

- Control target: `Venturi-Output` sink
- Physical device selection handled by loopback reconciliation from `Venturi-Output.monitor` to selected output

### `Mic`

- Control target: `Venturi-VirtualMic` source
- Selected physical input is rewired into this virtual mic source

### App channels (`Game`, `Media`, `Chat`, `Aux`)

- Control target: Venturi channel mix sinks (`Venturi-Game`, `Venturi-Media`, `Venturi-Chat`, `Venturi-Aux`)
- Category slider controls each channel bus volume — app-local source volume remains app/OS controlled
- Classification: `classify_with_priority(overrides, binary, app_name, media_role)`

## CLI Tools Used

- **`pw-dump --monitor`** — change notification stream
- **`wpctl`** — volume/mute control
- **`pw-metadata`** — stream routing target assignment
- **`pactl`** — null sinks / loopback / remap modules
- **`pw-play`** — soundboard playback
- **`pw-record`** — bus-level metering

## Failure Handling

Monitor process failures are handled with backoff and a circuit breaker:

- restart delay
- consecutive failure tracking in a time window
- give-up path emits `CoreEvent::Error`
- successful `InitialSnapshot` resets failure counters
