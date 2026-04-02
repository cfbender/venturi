use async_trait::async_trait;

use crate::AppError;

#[async_trait]
pub trait SessionService: Send + Sync {
    async fn save(&self) -> Result<(), AppError>;
    async fn restore(&self) -> Result<(), AppError>;
}
