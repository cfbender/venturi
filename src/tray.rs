use crossbeam_channel::Sender;

use crate::core::messages::CoreCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuAction {
    ShowHide,
    Quit,
}

#[derive(Debug, Clone)]
pub struct TrayHandle {
    entries: Vec<TrayMenuAction>,
    command_tx: Sender<CoreCommand>,
}

impl TrayHandle {
    pub fn entries(&self) -> &[TrayMenuAction] {
        &self.entries
    }

    pub fn activate(&self, action: TrayMenuAction) -> Result<(), String> {
        let command = match action {
            TrayMenuAction::ShowHide => CoreCommand::ToggleWindow,
            TrayMenuAction::Quit => CoreCommand::Shutdown,
        };
        self.command_tx.send(command).map_err(|err| err.to_string())
    }
}

#[cfg(target_os = "linux")]
pub fn create_tray(command_tx: Sender<CoreCommand>) -> Option<TrayHandle> {
    Some(TrayHandle {
        entries: vec![TrayMenuAction::ShowHide, TrayMenuAction::Quit],
        command_tx,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn create_tray(_command_tx: Sender<CoreCommand>) -> Option<TrayHandle> {
    None
}
