use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::config::persistence::{Paths, load_config};
use crate::core::messages::{CoreCommand, CoreEvent};
use crate::core::pipewire_manager::PipeWireManager;
use crate::gui::window::MainWindow;
use crate::tray::{TrayChannels, create_tray};

/// How long to wait for the core thread after the UI has gone away.
///
/// The core normally exits within milliseconds of `Shutdown`. If it is stuck
/// in a PipeWire call we still want the process to quit, so after this we give
/// up on a clean join, reap our helper processes and return.
const CORE_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

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

        let paths = Paths::resolve();
        let config = load_config(&paths);
        let _tray = if should_create_tray(config.general.show_tray_icon) {
            create_tray(TrayChannels {
                command_tx: bootstrap.command_tx.clone(),
                event_tx: bootstrap.event_tx.clone(),
            })
        } else {
            None
        };

        if daemon {
            wait_for_shutdown(&bootstrap.event_rx)?;
        } else {
            self.gui_launcher
                .launch(bootstrap.command_tx.clone(), bootstrap.event_rx.clone())?;
        }

        // Best effort: the core may already have processed a tray-initiated
        // Shutdown and dropped its receiver, which is fine.
        let _ = bootstrap.command_tx.send(CoreCommand::Shutdown);

        if !manager.join_with_grace(CORE_SHUTDOWN_GRACE) {
            tracing::warn!(
                "core did not shut down within {}s; exiting anyway",
                CORE_SHUTDOWN_GRACE.as_secs()
            );
            kill_child_processes();
        }
        Ok(())
    }
}

/// Terminate helper processes (`pw-dump --monitor`, `pw-record` meters, a hung
/// `pactl`) that would otherwise outlive us when the core thread cannot clean
/// up itself.
fn kill_child_processes() {
    let _ = std::process::Command::new("pkill")
        .args(["-TERM", "-P", &std::process::id().to_string()])
        .status();
}

pub fn run_app(daemon: bool) -> Result<(), String> {
    run_app_with_launcher(daemon, crate::gui::window::GtkGuiLauncher)
}

pub fn run_app_with_launcher<G: GuiLauncher>(daemon: bool, gui_launcher: G) -> Result<(), String> {
    let bootstrap = AppBootstrap::new();
    let runner = AppRunner::new(gui_launcher);
    runner.run(daemon, bootstrap)
}

fn wait_for_shutdown(event_rx: &Receiver<CoreEvent>) -> Result<(), String> {
    loop {
        match event_rx.recv() {
            Ok(CoreEvent::ShutdownRequested) => return Ok(()),
            Ok(_) => continue,
            Err(err) => {
                return Err(format!(
                    "core event channel closed before shutdown request: {err}"
                ));
            }
        }
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
        match event_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(event) if predicate(&event) => return Ok(()),
            _ => (),
        }
    }

    Err(timeout_message.to_string())
}

pub fn pump_event(window: &mut MainWindow, event: CoreEvent) {
    window.apply_core_event(&event);
}

pub fn should_create_tray(show_tray_icon: bool) -> bool {
    show_tray_icon
}
