use crate::{AppError, RouteCommand};

pub trait RoutingService {
    fn apply(&self, command: RouteCommand) -> Result<(), AppError>;
}
