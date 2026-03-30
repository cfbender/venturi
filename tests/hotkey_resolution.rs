use venturi::core::hotkeys::{
    HotkeyBackend, HotkeyBindings, HotkeyEvent, HotkeyState, WaylandPortalAdapter, build_adapter,
    choose_backend, collect_adapter_commands, commands_for_hotkey_event, resolve_backend,
};
use venturi::core::messages::{Channel, CoreCommand};

#[test]
fn prefers_wayland_portal_when_available() {
    assert_eq!(choose_backend(true), HotkeyBackend::WaylandPortal);
    assert_eq!(choose_backend(false), HotkeyBackend::X11);
}

#[test]
fn resolve_backend_prefers_portal_even_on_x11() {
    assert_eq!(
        resolve_backend(Some("x11"), true),
        HotkeyBackend::WaylandPortal
    );
    assert_eq!(resolve_backend(Some("x11"), false), HotkeyBackend::X11);
}

#[test]
fn push_to_talk_press_and_release_maps_to_mic_mute_commands() {
    let bindings = HotkeyBindings {
        mute_main: "Ctrl+Shift+M".to_string(),
        mute_mic: "Ctrl+Shift+N".to_string(),
        push_to_talk: "Alt+V".to_string(),
        toggle_window: "Ctrl+Shift+V".to_string(),
    };
    let state = HotkeyState {
        main_muted: false,
        mic_muted: true,
    };

    let pressed =
        commands_for_hotkey_event(&HotkeyEvent::Pressed("alt+v".to_string()), &bindings, state);
    assert_eq!(pressed, vec![CoreCommand::SetMute(Channel::Mic, false)]);

    let released = commands_for_hotkey_event(
        &HotkeyEvent::Released("alt+v".to_string()),
        &bindings,
        state,
    );
    assert_eq!(released, vec![CoreCommand::SetMute(Channel::Mic, true)]);
}

#[test]
fn toggle_window_hotkey_emits_toggle_command() {
    let bindings = HotkeyBindings {
        mute_main: "Ctrl+Shift+M".to_string(),
        mute_mic: "Ctrl+Shift+N".to_string(),
        push_to_talk: String::new(),
        toggle_window: "Ctrl+Shift+V".to_string(),
    };
    let state = HotkeyState {
        main_muted: false,
        mic_muted: false,
    };

    let cmds = commands_for_hotkey_event(
        &HotkeyEvent::Pressed("ctrl+shift+v".to_string()),
        &bindings,
        state,
    );
    assert_eq!(cmds, vec![CoreCommand::ToggleWindow]);
}

#[test]
fn build_adapter_uses_portal_first_policy() {
    let adapter = build_adapter(Some("x11"), true);
    assert_eq!(adapter.backend(), HotkeyBackend::WaylandPortal);

    let adapter = build_adapter(Some("x11"), false);
    assert_eq!(adapter.backend(), HotkeyBackend::X11);
}

#[test]
fn adapter_events_map_to_push_to_talk_commands() {
    let mut adapter = WaylandPortalAdapter::new();
    adapter.enqueue_for_test(HotkeyEvent::Pressed("alt+v".to_string()));
    adapter.enqueue_for_test(HotkeyEvent::Released("alt+v".to_string()));

    let bindings = HotkeyBindings {
        mute_main: "ctrl+shift+m".to_string(),
        mute_mic: "ctrl+shift+n".to_string(),
        push_to_talk: "alt+v".to_string(),
        toggle_window: "ctrl+shift+v".to_string(),
    };
    let state = HotkeyState {
        main_muted: false,
        mic_muted: true,
    };

    let pressed = collect_adapter_commands(
        &mut adapter as &mut dyn venturi::core::hotkeys::HotkeyAdapter,
        &bindings,
        state,
    );
    assert_eq!(pressed, vec![CoreCommand::SetMute(Channel::Mic, false)]);

    let released = collect_adapter_commands(
        &mut adapter as &mut dyn venturi::core::hotkeys::HotkeyAdapter,
        &bindings,
        state,
    );
    assert_eq!(released, vec![CoreCommand::SetMute(Channel::Mic, true)]);
}
