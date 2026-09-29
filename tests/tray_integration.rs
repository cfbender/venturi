use crossbeam_channel::{Receiver, unbounded};
use venturi::core::messages::{CoreCommand, CoreEvent};
use venturi::tray::{TrayChannels, TrayMenuAction, create_tray};

fn tray_channels() -> (TrayChannels, Receiver<CoreCommand>, Receiver<CoreEvent>) {
    let (command_tx, command_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();
    (
        TrayChannels {
            command_tx,
            event_tx,
        },
        command_rx,
        event_rx,
    )
}

#[test]
fn tray_has_expected_linux_menu_actions() {
    let (channels, _command_rx, _event_rx) = tray_channels();
    let tray = create_tray(channels).expect("tray should be available on linux");
    assert_eq!(
        tray.entries(),
        &[TrayMenuAction::ShowHide, TrayMenuAction::Quit]
    );
}

#[test]
fn tray_show_hide_dispatches_toggle_window_command_only() {
    let (channels, command_rx, event_rx) = tray_channels();
    let tray = create_tray(channels).expect("tray should be available on linux");

    tray.activate(TrayMenuAction::ShowHide)
        .expect("dispatch show/hide");
    assert_eq!(
        command_rx.recv().expect("receive command"),
        CoreCommand::ToggleWindow
    );
    assert!(
        event_rx.try_recv().is_err(),
        "show/hide must not ask the UI to shut down"
    );
}

#[test]
fn tray_quit_dispatches_shutdown_to_core_and_ui() {
    let (channels, command_rx, event_rx) = tray_channels();
    let tray = create_tray(channels).expect("tray should be available on linux");

    tray.activate(TrayMenuAction::Quit).expect("dispatch quit");
    assert_eq!(
        command_rx.recv().expect("receive command"),
        CoreCommand::Shutdown
    );
    assert!(matches!(
        event_rx.recv().expect("receive event"),
        CoreEvent::ShutdownRequested
    ));
}

#[test]
fn tray_quit_still_reaches_ui_when_core_is_gone() {
    let (channels, command_rx, event_rx) = tray_channels();
    let tray = create_tray(channels).expect("tray should be available on linux");
    drop(command_rx);

    // The core side is gone (or, in practice, wedged); the UI must still learn
    // about the quit so the process can exit.
    let _ = tray.activate(TrayMenuAction::Quit);
    assert!(matches!(
        event_rx.try_recv().expect("receive event"),
        CoreEvent::ShutdownRequested
    ));
}
