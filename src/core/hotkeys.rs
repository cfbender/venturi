use crate::config::schema;
use crate::core::messages::{Channel, CoreCommand};
use std::collections::VecDeque;
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum HotkeyAction {
    Pressed { chord: String },
    Released { chord: String },
}

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

impl From<&schema::Hotkeys> for HotkeyBindings {
    fn from(value: &schema::Hotkeys) -> Self {
        Self {
            mute_main: value.mute_main.clone(),
            mute_mic: value.mute_mic.clone(),
            push_to_talk: value.push_to_talk.clone(),
            toggle_window: value.toggle_window.clone(),
        }
    }
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

pub fn collect_adapter_commands(
    adapter: &mut dyn HotkeyAdapter,
    bindings: &HotkeyBindings,
    state: HotkeyState,
) -> Vec<CoreCommand> {
    adapter
        .poll_event()
        .map(|event| commands_for_hotkey_event(&event, bindings, state))
        .unwrap_or_default()
}

#[cfg(test)]
fn action_from_event(event: &HotkeyEvent) -> HotkeyAction {
    match event {
        HotkeyEvent::Pressed(chord) => HotkeyAction::Pressed {
            chord: chord.clone(),
        },
        HotkeyEvent::Released(chord) => HotkeyAction::Released {
            chord: chord.clone(),
        },
    }
}

#[cfg(test)]
fn event_from_action(action: HotkeyAction) -> HotkeyEvent {
    match action {
        HotkeyAction::Pressed { chord } => HotkeyEvent::Pressed(chord),
        HotkeyAction::Released { chord } => HotkeyEvent::Released(chord),
    }
}

fn normalize_chord(raw: &str) -> String {
    let mut has_ctrl = false;
    let mut has_alt = false;
    let mut has_shift = false;
    let mut has_super = false;
    let mut keys = Vec::new();

    for token in raw.split('+') {
        let normalized = token.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }

        match normalized.as_str() {
            "ctrl" | "control" | "primary" => has_ctrl = true,
            "alt" | "option" => has_alt = true,
            "shift" => has_shift = true,
            "super" | "meta" | "win" | "cmd" | "command" => has_super = true,
            _ => keys.push(normalized),
        }
    }

    // Build canonical order: ctrl, alt, shift, super, then keys
    let mut parts: Vec<&str> = Vec::with_capacity(4 + keys.len());
    if has_ctrl {
        parts.push("ctrl");
    }
    if has_alt {
        parts.push("alt");
    }
    if has_shift {
        parts.push("shift");
    }
    if has_super {
        parts.push("super");
    }

    let mut result = parts.join("+");
    if !keys.is_empty() {
        if !result.is_empty() {
            result.push('+');
        }
        result.push_str(&keys.join("+"));
    }
    result
}

#[cfg(test)]
mod tests {
    use crate::config::schema::Hotkeys;
    use crate::core::messages::{Channel, CoreCommand};

    use super::{
        action_from_event, commands_for_hotkey_event, event_from_action, HotkeyBindings,
        HotkeyEvent, HotkeyState,
    };

    #[test]
    fn chord_matching_is_case_and_modifier_order_insensitive() {
        let bindings = HotkeyBindings {
            mute_main: "Ctrl+Shift+M".to_string(),
            mute_mic: String::new(),
            push_to_talk: String::new(),
            toggle_window: String::new(),
        };

        let commands = commands_for_hotkey_event(
            &HotkeyEvent::Pressed("shift+ctrl+m".to_string()),
            &bindings,
            HotkeyState {
                main_muted: false,
                mic_muted: false,
            },
        );

        assert_eq!(commands, vec![CoreCommand::SetMute(Channel::Main, true)]);
    }

    #[test]
    fn chord_matching_accepts_common_modifier_aliases() {
        let config_hotkeys = Hotkeys {
            mute_main: String::new(),
            mute_mic: String::new(),
            push_to_talk: String::new(),
            toggle_window: "Control+Meta+V".to_string(),
        };
        let bindings = HotkeyBindings::from(&config_hotkeys);

        let commands = commands_for_hotkey_event(
            &HotkeyEvent::Pressed("ctrl+super+v".to_string()),
            &bindings,
            HotkeyState {
                main_muted: false,
                mic_muted: false,
            },
        );

        assert_eq!(commands, vec![CoreCommand::ToggleWindow]);
    }

    #[test]
    fn hotkey_adapter_action_roundtrip_is_typed() {
        let event = HotkeyEvent::Pressed("ctrl+shift+v".to_string());
        let action = action_from_event(&event);

        assert_eq!(event_from_action(action), event);
    }
}
