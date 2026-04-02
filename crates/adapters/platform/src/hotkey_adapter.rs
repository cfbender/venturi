#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyAction {
    Pressed { chord: String },
    Released { chord: String },
}
