use std::sync::Arc;

use async_trait::async_trait;
use venturi_application::{AppError, DeviceEntry, DeviceService, StableDeviceId};

type ListDevicesFn = dyn Fn() -> Result<Vec<DeviceEntry>, AppError> + Send + Sync;
type SelectDeviceFn = dyn Fn(Option<StableDeviceId>) -> Result<(), AppError> + Send + Sync;

#[derive(Clone)]
pub struct PipewireDeviceAdapter {
    list_devices: Arc<ListDevicesFn>,
    select_output: Arc<SelectDeviceFn>,
    select_input: Arc<SelectDeviceFn>,
}

impl PipewireDeviceAdapter {
    pub fn new<FL, FO, FI>(list_devices: FL, select_output: FO, select_input: FI) -> Self
    where
        FL: Fn() -> Result<Vec<DeviceEntry>, AppError> + Send + Sync + 'static,
        FO: Fn(Option<StableDeviceId>) -> Result<(), AppError> + Send + Sync + 'static,
        FI: Fn(Option<StableDeviceId>) -> Result<(), AppError> + Send + Sync + 'static,
    {
        Self {
            list_devices: Arc::new(list_devices),
            select_output: Arc::new(select_output),
            select_input: Arc::new(select_input),
        }
    }
}

impl Default for PipewireDeviceAdapter {
    fn default() -> Self {
        Self::new(|| Ok(Vec::new()), |_| Ok(()), |_| Ok(()))
    }
}

#[async_trait]
impl DeviceService for PipewireDeviceAdapter {
    async fn list_devices(&self) -> Result<Vec<DeviceEntry>, AppError> {
        (self.list_devices)()
    }

    async fn select_output(&self, output: Option<StableDeviceId>) -> Result<(), AppError> {
        (self.select_output)(output)
    }

    async fn select_input(&self, input: Option<StableDeviceId>) -> Result<(), AppError> {
        (self.select_input)(input)
    }
}
