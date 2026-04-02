use std::sync::Arc;

use async_trait::async_trait;
use venturi_application::{AppError, Channel, RoutingService};

type SetVolumeFn = dyn Fn(Channel, f32) -> Result<(), AppError> + Send + Sync;

#[derive(Clone)]
pub struct PipewireRoutingAdapter {
    set_volume: Arc<SetVolumeFn>,
}

impl PipewireRoutingAdapter {
    pub fn new<F>(set_volume: F) -> Self
    where
        F: Fn(Channel, f32) -> Result<(), AppError> + Send + Sync + 'static,
    {
        Self {
            set_volume: Arc::new(set_volume),
        }
    }
}

impl Default for PipewireRoutingAdapter {
    fn default() -> Self {
        Self::new(|_, _| Ok(()))
    }
}

#[async_trait]
impl RoutingService for PipewireRoutingAdapter {
    async fn set_volume(&self, channel: Channel, value: f32) -> Result<(), AppError> {
        (self.set_volume)(channel, value)
    }
}
