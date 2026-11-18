use std::time::{Duration, Instant};

// Requested host policy: fixed 60 Hz, with bounded catch-up for scheduling
// jitter. Each due update executes the VM; no game instructions are skipped.
pub const MAX_UPDATES_PER_PUMP: usize = 4;
const LONG_STALL: Duration = Duration::from_millis(250);

// Construct this only when the display is ready: setup time is not playback.
pub struct PlaybackClock {
    pub next_tick: Instant,
    pub deadline: Option<Instant>,
    remainder: u32,
    pub stalls: u64,
}

impl PlaybackClock {
    pub fn start(now: Instant, smoke: Duration) -> Self {
        Self {
            next_tick: now,
            deadline: (!smoke.is_zero()).then(|| now + smoke),
            remainder: 0,
            stalls: 0,
        }
    }

    pub fn begin_tick(&mut self, now: Instant) -> bool {
        if now < self.next_tick {
            return false;
        }
        // Suspension/loading must not replay seconds of stale input. This
        // discontinuity is counted in the performance report.
        if now.duration_since(self.next_tick) > LONG_STALL {
            self.next_tick = now;
            self.remainder = 0;
            self.stalls += 1;
        }
        self.remainder += 40;
        self.next_tick += Duration::from_nanos(16_666_666 + u64::from(self.remainder / 60));
        self.remainder %= 60;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_setup_time_is_not_playback_backlog_or_smoke_time() {
        let setup = Instant::now();
        let ready = setup + Duration::from_secs(4);
        let mut clock = PlaybackClock::start(ready, Duration::from_secs(3));
        assert_eq!(clock.next_tick, ready);
        assert!(clock.begin_tick(ready));
        assert!(clock.next_tick > ready);
        assert_eq!(clock.deadline, Some(ready + Duration::from_secs(3)));
    }

    #[test]
    fn long_stall_does_not_replay_stale_input() {
        let ready = Instant::now();
        let now = ready + Duration::from_secs(2);
        let mut clock = PlaybackClock::start(ready, Duration::ZERO);
        assert!(clock.begin_tick(ready));
        assert!(clock.begin_tick(now));
        assert!(!clock.begin_tick(now));
        assert!(!clock.begin_tick(now + Duration::from_micros(16_666)));
        assert!(clock.begin_tick(now + Duration::from_micros(16_667)));
        assert_eq!(clock.stalls, 1);
        assert_eq!(clock.deadline, None);
    }

    #[test]
    fn late_wakeup_does_not_accumulate_script_slowdown() {
        let ready = Instant::now();
        let mut clock = PlaybackClock::start(ready, Duration::ZERO);
        for _ in 0..600 {
            let now = clock.next_tick + Duration::from_millis(3);
            assert!(clock.begin_tick(now));
            assert!(!clock.begin_tick(now));
        }
        assert_eq!(clock.next_tick, ready + Duration::from_secs(10));
    }

    #[test]
    fn short_stall_catches_up_with_individual_updates() {
        let ready = Instant::now();
        let mut clock = PlaybackClock::start(ready, Duration::ZERO);
        let now = ready + Duration::from_millis(50);
        let count = (0..MAX_UPDATES_PER_PUMP).filter(|_| clock.begin_tick(now)).count();
        assert_eq!(count, 4);
        assert!(!clock.begin_tick(now));
        assert_eq!(clock.stalls, 0);
    }
}
