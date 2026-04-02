use crate::AppError;

pub trait SoundboardService {
    fn play_clip(&self, clip_id: &str) -> Result<(), AppError>;
    fn stop_all(&self) -> Result<(), AppError>;
}
