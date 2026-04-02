use crate::{RuntimeUiEvent, WindowBridge};

#[derive(Clone)]
pub struct GtkLauncher {
    bridge: WindowBridge,
}

impl GtkLauncher {
    pub fn new(bridge: WindowBridge) -> Self {
        Self { bridge }
    }

    pub fn notify_ready(&self) {
        self.bridge.on_event(RuntimeUiEvent::Ready);
    }
}
