use std::cell::RefCell;

use venturi_application::{AppError, RouteCommand, RoutingService, StableDeviceId};

#[derive(Default)]
struct RecordingRouter {
    commands: RefCell<Vec<RouteCommand>>,
}

impl RoutingService for RecordingRouter {
    fn apply(&self, command: RouteCommand) -> Result<(), AppError> {
        self.commands.borrow_mut().push(command);
        Ok(())
    }
}

fn dispatch(router: &dyn RoutingService, command: RouteCommand) -> Result<(), AppError> {
    router.apply(command)
}

#[test]
fn routing_service_trait_object_accepts_route_commands() {
    let router = RecordingRouter::default();
    let command = RouteCommand::Connect {
        source: StableDeviceId("stream-main".into()),
        target: StableDeviceId("sink-main".into()),
    };

    let result = dispatch(&router, command.clone());

    assert!(result.is_ok());
    assert_eq!(router.commands.into_inner(), vec![command]);
}
