use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeUiEvent {
    Ready,
    ToggleWindowRequested,
    ShutdownRequested,
}

type DispatchFn = dyn Fn(RuntimeUiEvent) + Send + Sync + 'static;

#[derive(Clone)]
pub struct WindowBridge {
    dispatch: Arc<DispatchFn>,
}

impl WindowBridge {
    pub fn new_for_test(dispatch: Box<DispatchFn>) -> Self {
        Self {
            dispatch: Arc::from(dispatch),
        }
    }

    pub fn on_event(&self, event: RuntimeUiEvent) {
        (self.dispatch)(event);
    }
}

impl From<venturi_runtime::RuntimeEvent> for RuntimeUiEvent {
    fn from(event: venturi_runtime::RuntimeEvent) -> Self {
        match event {
            venturi_runtime::RuntimeEvent::Ready => Self::Ready,
            venturi_runtime::RuntimeEvent::ShutdownRequested => Self::ShutdownRequested,
        }
    }
}
