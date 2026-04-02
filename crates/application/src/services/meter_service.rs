use async_trait::async_trait;
use tokio::sync::watch;

use crate::{AppError, MeterSnapshot};

#[async_trait]
pub trait MeterService: Send + Sync {
    async fn set_enabled(&self, enabled: bool) -> Result<(), AppError>;
    fn subscribe_levels(&self) -> watch::Receiver<MeterSnapshot>;
}
