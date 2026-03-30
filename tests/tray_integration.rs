use crossbeam_channel::unbounded;
use venturi::core::messages::CoreCommand;
use venturi::tray::{TrayMenuAction, create_tray};

#[test]
fn tray_has_expected_linux_menu_actions() {
    let (tx, _rx) = unbounded();
    let tray = create_tray(tx).expect("tray should be available on linux");
    assert_eq!(
        tray.entries(),
        &[TrayMenuAction::ShowHide, TrayMenuAction::Quit]
    );
}

#[test]
fn tray_show_hide_dispatches_toggle_window_command() {
    let (tx, rx) = unbounded();
    let tray = create_tray(tx).expect("tray should be available on linux");

    tray.activate(TrayMenuAction::ShowHide)
        .expect("dispatch show/hide");
    assert_eq!(
        rx.recv().expect("receive command"),
        CoreCommand::ToggleWindow
    );
}

#[test]
fn tray_quit_dispatches_shutdown_command() {
    let (tx, rx) = unbounded();
    let tray = create_tray(tx).expect("tray should be available on linux");

    tray.activate(TrayMenuAction::Quit).expect("dispatch quit");
    assert_eq!(rx.recv().expect("receive command"), CoreCommand::Shutdown);
}
