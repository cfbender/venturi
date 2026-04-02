use venturi_platform_adapter::{TrayAction, TrayCommand, TrayController};

#[test]
fn tray_actions_map_to_core_commands() {
    let mut dispatched = Vec::new();
    let mut controller = TrayController::new(|command| {
        dispatched.push(command);
    });

    controller.dispatch(TrayAction::ShowHide);
    controller.dispatch(TrayAction::Quit);

    assert_eq!(
        dispatched,
        vec![TrayCommand::ToggleWindow, TrayCommand::Shutdown]
    );
}
