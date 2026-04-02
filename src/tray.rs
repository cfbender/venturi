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
#[derive(Clone, Debug)]
struct TrayIconSelection {
    theme_root: String,
    icon_name: &'static str,
    icon_path: std::path::PathBuf,
}

#[cfg(target_os = "linux")]
const TRAY_ICON_CANDIDATES: [(&str, &str); 1] = [(TRAY_ICON_NAME, TRAY_ICON_RELATIVE_PATH)];

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
fn resolve_installed_tray_icon<I>(roots: I) -> Option<TrayIconSelection>
where
    I: IntoIterator<Item = String>,
{
    for root in roots {
        for (icon_name, icon_relative_path) in TRAY_ICON_CANDIDATES {
            let icon_path = std::path::PathBuf::from(&root).join(icon_relative_path);
            if icon_path.exists() {
                return Some(TrayIconSelection {
                    theme_root: root,
                    icon_name,
                    icon_path,
                });
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn installed_tray_icon() -> Option<TrayIconSelection> {
    resolve_installed_tray_icon(icon_theme_roots())
}

#[cfg(target_os = "linux")]
fn resolve_dev_tray_icon() -> Option<(&'static str, std::path::PathBuf)> {
    let dev_data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data");

    for (icon_name, _) in TRAY_ICON_CANDIDATES {
        let dev_icon = dev_data_dir.join(format!("{icon_name}.svg"));
        if dev_icon.exists() {
            return Some((icon_name, dev_icon));
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn installed_icon_theme_root() -> Option<String> {
    installed_tray_icon().map(|icon| icon.theme_root)
}

#[cfg(target_os = "linux")]
fn resolved_tray_icon_name() -> &'static str {
    installed_tray_icon()
        .map(|icon| icon.icon_name)
        .or_else(|| resolve_dev_tray_icon().map(|(icon_name, _)| icon_name))
        .unwrap_or(TRAY_ICON_NAME)
}

#[cfg(target_os = "linux")]
fn tray_icon_file_path() -> Option<std::path::PathBuf> {
    if let Some(icon) = installed_tray_icon() {
        return Some(icon.icon_path);
    }

    resolve_dev_tray_icon().map(|(_, icon_path)| icon_path)
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
        resolved_tray_icon_name().to_string()
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
    use std::fs;

    use crossbeam_channel::unbounded;
    use tempfile::tempdir;

    use super::{resolve_installed_tray_icon, VenturiTray, TRAY_ICON_NAME};

    #[test]
    fn tray_reports_venturi_icon_name() {
        let (tx, _rx) = unbounded();
        let tray = VenturiTray { command_tx: tx };

        let icon_name = <VenturiTray as ksni::Tray>::icon_name(&tray);
        assert_eq!(icon_name, TRAY_ICON_NAME);
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

    #[test]
    fn prefers_regular_icon_when_symbolic_also_available() {
        let temp = tempdir().expect("tempdir");
        let apps_dir = temp.path().join("hicolor/scalable/apps");
        fs::create_dir_all(&apps_dir).expect("create apps dir");

        let base_icon_path = apps_dir.join(format!("{TRAY_ICON_NAME}.svg"));
        fs::write(&base_icon_path, "<svg/>").expect("write base icon");

        let symbolic_icon_path = apps_dir.join("org.venturi.Venturi-symbolic.svg");
        fs::write(&symbolic_icon_path, "<svg/>").expect("write symbolic icon");

        let selection = resolve_installed_tray_icon([temp.path().display().to_string()])
            .expect("icon selection should resolve");

        assert_eq!(selection.icon_name, TRAY_ICON_NAME);
    }
}
