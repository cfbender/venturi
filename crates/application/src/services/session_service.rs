use crate::{AppError, Channel};

pub trait SessionService {
    fn active_channel(&self) -> Result<Channel, AppError>;
    fn set_active_channel(&self, channel: Channel) -> Result<(), AppError>;
}
