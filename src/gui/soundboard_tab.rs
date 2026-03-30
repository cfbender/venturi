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
