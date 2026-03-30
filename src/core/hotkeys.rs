#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyBackend {
    WaylandPortal,
    X11,
}

pub fn choose_backend(portal_available: bool) -> HotkeyBackend {
    if portal_available {
        HotkeyBackend::WaylandPortal
    } else {
        HotkeyBackend::X11
    }
}
