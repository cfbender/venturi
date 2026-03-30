#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualNodeSpec {
    pub name: String,
    pub media_class: String,
    pub factory_name: String,
    pub autoconnect: bool,
}

impl VirtualNodeSpec {
    pub fn new(name: impl Into<String>, media_class: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            media_class: media_class.into(),
            factory_name: "support.null-audio-sink".to_string(),
            autoconnect: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpec {
    pub from: String,
    pub to: String,
    pub passive: bool,
}

pub fn default_nodes() -> Vec<VirtualNodeSpec> {
    vec![
        VirtualNodeSpec::new("Venturi-Game", "Audio/Sink"),
        VirtualNodeSpec::new("Venturi-Media", "Audio/Sink"),
        VirtualNodeSpec::new("Venturi-Chat", "Audio/Sink"),
        VirtualNodeSpec::new("Venturi-Aux", "Audio/Sink"),
        VirtualNodeSpec::new("Venturi-Mic", "Audio/Sink"),
        VirtualNodeSpec::new("Venturi-Sound", "Audio/Sink"),
        VirtualNodeSpec::new("Venturi-Output", "Audio/Sink"),
        VirtualNodeSpec::new("Venturi-VirtualMic", "Audio/Source/Virtual"),
    ]
}

pub fn stale_venturi_nodes<'a, I>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    names
        .into_iter()
        .filter(|name| name.starts_with("Venturi-"))
        .map(ToOwned::to_owned)
        .collect()
}

pub fn default_mix_links() -> Vec<LinkSpec> {
    vec![
        LinkSpec {
            from: "Venturi-Game.monitor".to_string(),
            to: "Venturi-Output.input".to_string(),
            passive: true,
        },
        LinkSpec {
            from: "Venturi-Media.monitor".to_string(),
            to: "Venturi-Output.input".to_string(),
            passive: true,
        },
        LinkSpec {
            from: "Venturi-Chat.monitor".to_string(),
            to: "Venturi-Output.input".to_string(),
            passive: true,
        },
        LinkSpec {
            from: "Venturi-Aux.monitor".to_string(),
            to: "Venturi-Output.input".to_string(),
            passive: true,
        },
        LinkSpec {
            from: "Venturi-Mic.monitor".to_string(),
            to: "Venturi-VirtualMic.input".to_string(),
            passive: true,
        },
        LinkSpec {
            from: "Venturi-Sound.monitor".to_string(),
            to: "Venturi-VirtualMic.input".to_string(),
            passive: true,
        },
    ]
}
