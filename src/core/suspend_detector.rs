use std::time::{Duration, Instant, SystemTime};

/// How far the wall clock must run ahead of the monotonic clock between two
/// core-loop ticks before we call it a suspend/resume cycle. The loop ticks
/// every 50 ms, so anything in the seconds is not scheduling jitter.
pub(crate) const SUSPEND_GAP_THRESHOLD: Duration = Duration::from_secs(5);

/// Detects that the machine went to sleep and woke up again.
///
/// On Linux `Instant` is `CLOCK_MONOTONIC`, which does not advance while the
/// system is suspended, whereas `SystemTime` (`CLOCK_REALTIME`) does. Comparing
/// how much each clock advanced between two polls therefore exposes a suspend
/// without any D-Bus/logind dependency. A wall-clock step (NTP correction,
/// manual change) looks the same; the consequence is only a harmless forced
/// re-route, so that false positive is accepted.
#[derive(Debug)]
pub(crate) struct SuspendDetector {
    wall: SystemTime,
    mono: Instant,
}

impl SuspendDetector {
    pub(crate) fn new() -> Self {
        Self::with_clocks(SystemTime::now(), Instant::now())
    }

    pub(crate) fn with_clocks(wall: SystemTime, mono: Instant) -> Self {
        Self { wall, mono }
    }

    /// Returns `true` if a suspend appears to have happened since the last poll.
    pub(crate) fn poll(&mut self) -> bool {
        self.poll_with_clocks(SystemTime::now(), Instant::now())
    }

    pub(crate) fn poll_with_clocks(&mut self, now_wall: SystemTime, now_mono: Instant) -> bool {
        let wall_elapsed = now_wall.duration_since(self.wall).unwrap_or_default();
        let mono_elapsed = now_mono.saturating_duration_since(self.mono);
        self.wall = now_wall;
        self.mono = now_mono;
        suspend_detected(wall_elapsed, mono_elapsed)
    }
}

pub(crate) fn suspend_detected(wall_elapsed: Duration, mono_elapsed: Duration) -> bool {
    wall_elapsed.saturating_sub(mono_elapsed) >= SUSPEND_GAP_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::{SUSPEND_GAP_THRESHOLD, SuspendDetector, suspend_detected};
    use std::time::{Duration, Instant, SystemTime};

    #[test]
    fn ordinary_ticks_are_not_a_suspend() {
        assert!(!suspend_detected(
            Duration::from_millis(52),
            Duration::from_millis(50)
        ));
        // A long GC-like stall advances both clocks equally.
        assert!(!suspend_detected(
            Duration::from_secs(30),
            Duration::from_secs(30)
        ));
    }

    #[test]
    fn wall_clock_running_ahead_of_monotonic_is_a_suspend() {
        assert!(suspend_detected(
            Duration::from_secs(3600) + Duration::from_millis(50),
            Duration::from_millis(50)
        ));
        assert!(suspend_detected(
            SUSPEND_GAP_THRESHOLD + Duration::from_millis(50),
            Duration::from_millis(50)
        ));
        assert!(!suspend_detected(
            SUSPEND_GAP_THRESHOLD - Duration::from_millis(1) + Duration::from_millis(50),
            Duration::from_millis(50)
        ));
    }

    #[test]
    fn wall_clock_going_backwards_is_ignored() {
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mono = Instant::now();
        let mut detector = SuspendDetector::with_clocks(wall, mono);

        assert!(!detector.poll_with_clocks(
            wall - Duration::from_secs(600),
            mono + Duration::from_millis(50)
        ));
    }

    #[test]
    fn detector_fires_once_per_gap_and_then_resets() {
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mono = Instant::now();
        let mut detector = SuspendDetector::with_clocks(wall, mono);

        assert!(!detector.poll_with_clocks(
            wall + Duration::from_millis(50),
            mono + Duration::from_millis(50)
        ));
        // Sleep for two hours; monotonic clock only sees the tick interval.
        assert!(detector.poll_with_clocks(
            wall + Duration::from_secs(7200),
            mono + Duration::from_millis(100)
        ));
        // Next tick after resume is ordinary again.
        assert!(!detector.poll_with_clocks(
            wall + Duration::from_secs(7200) + Duration::from_millis(50),
            mono + Duration::from_millis(150)
        ));
    }
}
