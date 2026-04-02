use async_trait::async_trait;

use crate::{AppError, Channel};

#[async_trait]
pub trait RoutingService: Send + Sync {
    async fn set_volume(&self, channel: Channel, value: f32) -> Result<(), AppError>;
}
