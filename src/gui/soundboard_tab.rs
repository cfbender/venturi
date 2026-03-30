#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundboardPad {
    pub id: u32,
    pub name: String,
    pub icon: String,
    pub file: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PadAction {
    Rename(String),
    SetIcon(String),
    ChangeFile(String),
    Remove,
}

#[derive(Debug, Clone, Default)]
pub struct SoundboardTab {
    pub pads: Vec<SoundboardPad>,
}

impl SoundboardTab {
    pub fn ensure_empty_slots(&mut self, target: usize) {
        let start = self.pads.len() as u32;
        for i in self.pads.len()..target {
            self.pads.push(SoundboardPad {
                id: start + (i - self.pads.len()) as u32,
                name: "Add".to_string(),
                icon: "➕".to_string(),
                file: None,
                active: false,
            });
        }
    }

    pub fn apply_action(&mut self, pad_id: u32, action: PadAction) {
        if let Some(pad) = self.pads.iter_mut().find(|p| p.id == pad_id) {
            match action {
                PadAction::Rename(name) => pad.name = name,
                PadAction::SetIcon(icon) => pad.icon = icon,
                PadAction::ChangeFile(path) => pad.file = Some(path),
                PadAction::Remove => {
                    pad.name = "Add".to_string();
                    pad.icon = "➕".to_string();
                    pad.file = None;
                    pad.active = false;
                }
            }
        }
    }

    pub fn toggle_active(&mut self, pad_id: u32) -> bool {
        if let Some(pad) = self.pads.iter_mut().find(|p| p.id == pad_id) {
            pad.active = !pad.active;
            return pad.active;
        }
        false
    }
}

pub fn build_soundboard_widget(
    model: std::sync::Arc<std::sync::Mutex<SoundboardTab>>,
    command_tx: crossbeam_channel::Sender<crate::core::messages::CoreCommand>,
) -> gtk::Box {
    use gtk::prelude::*;

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(8);
    grid.set_row_spacing(8);

    {
        let mut state = model.lock().expect("soundboard lock");
        state.ensure_empty_slots(10);
    }

    let pads = model.lock().expect("soundboard lock").pads.clone();
    for (index, pad) in pads.into_iter().enumerate() {
        let button = gtk::Button::with_label(&format!("{} {}", pad.icon, pad.name));
        if pad.active {
            button.add_css_class("suggested-action");
        }

        let tx = command_tx.clone();
        let model = model.clone();
        button.connect_clicked(move |_| {
            if let Ok(mut state) = model.lock() {
                let active = state.toggle_active(pad.id);
                let _ = if active {
                    tx.send(crate::core::messages::CoreCommand::PlaySound(pad.id))
                } else {
                    tx.send(crate::core::messages::CoreCommand::StopSound(pad.id))
                };
            }
        });

        grid.attach(&button, (index % 5) as i32, (index / 5) as i32, 1, 1);
    }

    root.append(&grid);
    root
}
