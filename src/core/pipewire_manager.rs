use crossbeam_channel::{Receiver, Sender};

use crate::core::messages::{CoreCommand, CoreEvent};

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
