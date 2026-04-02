use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use venturi::app::{AppBootstrap, AppRunner, GuiLauncher, should_create_tray};
use venturi::core::messages::{CoreCommand, CoreEvent};

#[derive(Clone)]
struct CountingGui {
    launches: Arc<AtomicUsize>,
}

impl GuiLauncher for CountingGui {
    fn launch(
        &self,
        _command_tx: Sender<CoreCommand>,
        _event_rx: Receiver<CoreEvent>,
    ) -> Result<(), String> {
        self.launches.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[test]
fn daemon_mode_does_not_launch_gui() {
    let launches = Arc::new(AtomicUsize::new(0));
    let gui = CountingGui {
        launches: launches.clone(),
    };

    let runner = AppRunner::new(gui);
    let bootstrap = AppBootstrap::new();
    let command_tx = bootstrap.command_tx.clone();

    let handle = std::thread::spawn(move || runner.run(true, bootstrap));

    std::thread::sleep(Duration::from_millis(150));

    assert_eq!(launches.load(Ordering::Relaxed), 0);

    command_tx
        .send(CoreCommand::Shutdown)
        .expect("send shutdown command");
    let result = handle.join().expect("daemon thread should not panic");
    result.expect("daemon run");
}

#[test]
fn non_daemon_mode_launches_gui_once() {
    let launches = Arc::new(AtomicUsize::new(0));
    let gui = CountingGui {
        launches: launches.clone(),
    };

    let runner = AppRunner::new(gui);
    let bootstrap = AppBootstrap::new();
    runner.run(false, bootstrap).expect("gui run");

    assert_eq!(launches.load(Ordering::Relaxed), 1);
}

#[test]
fn daemon_mode_reaches_ready_and_roundtrip() {
    let launches = Arc::new(AtomicUsize::new(0));
    let gui = CountingGui { launches };
    let runner = AppRunner::new(gui);

    let bootstrap = AppBootstrap::new();
    let command_tx = bootstrap.command_tx.clone();

    let handle = std::thread::spawn(move || runner.run(true, bootstrap));

    std::thread::sleep(Duration::from_millis(150));
    command_tx
        .send(CoreCommand::Shutdown)
        .expect("send shutdown command");

    let result = handle.join().expect("daemon thread should not panic");
    result.expect("daemon mode should get ready and ping/pong");
}

#[test]
fn daemon_mode_stays_alive_until_shutdown_command() {
    let launches = Arc::new(AtomicUsize::new(0));
    let gui = CountingGui { launches };
    let runner = AppRunner::new(gui);

    let bootstrap = AppBootstrap::new();
    let command_tx = bootstrap.command_tx.clone();

    let handle = std::thread::spawn(move || runner.run(true, bootstrap));

    std::thread::sleep(Duration::from_millis(150));
    assert!(
        !handle.is_finished(),
        "daemon mode should remain running until explicitly shut down"
    );

    command_tx
        .send(CoreCommand::Shutdown)
        .expect("send shutdown command");
    let result = handle.join().expect("daemon thread should not panic");
    result.expect("daemon mode should exit cleanly after shutdown command");
}

#[test]
fn tray_creation_gate_respects_daemon_and_config_flag() {
    assert!(!should_create_tray(false));
    assert!(should_create_tray(true));
}
