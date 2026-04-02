use std::sync::{Arc, Mutex};

use venturi_app_gtk::{GtkLauncher, RuntimeUiEvent, WindowBridge};

#[test]
fn window_bridge_dispatches_runtime_ui_events() {
    let dispatched = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&dispatched);
    let bridge = WindowBridge::new_for_test(Box::new(move |event| {
        sink.lock().expect("capture event").push(event);
    }));

    bridge.on_event(RuntimeUiEvent::Ready);
    bridge.on_event(RuntimeUiEvent::ToggleWindowRequested);
    bridge.on_event(RuntimeUiEvent::ShutdownRequested);

    let events = dispatched.lock().expect("read dispatched events").clone();
    assert_eq!(
        events,
        vec![
            RuntimeUiEvent::Ready,
            RuntimeUiEvent::ToggleWindowRequested,
            RuntimeUiEvent::ShutdownRequested,
        ]
    );
}

#[test]
fn launcher_notify_ready_forwards_ready_event() {
    let dispatched = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&dispatched);
    let bridge = WindowBridge::new_for_test(Box::new(move |event| {
        sink.lock().expect("capture event").push(event);
    }));

    let launcher = GtkLauncher::new(bridge);
    launcher.notify_ready();

    let events = dispatched.lock().expect("read dispatched events").clone();
    assert_eq!(events, vec![RuntimeUiEvent::Ready]);
}
