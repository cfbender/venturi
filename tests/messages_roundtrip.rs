use crossbeam_channel::unbounded;
use venturi::core::messages::{Channel, CoreCommand, CoreEvent};

#[test]
fn command_and_event_are_debuggable_and_sendable() {
    let cmd = CoreCommand::SetVolume(Channel::Game, 0.8);
    let evt = CoreEvent::StreamAppeared {
        app_key: "org.venturi.venturi".to_string(),
        id: 10,
        name: "Discord".to_string(),
        category: Channel::Chat,
    };

    assert!(format!("{cmd:?}").contains("SetVolume"));
    assert!(format!("{evt:?}").contains("StreamAppeared"));

    let (cmd_tx, cmd_rx) = unbounded();
    let (evt_tx, evt_rx) = unbounded();
    cmd_tx.send(cmd.clone()).expect("send command");
    evt_tx.send(evt.clone()).expect("send event");

    assert_eq!(cmd_rx.recv().expect("recv command"), cmd);
    assert_eq!(evt_rx.recv().expect("recv event"), evt);
}
