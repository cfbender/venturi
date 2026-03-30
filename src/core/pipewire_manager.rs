use crossbeam_channel::{Receiver, Sender};
use std::time::Duration;

use crate::core::messages::{CoreCommand, CoreEvent};

pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub fn reconnect_delay() -> Duration {
    RECONNECT_DELAY
}

pub fn fallback_to_default_device() -> &'static str {
    "Default"
}

pub struct PipeWireManager {
    handle: std::thread::JoinHandle<()>,
}

impl PipeWireManager {
    pub fn spawn(command_rx: Receiver<CoreCommand>, event_tx: Sender<CoreEvent>) -> Self {
        let handle = std::thread::spawn(move || {
            let _ = event_tx.send(CoreEvent::Ready);
            for command in &command_rx {
                match command {
                    CoreCommand::Ping => {
                        let _ = event_tx.send(CoreEvent::Pong);
                    }
                    CoreCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        Self { handle }
    }

    pub fn join(self) -> std::thread::Result<()> {
        self.handle.join()
    }
}
