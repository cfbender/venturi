# Venturi — Design Specification

> **Version:** 1.0 · **Date:** 2026-03-29 · **Status:** Draft

## 1. Overview

Venturi is a Linux audio mixer application that provides per-application volume control, channel-based mixing, and virtual audio routing through PipeWire. It targets the same use case as Voicemeeter (Windows) and SteelSeries Sonar — giving gamers, streamers, and power users fine-grained control over their audio mix.

### 1.1 Goals

- **Channel-based mixing** — Route application audio into logical channels (Game, Media, Chat, Aux) with independent volume control
- **Virtual audio routing** — Create a virtual output (mixed audio) and virtual input (mic + soundboard) for use in OBS, Discord, etc.
- **Smart app categorization** — Automatically sort applications into channels using heuristics, with persistent user overrides
- **Active mixing console** — Designed to stay open during gaming/streaming sessions, with global hotkey support
- **Cross-distro packaging** — Flatpak-first, with cargo-deb and AppImage as secondary targets

### 1.2 Non-Goals (v1)

- Multiple output mixes (e.g., separate Monitor vs Stream) — architected for but not exposed in v1
- Audio effects beyond noise gate (EQ, compression, etc.)
- Profile switching / multiple configurations
- Remote control / mobile app
- JACK-only support (PipeWire only)

---

## 2. Architecture

### 2.1 High-Level Design

Single Rust binary with two logical halves:

```
┌─────────────────────────────────────────────┐
│                  venturi                     │
│                                             │
│  ┌─────────────────┐  ┌──────────────────┐  │
│  │      Core        │  │       GUI        │  │
│  │                  │  │                  │  │
│  │  PipeWire Mgr    │◄─┤  Channel Strips  │  │
│  │  Config Store    │  │  App Chips + DnD │  │
│  │  Hotkey Listener │  │  Soundboard      │  │
│  │  Tray Icon       │  │  Settings Panel  │  │
│  │  App Categorizer │──►                  │  │
│  └─────────────────┘  └──────────────────┘  │
│         ▲                                   │
│         │ pipewire-rs                       │
│  ───────┼───────────────────────────────────│
│         ▼                                   │
│    PipeWire daemon                          │
└─────────────────────────────────────────────┘
```

**Core** (always running):
- PipeWire manager — enumerates nodes/ports/links, creates virtual devices, manages routing
- Config store — reads/writes TOML config at `~/.config/venturi/config.toml`
- Hotkey listener — global shortcuts via `ashpd` (Wayland) / `global-hotkey` (X11)
- Tray icon — system tray presence, show/hide window
- App categorizer — classifies audio streams into channels

**GUI** (optional, toggle-able):
- GTK4 + libadwaita window with three tabs
- Communicates with Core via in-process Rust channels (`tokio::sync` or `std::sync::mpsc`)
- Can be hidden while Core continues operating
- `--daemon` flag starts without GUI

### 2.2 Threading Model

```
┌──────────────────┐     ┌──────────────────┐
│   GTK Main Loop  │     │  PipeWire Loop   │
│   (main thread)  │◄───►│  (dedicated      │
│                  │     │   thread)         │
│  UI rendering    │     │  Node/port enum   │
│  User input      │     │  Link management  │
│  DnD handling    │     │  Volume control   │
│                  │     │  VU meter data    │
└──────────────────┘     └──────────────────┘
        ▲                        ▲
        │                        │
        ▼                        ▼
┌──────────────────────────────────────┐
│         Shared State (Arc<Mutex<>>)  │
│                                      │
│  - Channel volumes & mute states     │
│  - App-to-channel assignments        │
│  - Node/port registry snapshot       │
│  - VU meter levels                   │
└──────────────────────────────────────┘
```

- **GTK main loop** runs on the main thread (GTK requirement)
- **PipeWire event loop** runs on a dedicated thread (pipewire-rs is `!Send`/`!Sync`, all PW calls happen here)
- All PipeWire proxies are owned exclusively by the PW thread — never shared

**Command/Event Pattern:**
- **GUI → PW thread** (command channel): `SetVolume(channel, f32)`, `SetMute(channel, bool)`, `MoveStream(stream_id, channel)`, `SetOutputDevice(node_name)`, `SetInputDevice(node_name)`, `PlaySound(pad_id)`, `StopSound(pad_id)`
- **PW thread → GUI** (event channel): `StreamAppeared { id, name, category }`, `StreamRemoved(id)`, `LevelsUpdate(HashMap<Channel, (f32, f32)>)`, `DevicesChanged(Vec<Device>)`, `Error(String)`
- VU meter levels use `Arc<AtomicU32>` per channel (lock-free — encode f32 via `f32::to_bits()`/`from_bits()`) to avoid mutex contention at ~20fps update rate
- Larger state (app-to-channel map, device list) uses `Arc<Mutex<State>>` for infrequent reads

### 2.3 PipeWire Graph

Venturi creates the following virtual PipeWire nodes:

```
App Streams                    Venturi Virtual Devices          Physical
───────────                    ───────────────────────          ────────

[Game apps]  ──► Venturi-Game  ─┐
[Media apps] ──► Venturi-Media ─┼──► Venturi-Output ──► Speakers/Headphones
[Chat apps]  ──► Venturi-Chat  ─┤
[Aux apps]   ──► Venturi-Aux   ─┘

[Microphone] ──► Venturi-Mic   ─┬──► Venturi-VirtualMic ──► Discord/OBS
[Soundboard] ──► Venturi-Sound ─┘
```

**Virtual nodes created by Venturi:**

| Node | Type | Purpose |
|------|------|---------|
| `Venturi-Game` | null-sink | Receives game audio |
| `Venturi-Media` | null-sink | Receives media/music audio |
| `Venturi-Chat` | null-sink | Receives voice chat audio |
| `Venturi-Aux` | null-sink | Receives uncategorized audio |
| `Venturi-Mic` | null-sink | Mic passthrough with processing |
| `Venturi-Sound` | null-sink | Soundboard audio playback |
| `Venturi-Output` | null-sink | Mixed output (all channels combined) |
| `Venturi-VirtualMic` | null-source | Virtual mic (mic + soundboard mix) |

**Routing operations:**
1. On app stream appearance → categorize → set stream's `target.node` metadata to appropriate Venturi channel sink (session-manager-friendly, avoids link races)
2. Channel volume changes → set `SPA_PROP_channelVolumes` on the channel's null-sink node
3. Main volume → controls `Venturi-Output` volume (post-mix master)
4. Mic volume → controls `Venturi-Mic` passthrough level
5. Mute → set volume to 0.0 on the corresponding node

**Mixing topology:**
- Each channel null-sink has **monitor output ports** that mirror received audio
- Mix channel audio into `Venturi-Output` by linking each channel sink's **monitor output ports** → `Venturi-Output`'s **input ports**
- PipeWire automatically sums overlapping links on the same input ports — no manual mixing needed
- Same pattern for virtual mic: `Venturi-Mic` monitor ports + `Venturi-Sound` monitor ports → `Venturi-VirtualMic` input ports

**Session manager (WirePlumber) interaction:**
- Set `node.autoconnect = false` on all Venturi-created nodes to prevent WirePlumber from managing them
- Set `node.passive = true` on monitor links to avoid affecting PipeWire's scheduling
- Use `target.node` metadata (via `pw-metadata` API) to redirect app streams — this is the session-manager-friendly approach that WirePlumber respects rather than fights
- On startup, check for stale `Venturi-*` nodes from a previous crashed instance (match by `node.name` prefix) and destroy them before creating fresh ones

### 2.4 Module Structure

```
venturi/
├── Cargo.toml
├── src/
│   ├── main.rs                 # CLI args, app init, thread spawning
│   ├── app.rs                  # Application state, Core↔GUI bridge
│   ├── core/
│   │   ├── mod.rs
│   │   ├── pipewire_manager.rs # PW event loop, node/link management
│   │   ├── virtual_devices.rs  # Create/destroy null-sinks
│   │   ├── router.rs           # Stream routing (move apps between channels)
│   │   ├── volume.rs           # Volume/mute control via SPA props
│   │   ├── meter.rs            # VU meter level extraction
│   │   └── hotkeys.rs          # Global hotkey registration
│   ├── categorizer/
│   │   ├── mod.rs
│   │   ├── rules.rs            # Static + heuristic classification rules
│   │   └── learning.rs         # Persistent user override storage
│   ├── config/
│   │   ├── mod.rs
│   │   ├── schema.rs           # TOML config structure (serde)
│   │   └── persistence.rs      # Read/write ~/.config/venturi/
│   ├── audio/
│   │   ├── mod.rs
│   │   ├── noise_gate.rs       # Noise gate processing
│   │   └── soundboard.rs       # Sound file loading & playback
│   ├── gui/
│   │   ├── mod.rs
│   │   ├── window.rs           # Main window + tab structure
│   │   ├── mixer_tab.rs        # Mixer view layout
│   │   ├── channel_strip.rs    # Custom widget: vertical strip
│   │   ├── vu_meter.rs         # Custom widget: stereo VU meter
│   │   ├── app_chip.rs         # Custom widget: draggable app badge
│   │   ├── soundboard_tab.rs   # Soundboard grid view
│   │   └── settings_tab.rs     # Settings panel
│   └── tray.rs                 # System tray icon
├── data/
│   ├── icons/                  # App icons, channel icons
│   ├── venturi.desktop         # Desktop entry
│   └── org.venturi.Venturi.metainfo.xml  # AppStream metadata
├── flatpak/
│   └── org.venturi.Venturi.json  # Flatpak manifest
└── docs/
    └── superpowers/specs/
        └── venturi-design-spec.md  # This document
```

---

## 3. User Interface

### 3.1 Window Shell

- **Toolkit:** GTK4 + libadwaita
- **Window:** `adw::ApplicationWindow` with `adw::HeaderBar`
- **Navigation:** `adw::ViewStack` with `adw::ViewSwitcher` for tab bar
- **Tabs:** Mixer | Soundboard | Settings
- **Default size:** ~900×600px, resizable
- **Theme:** Follows system dark/light preference via libadwaita

### 3.2 Mixer Tab

```
┌─────────────────────────────────────────────────────────────────┐
│  🔊 Output: Headphones (HD Audio) ▾    🎤 Input: Blue Yeti ▾   │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────┐ │ ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐ │ ┌─────┐         │
│  │ 🔊  │ │ │ 🎮  │ │ 🎵  │ │ 💬  │ │ 📦  │ │ │ 🎤  │         │
│  │Main │ │ │Game │ │Media│ │Chat │ │ Aux │ │ │ Mic │         │
│  │     │ │ │     │ │     │ │     │ │     │ │ │     │         │
│  │ ▐▌  │ │ │ ▐▌  │ │ ▐▌  │ │ ▐▌  │ │ ▐▌  │ │ │ ▐▌  │         │
│  │ ██  │ │ │ ██  │ │ ██  │ │ ██  │ │ ██  │ │ │ ██  │         │
│  │ ██  │ │ │ ██  │ │ ██  │ │ ██  │ │ ██  │ │ │ ██  │         │
│  │ ██  │ │ │ ██  │ │ ██  │ │ ██  │ │ ██  │ │ │ ██  │         │
│  │ ██  │ │ │ ██  │ │ ██  │ │ ██  │ │ ██  │ │ │ ██  │         │
│  │-0dB │ │ │-6dB │ │-3dB │ │-12dB│ │-inf │ │ │-0dB │         │
│  │ 🔇  │ │ │ 🔇  │ │ 🔇  │ │ 🔇  │ │ 🔇  │ │ │ 🔇  │         │
│  │     │ │ │     │ │     │ │     │ │     │ │ │     │         │
│  │     │ │ │┌───┐│ │┌───┐│ │┌───┐│ │┌───┐│ │ │     │         │
│  │     │ │ ││CS2││ ││Spo││ ││Dis││ ││mpv││ │ │     │         │
│  │     │ │ │└───┘│ │└───┘│ │└───┘│ │└───┘│ │ │     │         │
│  │     │ │ │     │ │┌───┐│ │     │ │     │ │ │     │         │
│  │     │ │ │     │ ││Fir││ │     │ │     │ │ │     │         │
│  │     │ │ │     │ │└───┘│ │     │ │     │ │ │     │         │
│  └─────┘ │ └─────┘ └─────┘ └─────┘ └─────┘ │ └─────┘         │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Channel strips** (left to right, separated by visual dividers):

| Channel | Color | Purpose | Has App Chips |
|---------|-------|---------|---------------|
| Main | Blue (`#3584e4`) | Master output volume | No |
| Game | Red (`#e01b24`) | Game audio | Yes |
| Media | Green (`#33d17a`) | Music, video, browser | Yes |
| Chat | Yellow (`#f6d32d`) | Voice chat apps | Yes |
| Aux | Cyan (`#33c7de`) | Catch-all / uncategorized | Yes |
| Mic | Purple (`#9141ac`) | Microphone input | No |

**Each channel strip contains (top to bottom):**
1. Channel icon (emoji or symbolic icon)
2. Channel label
3. Stereo VU meter (thin vertical bars, L/R)
4. Vertical slider (gtk4::Scale, inverted so up = louder)
5. dB readout label (e.g., "-6.0 dB")
6. Mute toggle button
7. App chips area (Game/Media/Chat/Aux only)

**App chips:**
- Small rounded badges showing truncated app name
- Status dot: green (playing), gray (idle), red (muted)
- Click-to-mute icon on hover
- Draggable — drop onto another channel strip to reassign
- Assignment persists in config

**Device selectors:**
- Top bar with two dropdown selectors
- Output: lists PipeWire audio sinks (speakers, headphones)
- Input: lists PipeWire audio sources (microphones)
- Changing selection re-routes Venturi-Output / Venturi-Mic

### 3.3 Soundboard Tab

```
┌──────────────────────────────────────────────────┐
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐    │
│  │  🔔    │ │  👏    │ │  📯    │ │  🥁    │ ...│
│  │ Airhorn│ │ Applaus│ │ Sadtrom│ │ Rimshot│    │
│  └────────┘ └────────┘ └────────┘ └────────┘    │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐    │
│  │  ➕    │ │  ➕    │ │  ➕    │ │  ➕    │ ...│
│  │  Add   │ │  Add   │ │  Add   │ │  Add   │    │
│  └────────┘ └────────┘ └────────┘ └────────┘    │
│                                                  │
└──────────────────────────────────────────────────┘
```

- 5-column responsive grid of pads
- Click pad → plays sound file through `Venturi-Sound` sink → mixed into `Venturi-VirtualMic`
- Empty slots show "+" button → opens file chooser
- Right-click → context menu: rename, change icon, change file, remove
- Supports: WAV, OGG, MP3, FLAC (via `rodio` or `symphonia` crate)
- Playing indicator: pad pulses/highlights while sound is active
- Stop-on-click: clicking an active pad stops it

### 3.4 Settings Tab

```
┌──────────────────────────────────────────────────┐
│  Microphone Processing                           │
│  ┌──────────────────────────────────────┐        │
│  │ Noise Gate          [Toggle: ON/OFF] │        │
│  │ Threshold      ───●──────── -40 dB   │        │
│  └──────────────────────────────────────┘        │
│                                                  │
│  Hotkeys                                         │
│  ┌──────────────────────────────────────┐        │
│  │ Mute Main        [Ctrl+Shift+M    ] │        │
│  │ Mute Mic         [Ctrl+Shift+N    ] │        │
│  │ Push-to-Talk     [Mouse5          ] │        │
│  │ Toggle Window    [Ctrl+Shift+V    ] │        │
│  └──────────────────────────────────────┘        │
│                                                  │
│  Configuration                                   │
│  ┌──────────────────────────────────────┐        │
│  │ Config path: ~/.config/venturi/      │        │
│  │ [Open in file manager]               │        │
│  └──────────────────────────────────────┘        │
│                                                  │
│  About Venturi                                   │
│  ┌──────────────────────────────────────┐        │
│  │ Version 0.1.0                        │        │
│  │ PipeWire: 1.2.x · GTK: 4.x          │        │
│  └──────────────────────────────────────┘        │
└──────────────────────────────────────────────────┘
```

- **Noise gate:** Toggle switch + threshold slider (-60dB to 0dB)
- **Hotkeys:** Configurable key bindings. Click to record new binding. Uses `ashpd` GlobalShortcuts on Wayland, `global-hotkey` on X11.
- **Config path:** Read-only display with "Open in file manager" button
- **About:** Version info, detected PipeWire and GTK versions

---

## 4. App Categorization

### 4.1 Classification Pipeline

When a new audio stream appears in PipeWire:

```
New Stream → Check User Overrides → Check Static Map → Check Heuristics → Aux (default)
```

### 4.2 Data Sources

From PipeWire node properties:
- `application.name` — e.g., "Firefox", "Discord", "Steam"
- `application.process.binary` — e.g., "firefox", "discord", "steam"
- `media.role` — PipeWire role hint: "Game", "Music", "Communication", etc.
- `media.class` — "Stream/Output/Audio", "Stream/Input/Audio"
- `application.id` — desktop file ID
- `node.name` — PipeWire node name

### 4.3 Classification Rules

**Matching key:** All rules match against `application.process.binary` (lowercased). If that property is absent, fall back to `application.name` (lowercased). The matching key is stored in user overrides.

**Priority 1 — User overrides** (stored in config):
```toml
[categorizer.overrides]
"discord" = "Chat"
"teamspeak" = "Chat"
"my-custom-game" = "Game"
```

**Priority 2 — Static mapping** (built-in):
```rust
// Game: known game launchers and engines
"steam", "gamescope", "lutris", "heroic" => Game
// Media: media players, browsers
"firefox", "chromium", "spotify", "vlc", "mpv" => Media
// Chat: voice/video apps
"discord", "mumble", "teamspeak", "zoom", "slack" => Chat
```

**Priority 3 — PipeWire metadata heuristics:**
```rust
match media_role {
    "Game" => Game,
    "Music" | "Movie" | "Video" => Media,
    "Communication" | "Phone" => Chat,
    _ => continue to default
}
```

**Priority 4 — Default:** Aux (catch-all)

### 4.4 Learning

When the user drags an app chip from one channel to another:
1. Store mapping: `app_binary → new_channel` in config
2. This becomes a Priority 1 override for all future sessions
3. Any existing streams from that app are immediately re-routed

---

## 5. Configuration

### 5.1 File Location

```
~/.config/venturi/
├── config.toml          # Main configuration (dotfile-friendly, no runtime state)
└── soundboard/          # Soundboard audio files (copied here)

~/.local/state/venturi/
└── state.toml           # Runtime state (volumes, mute states — not meant for dotfiles)
```

Follows XDG Base Directory specification. `$XDG_CONFIG_HOME/venturi/` for config, `$XDG_STATE_HOME/venturi/` for state (defaults to `~/.local/state/venturi/`).

### 5.2 Config Schema

```toml
[general]
version = 1
start_minimized = false
show_tray_icon = true

[audio]
output_device = "alsa_output.usb-SteelSeries..."  # PW node name, or "default"
input_device = "alsa_input.usb-Blue_Yeti..."       # PW node name, or "default"

[mic_processing]
noise_gate_enabled = true
noise_gate_threshold = -40.0  # dB

[categorizer.overrides]
"discord" = "Chat"
"obs" = "Media"
# User drag-and-drop overrides get appended here

[hotkeys]
mute_main = "ctrl+shift+m"
mute_mic = "ctrl+shift+n"
push_to_talk = ""             # Disabled by default
toggle_window = "ctrl+shift+v"

[[soundboard.pads]]
name = "Airhorn"
file = "airhorn.wav"
icon = "🔔"

[[soundboard.pads]]
name = "Applause"
file = "applause.ogg"
icon = "👏"
```

### 5.3 State File

Runtime state is stored separately from config so the config stays dotfile-friendly:

**Location:** `$XDG_STATE_HOME/venturi/state.toml` (defaults to `~/.local/state/venturi/state.toml`)

```toml
[volumes]
main = 1.0
game = 0.75
media = 0.6
chat = 0.5
aux = 0.8
mic = 1.0

[muted]
main = false
game = false
media = false
chat = false
aux = false
mic = false
```

- Saved on every volume/mute change (debounced — 500ms after last change)
- If missing or corrupt, silently start with defaults (unity volume, unmuted)
- Not intended for manual editing or version control

### 5.4 Config Behavior

- Config loaded at startup, saved on every change (debounced — 500ms after last change)
- If config file doesn't exist, create with defaults
- If config file is malformed, log warning and use defaults (don't crash)
- Config changes from GUI are written immediately (after debounce)
- **Volume and mute states live in the state file, not config** — config stays clean for dotfile management
- Config `version` field enables future schema migration: on load, check version and apply transforms if needed (rename-and-migrate approach, never silently drop fields)

---

## 6. PipeWire Integration

### 6.1 Lifecycle

1. **Startup:**
   - Connect to PipeWire daemon via `pipewire-rs`
   - Detect and destroy stale `Venturi-*` nodes from a previous crashed instance (match by `node.name` prefix)
   - Create virtual null-sink nodes (Venturi-Game, Venturi-Media, etc.) with `node.autoconnect = false`
   - Create virtual source node (Venturi-VirtualMic)
   - Set up internal mixing links (channel sink monitor ports → Venturi-Output input ports)
   - Register for `global` / `global_remove` events
   - Apply saved config (device selections, categorizer overrides)
   - Restore channel volumes and mute states from state file (or defaults if missing)
   - Scan existing streams and categorize

2. **Runtime:**
   - React to new streams → categorize → route
   - React to stream removal → clean up app chips
   - React to volume/mute changes from GUI → update PW node properties
   - React to device changes → re-route output/input
   - Feed VU meter data to GUI (~20 fps)

3. **Shutdown:**
   - Save config
   - Destroy all Venturi virtual nodes and links
   - Disconnect from PipeWire

### 6.2 Virtual Device Creation

Using PipeWire's factory system via `Core::create_object()`:

```rust
// Pseudocode — null-sink (for channel sinks like Venturi-Game)
core.create_object(
    "adapter",               // factory name
    "PipeWire:Interface:Node",
    PW_VERSION_NODE,
    &properties! {
        "factory.name" => "support.null-audio-sink",
        "node.name" => "Venturi-Game",
        "node.description" => "Venturi Game Channel",
        "media.class" => "Audio/Sink",
        "audio.channels" => "2",
        "audio.position" => "FL,FR",
        "node.autoconnect" => "false",  // Prevent WirePlumber from managing
    },
);

// Pseudocode — virtual source (for Venturi-VirtualMic)
// Other apps (Discord, OBS) will see this as an input device
core.create_object(
    "adapter",
    "PipeWire:Interface:Node",
    PW_VERSION_NODE,
    &properties! {
        "factory.name" => "support.null-audio-sink",
        "node.name" => "Venturi-VirtualMic",
        "node.description" => "Venturi Virtual Microphone",
        "media.class" => "Audio/Source/Virtual",  // Appears as a microphone to other apps
        "audio.channels" => "2",
        "audio.position" => "FL,FR",
        "node.autoconnect" => "false",
    },
);
```

Note: `Audio/Source/Virtual` media class makes the node appear as a selectable microphone in applications like Discord and OBS. The null-audio-sink factory with this class creates a sink whose monitor ports act as the source output.

### 6.3 Stream Routing

To move an app's audio to a Venturi channel sink:

**Primary approach (session-manager-friendly):**
1. Use PipeWire metadata API to set `target.node` on the app's stream node, pointing to the target Venturi channel sink's node ID
2. WirePlumber respects `target.node` metadata and handles the actual link creation/destruction
3. This avoids race conditions with the session manager

**Fallback (if metadata approach insufficient):**
1. Find the app's output port IDs via the registry
2. Find the target channel sink's input port IDs
3. Destroy existing links from the app's ports
4. Create new links: app output ports → channel sink input ports
5. Set `node.dont-reconnect = true` on the stream to prevent WirePlumber from overriding

### 6.4 Volume Control

```rust
// Set channel volume via SPA property
let values = [volume_linear, volume_linear]; // L, R
node_proxy.set_param(
    spa::param::ParamType::Props,
    0,
    spa::pod::object! {
        spa::param::ParamType::Props,
        spa::prop::channelVolumes => &values,
    },
);
```

Volume range: 0.0 (silence / -∞ dB) to 1.5 (~3.5 dB boost)

dB conversion: `dB = 20 * log10(linear)`, `linear = 10^(dB/20)`

**Slider curve:** Use cubic mapping for natural feel: `linear = slider_position³ * 1.5`. This gives fine control at low volumes (most of slider travel) and coarser control at high volumes.

**dB display:** 0.0 → "-∞ dB", 1.0 → "0.0 dB", 1.5 → "+3.5 dB". Keyboard step: 1dB increments.

Note: Channel volume controls the null-sink input gain — all apps feeding that channel are scaled uniformly. Per-app volume (post-v1) would control individual stream node volumes instead.

### 6.5 VU Meters

Option A (preferred): Use PipeWire's built-in peak monitoring by subscribing to profiler data from nodes.

Option B (fallback): Create PipeWire streams that tap into each channel's audio for level measurement.

Target refresh: ~20 updates/sec to GUI for smooth meters. Ballistics: peak-hold with 300ms decay (standard PPM-style).

VU levels communicated via `Arc<AtomicU32>` per channel (lock-free) — PW thread writes, GTK thread reads.

---

## 7. Audio Processing

### 7.1 Noise Gate

Simple threshold-based gate applied to mic input:

```
Input Level > Threshold → pass through (gain = 1.0)
Input Level < Threshold → mute (gain = 0.0)
```

Parameters:
- **Threshold:** -60dB to 0dB (configurable)
- **Attack:** 1ms (instant open)
- **Release:** 100ms (gradual close to avoid cutting words)

Implementation: Use a PipeWire `Filter` (`pw_filter`) node inserted between the physical mic and `Venturi-Mic` node. The filter's `process` callback receives audio buffers for per-sample gate processing. This is distinct from `pw_stream` (which is for playback/capture) — `pw_filter` provides in-line DSP processing.

### 7.2 Soundboard

- Load audio files into memory at startup (or on-demand for large files)
- Single PipeWire `Stream` for all soundboard audio — decode files with `symphonia`, software-mix active pads in a Rust buffer, write mixed output to PW stream
- Mix with mic audio at `Venturi-VirtualMic` (via PipeWire links — Venturi-Sound monitor ports + Venturi-Mic monitor ports → VirtualMic input ports)
- Support simultaneous playback of multiple pads (mixed in software before PW)
- File formats: WAV, OGG, MP3, FLAC (via `symphonia` crate for decoding)

---

## 8. Global Hotkeys

### 8.1 Backend Selection

```rust
// Try Wayland portal first, fall back to X11
async fn init_hotkeys() -> HotkeyBackend {
    // Attempt Wayland GlobalShortcuts portal
    if let Ok(proxy) = ashpd::desktop::global_shortcuts::GlobalShortcuts::new().await {
        if let Ok(session) = proxy.create_session().await {
            return HotkeyBackend::Wayland(session);
        }
    }
    // Fallback to X11 (works on X11 and XWayland)
    HotkeyBackend::X11
}
```

Rationale: Checking `$XDG_SESSION_TYPE` alone is unreliable (may be unset, XWayland edge cases). Instead, attempt the Wayland portal — if it succeeds, use it; otherwise fall back to X11.

### 8.2 Default Bindings

| Action | Default Binding |
|--------|----------------|
| Mute Main | `Ctrl+Shift+M` |
| Mute Mic | `Ctrl+Shift+N` |
| Push-to-Talk | *(disabled)* |
| Toggle Window | `Ctrl+Shift+V` |

### 8.3 Wayland Considerations

The XDG Desktop Portal `GlobalShortcuts` interface shows a system dialog where the user picks the key combination. Venturi suggests a default but the user has final say. This is by design (Wayland security model).

Push-to-talk requires key-down/key-up detection. The `ashpd` GlobalShortcuts portal provides `Activated`/`Deactivated` signals which support this. The X11 `global-hotkey` crate supports key-release events.

---

## 8.5 Error States

| Condition | UI Behavior |
|-----------|------------|
| PipeWire daemon not running | Show banner: "PipeWire not detected. Start PipeWire and restart Venturi." Disable all controls. |
| PipeWire connection lost | Show banner: "Connection to PipeWire lost. Reconnecting..." Auto-retry every 2s. |
| Output device disconnected | Reset output selector to "Default". Show brief toast notification. |
| Input device disconnected | Reset input selector to "Default". Show brief toast notification. |
| No audio devices available | Device selectors show "No devices found". Mixing controls remain visible but non-functional. |
| Config file corrupt | Log warning, load defaults, show toast: "Config was reset due to errors." |

---

## 9. Technology Stack

| Component | Crate | Version |
|-----------|-------|---------|
| PipeWire bindings | `pipewire` | `0.9` |
| GTK4 UI | `gtk4` | `0.11` |
| Adaptive UI / theming | `libadwaita` | `0.9` |
| Config serialization | `serde` + `toml` | latest |
| Audio decoding | `symphonia` | latest |
| Wayland hotkeys | `ashpd` | latest |
| X11 hotkeys | `global-hotkey` | `0.7` |
| CLI args | `clap` | `4.x` |
| Logging | `tracing` + `tracing-subscriber` | latest |
| Cross-thread comms | `crossbeam-channel` | latest |

**Logging strategy:** Default log level `info`. Enable debug output via `RUST_LOG=venturi=debug` env var or `--verbose` / `-v` CLI flag. Structured logging with `tracing` spans for PipeWire events, GUI events, and config I/O.

### 9.1 Build Dependencies

```
# Debian/Ubuntu
sudo apt install libpipewire-0.3-dev libgtk-4-dev libadwaita-1-dev pkg-config clang

# Fedora
sudo dnf install pipewire-devel gtk4-devel libadwaita-devel clang

# Arch
sudo pacman -S pipewire gtk4 libadwaita clang
```

### 9.2 Packaging

**Primary: Flatpak**
- Runtime: `org.gnome.Platform` (provides GTK4 + libadwaita)
- SDK: `org.gnome.Sdk` + Rust SDK extension
- PipeWire access: `--socket=pulseaudio` (PipeWire's PulseAudio compat) + `--filesystem=xdg-run/pipewire-0` (direct `pw_core` connection for registry/link management)
- Template: Sonusmix's Flatpak manifest (`org.sonusmix.Sonusmix.json`)

**Secondary: cargo-deb**
- For Debian/Ubuntu direct install
- Declares runtime deps on `libpipewire-0.3-0`, `libgtk-4-1`, `libadwaita-1-0`

**Tertiary: AppImage**
- Bundles GTK4 + libadwaita + PipeWire client libs
- Via `cargo-appimage`

---

## 10. Future Considerations (Post-v1)

These are explicitly out of scope for v1 but the architecture should not preclude them:

- **Multiple output mixes:** Additional virtual outputs for streaming (e.g., "Stream Mix" without mic monitoring)
- **Per-app volume:** Individual volume sliders within channel strips (not just channel-level)
- **Audio effects:** EQ, compressor, de-esser on mic chain
- **Profiles:** Save/load different mixing configurations
- **Client/server split:** Allow remote GUI (phone app, web interface)
- **Plugin system:** User-defined audio processing plugins
- **MIDI control:** Map physical MIDI controllers to channel faders

---

## Appendix A: Key Decisions Log

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | Rust, not C++/Python | Memory safety; excellent PW/GTK bindings; strong ecosystem tooling |
| 2 | Single binary hybrid (Core+GUI) | Simpler v1; in-process channels faster than IPC; `--daemon` for headless |
| 3 | GTK4 + libadwaita, not Qt/egui | Best Linux-native integration; follows GNOME HIG; Flatpak-friendly |
| 4 | TOML config, not JSON/YAML | Human-readable, Rust ecosystem standard (serde support) |
| 5 | Learning categorizer | Best UX: works out of box, gets smarter with use |
| 6 | Aux as catch-all | No audio falls through the cracks; user can always find uncategorized apps |
| 7 | Null-sink per channel | Clean PipeWire graph; independent volume control; easy routing |
| 8 | Single config (no profiles v1) | Reduce complexity; profiles are a clean addition later |
| 9 | Flatpak-first packaging | Broadest reach; GNOME runtime provides all deps |
| 10 | Dual hotkey backend | Necessary for Wayland+X11 coverage; no single solution works everywhere |
