use std::sync::Arc;

use async_trait::async_trait;
use venturi_application::{AppError, SoundboardService};

type PlayFn = dyn Fn(u32, String) -> Result<(), AppError> + Send + Sync;
type StopFn = dyn Fn(u32) -> Result<(), AppError> + Send + Sync;

#[derive(Clone)]
pub struct PipewireSoundboardAdapter {
    play: Arc<PlayFn>,
    preview: Arc<PlayFn>,
    stop: Arc<StopFn>,
}

impl PipewireSoundboardAdapter {
    pub fn new<FP, FV, FS>(play: FP, preview: FV, stop: FS) -> Self
    where
        FP: Fn(u32, String) -> Result<(), AppError> + Send + Sync + 'static,
        FV: Fn(u32, String) -> Result<(), AppError> + Send + Sync + 'static,
        FS: Fn(u32) -> Result<(), AppError> + Send + Sync + 'static,
    {
        Self {
            play: Arc::new(play),
            preview: Arc::new(preview),
            stop: Arc::new(stop),
        }
    }
}

impl Default for PipewireSoundboardAdapter {
    fn default() -> Self {
        Self::new(|_, _| Ok(()), |_, _| Ok(()), |_| Ok(()))
    }
}

#[async_trait]
impl SoundboardService for PipewireSoundboardAdapter {
    async fn play(&self, pad_id: u32, file: String) -> Result<(), AppError> {
        (self.play)(pad_id, file)
    }

    async fn preview(&self, pad_id: u32, file: String) -> Result<(), AppError> {
        (self.preview)(pad_id, file)
    }

    async fn stop(&self, pad_id: u32) -> Result<(), AppError> {
        (self.stop)(pad_id)
    }
}
