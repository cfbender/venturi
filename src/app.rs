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
    fn launch(&self) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy)]
pub struct NoopGuiLauncher;

impl GuiLauncher for NoopGuiLauncher {
    fn launch(&self) -> Result<(), String> {
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

        let first_event = bootstrap
            .event_rx
            .recv_timeout(std::time::Duration::from_millis(200))
            .map_err(|_| "core did not become ready".to_string())?;
        if first_event != CoreEvent::Ready {
            return Err("unexpected first core event".to_string());
        }

        bootstrap
            .command_tx
            .send(CoreCommand::Ping)
            .map_err(|e| e.to_string())?;
        let pong = bootstrap
            .event_rx
            .recv_timeout(std::time::Duration::from_millis(200))
            .map_err(|_| "core did not answer ping".to_string())?;
        if pong != CoreEvent::Pong {
            return Err("unexpected ping response".to_string());
        }

        if !daemon {
            self.gui_launcher.launch()?;
        }

        bootstrap
            .command_tx
            .send(CoreCommand::Shutdown)
            .map_err(|e| e.to_string())?;
        let _ = manager.join();
        Ok(())
    }
}

pub fn pump_event(window: &mut MainWindow, event: CoreEvent) {
    window.apply_core_event(&event);
}
