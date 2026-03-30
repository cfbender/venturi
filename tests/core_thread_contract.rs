use std::time::{Duration, Instant};

use crossbeam_channel::unbounded;
use venturi::core::messages::{CoreCommand, CoreEvent};
use venturi::core::pipewire_manager::PipeWireManager;

#[test]
fn manager_emits_ready_and_handles_ping() {
    let (command_tx, command_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();

    let manager = PipeWireManager::spawn(command_rx, event_tx);
    let ready = event_rx.recv().expect("ready event");
    assert_eq!(ready, CoreEvent::Ready);

    command_tx.send(CoreCommand::Ping).expect("send ping");
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut seen_pong = false;
    while Instant::now() < deadline {
        if let Ok(event) = event_rx.recv_timeout(Duration::from_millis(100))
            && event == CoreEvent::Pong
        {
            seen_pong = true;
            break;
        }
    }
    assert!(seen_pong, "expected to receive pong within 1s");

    command_tx
        .send(CoreCommand::Shutdown)
        .expect("send shutdown");
    manager.join().expect("join manager thread");
}
