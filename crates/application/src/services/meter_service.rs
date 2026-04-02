use crate::{AppError, MeterSnapshot};

pub trait MeterService {
    fn snapshot(&self) -> Result<MeterSnapshot, AppError>;
}
