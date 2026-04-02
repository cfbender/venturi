use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::watch;
use venturi_application::{AppError, Channel, MeterService, MeterSnapshot};

type SetEnabledFn = dyn Fn(bool) -> Result<(), AppError> + Send + Sync;

#[derive(Clone)]
pub struct PipewireMeterAdapter {
    set_enabled: Arc<SetEnabledFn>,
    levels_tx: watch::Sender<MeterSnapshot>,
}

impl PipewireMeterAdapter {
    pub fn new<F>(set_enabled: F) -> Self
    where
        F: Fn(bool) -> Result<(), AppError> + Send + Sync + 'static,
    {
        let (levels_tx, _) = watch::channel(MeterSnapshot {
            channel: Channel::Main,
            level: 0.0,
            peak: 0.0,
        });
        Self {
            set_enabled: Arc::new(set_enabled),
            levels_tx,
        }
    }

    pub fn publish_level(&self, snapshot: MeterSnapshot) {
        let _ = self.levels_tx.send(snapshot);
    }
}

impl Default for PipewireMeterAdapter {
    fn default() -> Self {
        Self::new(|_| Ok(()))
    }
}

#[async_trait]
impl MeterService for PipewireMeterAdapter {
    async fn set_enabled(&self, enabled: bool) -> Result<(), AppError> {
        (self.set_enabled)(enabled)
    }

    fn subscribe_levels(&self) -> watch::Receiver<MeterSnapshot> {
        self.levels_tx.subscribe()
    }
}
