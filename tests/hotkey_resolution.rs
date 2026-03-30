use venturi::core::hotkeys::{HotkeyBackend, choose_backend};

#[test]
fn prefers_wayland_portal_when_available() {
    assert_eq!(choose_backend(true), HotkeyBackend::WaylandPortal);
    assert_eq!(choose_backend(false), HotkeyBackend::X11);
}
