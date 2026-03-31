use crossbeam_channel::Sender;

use crate::core::messages::CoreCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuAction {
    ShowHide,
    Quit,
}

#[derive(Clone)]
pub struct TrayHandle {
    entries: Vec<TrayMenuAction>,
    command_tx: Sender<CoreCommand>,
    #[cfg(target_os = "linux")]
    _backend: Option<LinuxTrayBackend>,
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
    let backend = LinuxTrayBackend::spawn(command_tx.clone());
    Some(TrayHandle {
        entries: vec![TrayMenuAction::ShowHide, TrayMenuAction::Quit],
        command_tx,
        _backend: backend,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn create_tray(_command_tx: Sender<CoreCommand>) -> Option<TrayHandle> {
    None
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
struct LinuxTrayBackend {
    _handle: ksni::blocking::Handle<VenturiTray>,
}

#[cfg(target_os = "linux")]
impl LinuxTrayBackend {
    fn spawn(command_tx: Sender<CoreCommand>) -> Option<Self> {
        use ksni::blocking::TrayMethods;

        let tray = VenturiTray { command_tx };
        match tray.assume_sni_available(true).spawn() {
            Ok(handle) => Some(Self { _handle: handle }),
            Err(error) => {
                eprintln!("venturi tray backend unavailable: {error}");
                None
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct VenturiTray {
    command_tx: Sender<CoreCommand>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for VenturiTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "org.venturi.Venturi".to_string()
    }

    fn title(&self) -> String {
        "Venturi".to_string()
    }

    fn icon_name(&self) -> String {
        "org.venturi.Venturi".to_string()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem {
                label: "Show/Hide".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.command_tx.send(CoreCommand::ToggleWindow);
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.command_tx.send(CoreCommand::Shutdown);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use crossbeam_channel::unbounded;

    use super::VenturiTray;

    #[test]
    fn tray_reports_venturi_icon_name() {
        let (tx, _rx) = unbounded();
        let tray = VenturiTray { command_tx: tx };
        assert_eq!(
            <VenturiTray as ksni::Tray>::icon_name(&tray),
            "org.venturi.Venturi"
        );
    }
}
