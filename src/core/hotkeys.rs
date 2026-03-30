use crate::core::messages::{Channel, CoreCommand};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyBackend {
    WaylandPortal,
    X11,
}

pub trait HotkeyAdapter {
    fn backend(&self) -> HotkeyBackend;
    fn register(&mut self, _bindings: &HotkeyBindings) -> Result<(), String> {
        Ok(())
    }
    fn poll_event(&mut self) -> Option<HotkeyEvent>;
}

#[derive(Debug, Default)]
pub struct WaylandPortalAdapter {
    queued_events: VecDeque<HotkeyEvent>,
}

impl WaylandPortalAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue_for_test(&mut self, event: HotkeyEvent) {
        self.queued_events.push_back(event);
    }
}

impl HotkeyAdapter for WaylandPortalAdapter {
    fn backend(&self) -> HotkeyBackend {
        HotkeyBackend::WaylandPortal
    }

    fn poll_event(&mut self) -> Option<HotkeyEvent> {
        self.queued_events.pop_front()
    }
}

#[derive(Debug, Default)]
pub struct X11HotkeyAdapter {
    queued_events: VecDeque<HotkeyEvent>,
}

impl X11HotkeyAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue_for_test(&mut self, event: HotkeyEvent) {
        self.queued_events.push_back(event);
    }
}

impl HotkeyAdapter for X11HotkeyAdapter {
    fn backend(&self) -> HotkeyBackend {
        HotkeyBackend::X11
    }

    fn poll_event(&mut self) -> Option<HotkeyEvent> {
        self.queued_events.pop_front()
    }
}

pub fn build_adapter(
    session_type: Option<&str>,
    portal_available: bool,
) -> Box<dyn HotkeyAdapter + Send> {
    match resolve_backend(session_type, portal_available) {
        HotkeyBackend::WaylandPortal => Box::new(WaylandPortalAdapter::new()),
        HotkeyBackend::X11 => Box::new(X11HotkeyAdapter::new()),
    }
}

pub fn choose_backend(portal_available: bool) -> HotkeyBackend {
    if portal_available {
        HotkeyBackend::WaylandPortal
    } else {
        HotkeyBackend::X11
    }
}

pub fn resolve_backend(session_type: Option<&str>, portal_available: bool) -> HotkeyBackend {
    if portal_available {
        return HotkeyBackend::WaylandPortal;
    }

    match session_type.map(str::to_ascii_lowercase).as_deref() {
        Some("x11") => HotkeyBackend::X11,
        Some("wayland") => HotkeyBackend::X11,
        _ => HotkeyBackend::X11,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed(String),
    Released(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyBindings {
    pub mute_main: String,
    pub mute_mic: String,
    pub push_to_talk: String,
    pub toggle_window: String,
}

impl HotkeyBindings {
    pub fn matches_press(&self, event: &HotkeyEvent, binding: &str) -> bool {
        matches!(event, HotkeyEvent::Pressed(chord) if !binding.is_empty() && normalize_chord(chord) == normalize_chord(binding))
    }

    pub fn matches_release(&self, event: &HotkeyEvent, binding: &str) -> bool {
        matches!(event, HotkeyEvent::Released(chord) if !binding.is_empty() && normalize_chord(chord) == normalize_chord(binding))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyState {
    pub main_muted: bool,
    pub mic_muted: bool,
}

pub fn commands_for_hotkey_event(
    event: &HotkeyEvent,
    bindings: &HotkeyBindings,
    state: HotkeyState,
) -> Vec<CoreCommand> {
    if bindings.matches_press(event, &bindings.mute_main) {
        return vec![CoreCommand::SetMute(Channel::Main, !state.main_muted)];
    }

    if bindings.matches_press(event, &bindings.mute_mic) {
        return vec![CoreCommand::SetMute(Channel::Mic, !state.mic_muted)];
    }

    if bindings.matches_press(event, &bindings.push_to_talk) {
        return vec![CoreCommand::SetMute(Channel::Mic, false)];
    }

    if bindings.matches_release(event, &bindings.push_to_talk) {
        return vec![CoreCommand::SetMute(Channel::Mic, true)];
    }

    if bindings.matches_press(event, &bindings.toggle_window) {
        return vec![CoreCommand::ToggleWindow];
    }

    Vec::new()
}

fn normalize_chord(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}
