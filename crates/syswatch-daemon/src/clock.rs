use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Elapsed,
    Shutdown,
}

pub trait Clock {
    fn now(&self) -> Instant;
    fn wait_until(&self, deadline: Instant) -> WaitOutcome;
}

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wait_until(&self, deadline: Instant) -> WaitOutcome {
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        WaitOutcome::Elapsed
    }
}
