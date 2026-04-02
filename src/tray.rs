use crossbeam_channel::Sender;

use crate::core::messages::CoreCommand;

#[cfg(target_os = "linux")]
const TRAY_ICON_NAME: &str = "org.venturi.Venturi";
#[cfg(target_os = "linux")]
const TRAY_ICON_RELATIVE_PATH: &str = "hicolor/scalable/apps/org.venturi.Venturi.svg";

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
fn icon_theme_roots() -> Vec<String> {
    let xdg_data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.local/share")
    });

    vec![
        format!("{xdg_data_home}/icons"),
        "/usr/share/icons".to_string(),
        "/usr/local/share/icons".to_string(),
    ]
}

#[cfg(target_os = "linux")]
fn installed_icon_theme_root() -> Option<String> {
    for root in icon_theme_roots() {
        let icon_path = std::path::PathBuf::from(&root).join(TRAY_ICON_RELATIVE_PATH);
        if icon_path.exists() {
            return Some(root);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn tray_icon_file_path() -> Option<std::path::PathBuf> {
    if let Some(theme_root) = installed_icon_theme_root() {
        return Some(std::path::PathBuf::from(theme_root).join(TRAY_ICON_RELATIVE_PATH));
    }

    let dev_icon = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(format!("{TRAY_ICON_NAME}.svg"));
    if dev_icon.exists() {
        Some(dev_icon)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn load_tray_icon_pixmap() -> Option<ksni::Icon> {
    use gtk::prelude::{TextureExt, TextureExtManual};

    let icon_file = tray_icon_file_path()?;
    let texture = gtk::gdk::Texture::from_file(&gtk::gio::File::for_path(icon_file)).ok()?;
    let width = texture.width();
    let height = texture.height();
    if width <= 0 || height <= 0 {
        return None;
    }

    let stride = (width as usize).saturating_mul(4);
    let mut data = vec![0u8; stride.saturating_mul(height as usize)];
    texture.download(&mut data, stride);

    // ksni expects ARGB32 data in network byte order.
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }

    Some(ksni::Icon {
        width,
        height,
        data,
    })
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

    fn icon_theme_path(&self) -> String {
        installed_icon_theme_root().unwrap_or_default()
    }

    fn icon_name(&self) -> String {
        TRAY_ICON_NAME.to_string()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static ICON_PIXMAP: std::sync::LazyLock<Option<ksni::Icon>> =
            std::sync::LazyLock::new(load_tray_icon_pixmap);

        ICON_PIXMAP.clone().into_iter().collect()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem {
                label: "Venturi".to_string(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
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

    #[test]
    fn tray_exposes_icon_pixmap_data() {
        let (tx, _rx) = unbounded();
        let tray = VenturiTray { command_tx: tx };

        let pixmaps = <VenturiTray as ksni::Tray>::icon_pixmap(&tray);
        assert!(
            !pixmaps.is_empty(),
            "expected tray icon pixmap to be populated"
        );

        let icon = &pixmaps[0];
        assert!(icon.width > 0 && icon.height > 0);
        assert_eq!(
            icon.data.len(),
            (icon.width as usize) * (icon.height as usize) * 4
        );
    }
}
