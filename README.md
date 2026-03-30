# Venturi

Venturi is a Linux audio mixer for PipeWire with channel-based routing, virtual devices, and a mixer-first workflow.

## Build Dependencies

### Debian/Ubuntu

```bash
sudo apt install libpipewire-0.3-dev libgtk-4-dev libadwaita-1-dev pkg-config clang
```

### Fedora

```bash
sudo dnf install pipewire-devel gtk4-devel libadwaita-devel clang
```

### Arch

```bash
sudo pacman -S pipewire gtk4 libadwaita clang
```

## Build & Test

```bash
cargo check
cargo test
```

## Run

```bash
cargo run
cargo run -- --daemon
```

## Packaging

- Flatpak manifest: `flatpak/org.venturi.Venturi.json`
- Debian package tooling: `cargo install cargo-deb && cargo deb`
- AppImage tooling: `cargo install cargo-appimage && cargo appimage`
