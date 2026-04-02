#!/usr/bin/env bash
#
# Install Venturi desktop integration (app launcher entry, icon, autostart).
# Works with cargo install, mise, or a local build.
#
# Usage:
#   ./scripts/install-desktop.sh          # install
#   ./scripts/install-desktop.sh remove   # uninstall
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
AUTOSTART_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"

DESKTOP_FILE="venturi.desktop"
AUTOSTART_FILE="venturi-autostart.desktop"
ICON_FILE="org.venturi.Venturi.svg"
LEGACY_SYMBOLIC_ICON_FILE="org.venturi.Venturi-symbolic.svg"

# Resolve the absolute path to the venturi binary.
# Checks (in order): mise, cargo bin, PATH, local release build.
find_venturi_bin() {
    # 1. mise install location (handles both short and URL-style package names)
    local mise_dir="${XDG_DATA_HOME:-$HOME/.local/share}/mise/installs"
    local mise_bin
    mise_bin="$(find "$mise_dir" -path '*venturi*/bin/venturi' -type f 2>/dev/null | head -1)"
    if [[ -n "$mise_bin" && -x "$mise_bin" ]]; then
        echo "$mise_bin"
        return
    fi

    # 2. cargo bin
    local cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin/venturi"
    if [[ -x "$cargo_bin" ]]; then
        echo "$cargo_bin"
        return
    fi

    # 3. Already on PATH (e.g. system package)
    if command -v venturi &>/dev/null; then
        command -v venturi
        return
    fi

    # 4. Local release build in this repo
    local local_bin="$PROJECT_DIR/target/release/venturi"
    if [[ -x "$local_bin" ]]; then
        echo "$local_bin"
        return
    fi

    return 1
}

remove() {
    echo "Removing Venturi desktop integration..."
    rm -f "$APP_DIR/$DESKTOP_FILE"
    rm -f "$ICON_DIR/$ICON_FILE"
    rm -f "$ICON_DIR/$LEGACY_SYMBOLIC_ICON_FILE"
    rm -f "$AUTOSTART_DIR/$AUTOSTART_FILE"
    update-desktop-database "$APP_DIR" 2>/dev/null || true
    gtk-update-icon-cache -f -t "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" 2>/dev/null || true
    echo "Done."
}

install() {
    local venturi_bin
    if ! venturi_bin="$(find_venturi_bin)"; then
        echo "Error: Could not find the venturi binary." >&2
        echo "Install it first with one of:" >&2
        echo "  cargo install --path ." >&2
        echo "  mise install cargo:https://github.com/cfbender/venturi" >&2
        exit 1
    fi

    echo "Installing Venturi desktop integration..."
    echo "  Binary: $venturi_bin"

    mkdir -p "$APP_DIR" "$ICON_DIR" "$AUTOSTART_DIR"

    # Application launcher entry (with absolute path to binary)
    cat > "$APP_DIR/$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Venturi
Comment=Linux audio mixer for PipeWire
Exec=$venturi_bin
Icon=org.venturi.Venturi
Categories=AudioVideo;Audio;
Terminal=false
EOF
    echo "  Installed $APP_DIR/$DESKTOP_FILE"

    # Application icon
    rm -f "$ICON_DIR/$LEGACY_SYMBOLIC_ICON_FILE"
    cp "$PROJECT_DIR/data/$ICON_FILE" "$ICON_DIR/$ICON_FILE"
    echo "  Installed $ICON_DIR/$ICON_FILE"

    # Autostart entry (launches in daemon mode so it runs in the tray)
    cat > "$AUTOSTART_DIR/$AUTOSTART_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Venturi
Comment=Linux audio mixer for PipeWire
Exec=$venturi_bin --daemon
Icon=org.venturi.Venturi
Categories=AudioVideo;Audio;
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
    echo "  Installed $AUTOSTART_DIR/$AUTOSTART_FILE"

    # Update caches
    update-desktop-database "$APP_DIR" 2>/dev/null || true
    gtk-update-icon-cache -f -t "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" 2>/dev/null || true

    echo "Done. Venturi should now appear in your app launcher and start on login."
}

case "${1:-install}" in
    remove|uninstall)
        remove
        ;;
    install|"")
        install
        ;;
    *)
        echo "Usage: $0 [install|remove]"
        exit 1
        ;;
esac
