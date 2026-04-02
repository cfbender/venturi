use std::error::Error;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use async_trait::async_trait;
use tokio::sync::watch;
use venturi_application::{
    AppError, Channel, DeviceEntry, DeviceKind, DeviceService, MeterService, MeterSnapshot,
    RouteCommand, RoutingService, SessionService, SoundboardService, StableDeviceId,
};

struct InMemoryServices {
    levels_rx: watch::Receiver<MeterSnapshot>,
    _levels_tx: watch::Sender<MeterSnapshot>,
    selected_output: Mutex<Option<StableDeviceId>>,
    selected_input: Mutex<Option<StableDeviceId>>,
    last_volume: Mutex<Option<(Channel, u8)>>,
    played_pad: AtomicU32,
    previewed_pad: AtomicU32,
    stopped_pad: AtomicU32,
    saved: AtomicBool,
    restored: AtomicBool,
}

impl InMemoryServices {
    fn new() -> Self {
        let (levels_tx, levels_rx) = watch::channel(MeterSnapshot {
            channel: Channel::Main,
            level: 0.42,
            peak: 0.9,
        });

        Self {
            levels_rx,
            _levels_tx: levels_tx,
            selected_output: Mutex::new(None),
            selected_input: Mutex::new(None),
            last_volume: Mutex::new(None),
            played_pad: AtomicU32::new(0),
            previewed_pad: AtomicU32::new(0),
            stopped_pad: AtomicU32::new(0),
            saved: AtomicBool::new(false),
            restored: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl RoutingService for InMemoryServices {
    async fn set_volume(&self, channel: Channel, value: f32) -> Result<(), AppError> {
        let quantized = (value * 100.0).round() as u8;
        *self.last_volume.lock().expect("volume lock") = Some((channel, quantized));
        Ok(())
    }
}

#[async_trait]
impl DeviceService for InMemoryServices {
    async fn list_devices(&self) -> Result<Vec<DeviceEntry>, AppError> {
        Ok(vec![
            DeviceEntry {
                kind: DeviceKind::Output,
                id: StableDeviceId("output-1".into()),
                label: "Headphones".into(),
            },
            DeviceEntry {
                kind: DeviceKind::Input,
                id: StableDeviceId("input-1".into()),
                label: "Microphone".into(),
            },
        ])
    }

    async fn select_output(&self, output: Option<StableDeviceId>) -> Result<(), AppError> {
        *self.selected_output.lock().expect("output lock") = output;
        Ok(())
    }

    async fn select_input(&self, input: Option<StableDeviceId>) -> Result<(), AppError> {
        *self.selected_input.lock().expect("input lock") = input;
        Ok(())
    }
}

#[async_trait]
impl MeterService for InMemoryServices {
    async fn set_enabled(&self, enabled: bool) -> Result<(), AppError> {
        if enabled {
            Ok(())
        } else {
            Err(AppError::Validation("meter disabled for test".into()))
        }
    }

    fn subscribe_levels(&self) -> watch::Receiver<MeterSnapshot> {
        self.levels_rx.clone()
    }
}

#[async_trait]
impl SoundboardService for InMemoryServices {
    async fn play(&self, pad_id: u32, _file: String) -> Result<(), AppError> {
        self.played_pad.store(pad_id, Ordering::Relaxed);
        Ok(())
    }

    async fn preview(&self, pad_id: u32, _file: String) -> Result<(), AppError> {
        self.previewed_pad.store(pad_id, Ordering::Relaxed);
        Ok(())
    }

    async fn stop(&self, pad_id: u32) -> Result<(), AppError> {
        self.stopped_pad.store(pad_id, Ordering::Relaxed);
        Ok(())
    }
}

#[async_trait]
impl SessionService for InMemoryServices {
    async fn save(&self) -> Result<(), AppError> {
        self.saved.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn restore(&self) -> Result<(), AppError> {
        self.restored.store(true, Ordering::Relaxed);
        Ok(())
    }
}

#[tokio::test]
async fn all_service_traits_support_trait_object_dispatch() {
    let services = InMemoryServices::new();

    let routing: &(dyn RoutingService + Send + Sync) = &services;
    routing
        .set_volume(Channel::Main, 0.75)
        .await
        .expect("route");

    let devices: &(dyn DeviceService + Send + Sync) = &services;
    let listed = devices.list_devices().await.expect("list devices");
    devices
        .select_output(Some(StableDeviceId("output-1".into())))
        .await
        .expect("select output");
    devices
        .select_input(Some(StableDeviceId("input-1".into())))
        .await
        .expect("select input");

    let meters: &(dyn MeterService + Send + Sync) = &services;
    meters.set_enabled(true).await.expect("enable meters");
    let levels = meters.subscribe_levels();

    let soundboard: &(dyn SoundboardService + Send + Sync) = &services;
    soundboard
        .play(1, "airhorn.wav".into())
        .await
        .expect("play");
    soundboard
        .preview(2, "preview.wav".into())
        .await
        .expect("preview");
    soundboard.stop(3).await.expect("stop");

    let session: &(dyn SessionService + Send + Sync) = &services;
    session.save().await.expect("save");
    session.restore().await.expect("restore");

    assert_eq!(listed.len(), 2);
    assert_eq!(levels.borrow().channel, Channel::Main);
}

#[test]
fn route_command_enum_remains_typed() {
    let command = RouteCommand::Connect {
        source: StableDeviceId("stream-main".into()),
        target: StableDeviceId("sink-main".into()),
    };

    assert!(matches!(command, RouteCommand::Connect { .. }));
}

#[test]
fn app_error_implements_display_and_error() {
    let error = AppError::Validation("invalid channel".into());

    assert_eq!(error.to_string(), "validation error: invalid channel");

    fn assert_std_error(value: &dyn Error) {
        assert_eq!(value.to_string(), "validation error: invalid channel");
    }

    assert_std_error(&error);
}
