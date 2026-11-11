use std::{
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Elapsed,
    Shutdown,
}

pub trait Clock {
    fn now(&self) -> Instant;
    fn wait_until(&self, deadline: Instant) -> WaitOutcome;
}

pub struct SystemClock {
    shutdown: Receiver<i32>,
}

impl SystemClock {
    pub fn new(shutdown: Receiver<i32>) -> Self {
        Self { shutdown }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wait_until(&self, deadline: Instant) -> WaitOutcome {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match self.shutdown.recv_timeout(remaining) {
            Ok(_) | Err(RecvTimeoutError::Disconnected) => WaitOutcome::Shutdown,
            Err(RecvTimeoutError::Timeout) => WaitOutcome::Elapsed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, SystemClock, WaitOutcome};
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    #[test]
    fn system_clock_returns_shutdown_when_a_signal_is_queued() {
        let (tx, rx) = mpsc::channel();
        tx.send(signal_hook::consts::SIGTERM).unwrap();
        let clock = SystemClock::new(rx);
        assert_eq!(
            clock.wait_until(Instant::now() + Duration::from_secs(60)),
            WaitOutcome::Shutdown
        );
    }

    #[test]
    fn system_clock_returns_shutdown_when_the_channel_is_disconnected() {
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let clock = SystemClock::new(rx);
        assert_eq!(
            clock.wait_until(Instant::now() + Duration::from_secs(60)),
            WaitOutcome::Shutdown
        );
    }
}
