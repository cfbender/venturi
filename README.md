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

### Flatpak

Manifest: `flatpak/org.venturi.Venturi.json`

Dry-run/lint (manifest parse only):

```bash
flatpak-builder --show-manifest flatpak-build flatpak/org.venturi.Venturi.json
```

Local build:

```bash
flatpak-builder --force-clean flatpak-build flatpak/org.venturi.Venturi.json
```

### Debian (.deb)

```bash
cargo install cargo-deb
cargo deb
```

### AppImage

Config scaffold: `AppImageBuilder.yml`

```bash
cargo install cargo-appimage
cargo appimage
```
