use crate::{AppError, DeviceEntry, DeviceKind};

pub trait DeviceService {
    fn list_devices(&self, kind: DeviceKind) -> Result<Vec<DeviceEntry>, AppError>;
}
