#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceKind {
    Output,
    Input,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableDeviceId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub kind: DeviceKind,
    pub id: StableDeviceId,
    pub label: String,
}
