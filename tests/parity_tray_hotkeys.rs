use crossbeam_channel::unbounded;
use venturi::core::hotkeys::{HotkeyBindings, HotkeyEvent, HotkeyState, commands_for_hotkey_event};
use venturi::core::messages::CoreCommand;
use venturi::tray::{TrayMenuAction, create_tray};
use venturi_runtime::test_harness;

#[test]
fn tray_and_hotkeys_keep_lifecycle_snapshot_in_sync() {
    let harness = test_harness();
    let (tx, rx) = unbounded();
    let tray = create_tray(tx).expect("tray should be available on linux");

    tray.activate(TrayMenuAction::ShowHide)
        .expect("dispatch show/hide");
    tray.activate(TrayMenuAction::Quit).expect("dispatch quit");

    let tray_toggle = rx.recv().expect("receive tray toggle command");
    let tray_shutdown = rx.recv().expect("receive tray shutdown command");

    let bindings = HotkeyBindings {
        mute_main: String::new(),
        mute_mic: String::new(),
        push_to_talk: String::new(),
        toggle_window: "ctrl+shift+v".to_string(),
    };

    let hotkey_commands = commands_for_hotkey_event(
        &HotkeyEvent::Pressed("ctrl+shift+v".to_string()),
        &bindings,
        HotkeyState {
            main_muted: false,
            mic_muted: false,
        },
    );

    let mut all_commands = vec![tray_toggle, tray_shutdown];
    all_commands.extend(hotkey_commands);

    let toggle_count = all_commands
        .iter()
        .filter(|command| matches!(command, CoreCommand::ToggleWindow))
        .count();
    let shutdown_count = all_commands
        .iter()
        .filter(|command| matches!(command, CoreCommand::Shutdown))
        .count();

    assert_eq!(toggle_count, 2);
    assert_eq!(shutdown_count, 1);

    for command in all_commands {
        match command {
            CoreCommand::ToggleWindow => harness.request_toggle_window(),
            CoreCommand::Shutdown => harness.request_shutdown(),
            _ => {}
        }
    }

    let view = harness.snapshot().view();
    assert!(view.toggle_window_requested());
    assert!(view.shutdown_requested());
}
