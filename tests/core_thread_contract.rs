use std::time::{Duration, Instant};

use crossbeam_channel::unbounded;
use venturi::core::messages::{CoreCommand, CoreEvent};
use venturi::core::pipewire_manager::PipeWireManager;

fn wait_for_ready(event_rx: &crossbeam_channel::Receiver<CoreEvent>) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if let Ok(event) = event_rx.recv_timeout(Duration::from_millis(100))
            && event == CoreEvent::Ready
        {
            return;
        }
    }

    panic!("expected to receive Ready within 1s");
}

#[test]
fn manager_emits_ready() {
    let (command_tx, command_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();

    let manager = PipeWireManager::spawn(command_rx, event_tx);
    wait_for_ready(&event_rx);

    command_tx
        .send(CoreCommand::Shutdown)
        .expect("send shutdown");
    manager.join().expect("join manager thread");
}

#[test]
fn manager_emits_toggle_window_event() {
    let (command_tx, command_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();

    let manager = PipeWireManager::spawn(command_rx, event_tx);
    wait_for_ready(&event_rx);

    command_tx
        .send(CoreCommand::ToggleWindow)
        .expect("send toggle-window command");

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut seen_toggle = false;
    while Instant::now() < deadline {
        if let Ok(event) = event_rx.recv_timeout(Duration::from_millis(100))
            && event == CoreEvent::ToggleWindowRequested
        {
            seen_toggle = true;
            break;
        }
    }

    assert!(
        seen_toggle,
        "expected to receive ToggleWindowRequested within 1s"
    );

    command_tx
        .send(CoreCommand::Shutdown)
        .expect("send shutdown");
    manager.join().expect("join manager thread");
}

#[test]
fn manager_emits_shutdown_requested_before_exit() {
    let (command_tx, command_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();

    let manager = PipeWireManager::spawn(command_rx, event_tx);
    wait_for_ready(&event_rx);

    command_tx
        .send(CoreCommand::Shutdown)
        .expect("send shutdown command");

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut seen_shutdown_requested = false;
    while Instant::now() < deadline {
        if let Ok(event) = event_rx.recv_timeout(Duration::from_millis(100))
            && event == CoreEvent::ShutdownRequested
        {
            seen_shutdown_requested = true;
            break;
        }
    }

    assert!(
        seen_shutdown_requested,
        "expected to receive ShutdownRequested before core exits"
    );

    manager.join().expect("join manager thread");
}
