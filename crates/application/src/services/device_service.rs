use async_trait::async_trait;

use crate::{AppError, DeviceEntry, StableDeviceId};

#[async_trait]
pub trait DeviceService: Send + Sync {
    async fn list_devices(&self) -> Result<Vec<DeviceEntry>, AppError>;
    async fn select_output(&self, output: Option<StableDeviceId>) -> Result<(), AppError>;
    async fn select_input(&self, input: Option<StableDeviceId>) -> Result<(), AppError>;
}
