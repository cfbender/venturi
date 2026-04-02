use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::readiness::ReadinessBarrier;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    Ready,
    ShutdownRequested,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeSupervisorError {
    #[error("runtime supervisor failed to emit ready event")]
    ReadyEventSendFailed,
}

#[derive(Clone, Debug)]
pub struct RuntimeSupervisor {
    readiness: ReadinessBarrier,
    events_tx: broadcast::Sender<RuntimeEvent>,
    _shutdown: CancellationToken,
}

impl RuntimeSupervisor {
    pub fn new_for_test() -> Self {
        let (events_tx, _) = broadcast::channel(8);
        Self {
            readiness: ReadinessBarrier::new(),
            events_tx,
            _shutdown: CancellationToken::new(),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events_tx.subscribe()
    }

    pub async fn start(&self) -> Result<(), RuntimeSupervisorError> {
        self.readiness.mark_ready();
        self.events_tx
            .send(RuntimeEvent::Ready)
            .map_err(|_| RuntimeSupervisorError::ReadyEventSendFailed)?;
        Ok(())
    }
}
