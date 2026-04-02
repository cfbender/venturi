use async_trait::async_trait;

use crate::AppError;

#[async_trait]
pub trait SoundboardService: Send + Sync {
    async fn play(&self, pad_id: u32, file: String) -> Result<(), AppError>;
    async fn preview(&self, pad_id: u32, file: String) -> Result<(), AppError>;
    async fn stop(&self, pad_id: u32) -> Result<(), AppError>;
}
