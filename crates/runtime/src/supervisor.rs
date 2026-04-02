use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::composition::RuntimeComposition;
use crate::readiness::ReadinessBarrier;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    Ready,
    ShutdownRequested,
}

#[derive(Clone, Debug)]
pub struct RuntimeSupervisor {
    readiness: ReadinessBarrier,
    events_tx: broadcast::Sender<RuntimeEvent>,
    composition: Option<RuntimeComposition>,
    _shutdown: CancellationToken,
}

impl RuntimeSupervisor {
    pub fn new_for_test() -> Self {
        let (events_tx, _) = broadcast::channel(8);
        Self {
            readiness: ReadinessBarrier::new(),
            events_tx,
            composition: None,
            _shutdown: CancellationToken::new(),
        }
    }

    pub fn with_composition(composition: RuntimeComposition) -> Self {
        let mut supervisor = Self::new_for_test();
        supervisor.composition = Some(composition);
        supervisor
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events_tx.subscribe()
    }

    pub fn start(&self) {
        self.readiness.mark_ready();
        let _ = self.events_tx.send(RuntimeEvent::Ready);
    }

    pub fn composition(&self) -> Option<&RuntimeComposition> {
        self.composition.as_ref()
    }
}
