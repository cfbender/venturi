use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use venturi::app::{AppBootstrap, AppRunner, GuiLauncher};

#[derive(Clone)]
struct CountingGui {
    launches: Arc<AtomicUsize>,
}

impl GuiLauncher for CountingGui {
    fn launch(&self) -> Result<(), String> {
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
    runner.run(true, bootstrap).expect("daemon run");

    assert_eq!(launches.load(Ordering::Relaxed), 0);
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
    runner
        .run(true, bootstrap)
        .expect("daemon mode should get ready and ping/pong");
}
