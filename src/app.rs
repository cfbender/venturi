use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::core::messages::{CoreCommand, CoreEvent};
use crate::core::pipewire_manager::PipeWireManager;
use crate::gui::window::MainWindow;

#[derive(Debug)]
pub struct AppBootstrap {
    pub command_tx: Sender<CoreCommand>,
    pub command_rx: Receiver<CoreCommand>,
    pub event_tx: Sender<CoreEvent>,
    pub event_rx: Receiver<CoreEvent>,
}

impl AppBootstrap {
    pub fn new() -> Self {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        Self {
            command_tx,
            command_rx,
            event_tx,
            event_rx,
        }
    }

    pub fn spawn_mock_core(&self) -> PipeWireManager {
        PipeWireManager::spawn(self.command_rx.clone(), self.event_tx.clone())
    }
}

impl Default for AppBootstrap {
    fn default() -> Self {
        Self::new()
    }
}

pub trait GuiLauncher {
    fn launch(
        &self,
        command_tx: Sender<CoreCommand>,
        event_rx: Receiver<CoreEvent>,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy)]
pub struct NoopGuiLauncher;

impl GuiLauncher for NoopGuiLauncher {
    fn launch(
        &self,
        _command_tx: Sender<CoreCommand>,
        _event_rx: Receiver<CoreEvent>,
    ) -> Result<(), String> {
        Ok(())
    }
}

pub struct AppRunner<G: GuiLauncher> {
    gui_launcher: G,
}

impl<G: GuiLauncher> AppRunner<G> {
    pub fn new(gui_launcher: G) -> Self {
        Self { gui_launcher }
    }

    pub fn run(&self, daemon: bool, bootstrap: AppBootstrap) -> Result<(), String> {
        let manager = bootstrap.spawn_mock_core();

        wait_for_event(
            &bootstrap.event_rx,
            std::time::Duration::from_secs(1),
            |event| matches!(event, CoreEvent::Ready),
            "core did not become ready",
        )?;

        bootstrap
            .command_tx
            .send(CoreCommand::Ping)
            .map_err(|e| e.to_string())?;
        wait_for_event(
            &bootstrap.event_rx,
            std::time::Duration::from_secs(1),
            |event| matches!(event, CoreEvent::Pong),
            "core did not answer ping",
        )?;

        if !daemon {
            self.gui_launcher
                .launch(bootstrap.command_tx.clone(), bootstrap.event_rx.clone())?;
        }

        bootstrap
            .command_tx
            .send(CoreCommand::Shutdown)
            .map_err(|e| e.to_string())?;
        let _ = manager.join();
        Ok(())
    }
}

fn wait_for_event<F>(
    event_rx: &Receiver<CoreEvent>,
    timeout: std::time::Duration,
    predicate: F,
    timeout_message: &str,
) -> Result<(), String>
where
    F: Fn(&CoreEvent) -> bool,
{
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(event) = event_rx.recv_timeout(std::time::Duration::from_millis(100))
            && predicate(&event)
        {
            return Ok(());
        }
    }

    Err(timeout_message.to_string())
}

pub fn pump_event(window: &mut MainWindow, event: CoreEvent) {
    window.apply_core_event(&event);
}
