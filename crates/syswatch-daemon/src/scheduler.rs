use std::{
    error::Error,
    fmt,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use syswatch_core::MonitoringConfig;

use crate::{
    clock::{Clock, WaitOutcome},
    latest::LatestSnapshot,
    sampler::Sampler,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerError;

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sampling deadline is not representable")
    }
}
impl Error for SchedulerError {}

pub struct Scheduler<C, S> {
    clock: C,
    sampler: S,
    config: MonitoringConfig,
    aggregate_deadline: Instant,
    process_deadline: Instant,
    latest: Arc<Mutex<LatestSnapshot>>,
}

impl<C: Clock, S: Sampler> Scheduler<C, S> {
    pub fn new(clock: C, sampler: S, config: MonitoringConfig) -> Self {
        let now = clock.now();
        Self {
            clock,
            sampler,
            config,
            aggregate_deadline: now,
            process_deadline: now,
            latest: Arc::new(Mutex::new(LatestSnapshot::default())),
        }
    }

    pub fn latest(&self) -> Option<syswatch_core::SystemSnapshot> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .snapshot()
    }

    pub fn latest_handle(&self) -> Arc<Mutex<LatestSnapshot>> {
        Arc::clone(&self.latest)
    }

    pub fn run(&mut self) -> Result<(), SchedulerError> {
        loop {
            let now = self.clock.now();
            if now >= self.aggregate_deadline {
                let aggregate = self.sampler.refresh_aggregate();
                self.latest
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .set_aggregate(aggregate);
                self.aggregate_deadline =
                    checked_deadline(self.clock.now(), self.config.aggregate_interval())?;
            }
            if self.clock.now() >= self.process_deadline {
                let processes = self.sampler.refresh_processes();
                self.latest
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .set_processes(processes);
                self.process_deadline =
                    checked_deadline(self.clock.now(), self.config.process_interval())?;
            }
            let deadline = self.aggregate_deadline.min(self.process_deadline);
            if self.clock.wait_until(deadline) == WaitOutcome::Shutdown {
                return Ok(());
            }
        }
    }
}

fn checked_deadline(now: Instant, interval: Duration) -> Result<Instant, SchedulerError> {
    now.checked_add(interval).ok_or(SchedulerError)
}

#[cfg(test)]
mod tests {
    use super::{Scheduler, checked_deadline};
    use crate::{
        clock::{Clock, WaitOutcome},
        sampler::{ProcessScan, Sampler},
    };
    use std::{
        cell::Cell,
        rc::Rc,
        time::{Duration, Instant},
    };
    use syswatch_core::{AggregateSnapshot, MemorySnapshot, MonitoringConfig};

    struct FakeClock {
        now: Rc<Cell<Instant>>,
        shutdown_after: usize,
        wait_count: Cell<usize>,
        waits: RefCell<Vec<Instant>>,
    }
    use std::cell::RefCell;
    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.now.get()
        }
        fn wait_until(&self, deadline: Instant) -> WaitOutcome {
            self.waits.borrow_mut().push(deadline);
            self.now.set(deadline);
            let count = self.wait_count.get() + 1;
            self.wait_count.set(count);
            if count == self.shutdown_after {
                WaitOutcome::Shutdown
            } else {
                WaitOutcome::Elapsed
            }
        }
    }
    #[derive(Default)]
    struct FakeSampler {
        calls: Vec<&'static str>,
        now: Option<Rc<Cell<Instant>>>,
        aggregate_work: Duration,
    }
    impl Sampler for FakeSampler {
        fn refresh_aggregate(&mut self) -> AggregateSnapshot {
            self.calls.push("aggregate");
            if let Some(now) = &self.now {
                now.set(now.get() + self.aggregate_work);
            }
            AggregateSnapshot {
                timestamp_ms: 1,
                cpu_usage_percent: 0.0,
                cpu_updated_at_ms: None,
                memory: MemorySnapshot {
                    total_bytes: 1,
                    used_bytes: 0,
                },
            }
        }
        fn refresh_processes(&mut self) -> ProcessScan {
            self.calls.push("process");
            ProcessScan {
                processes: vec![],
                observed_at_ms: 2,
            }
        }
    }
    impl FakeClock {
        fn new(now: Instant, shutdown_after: usize) -> (Self, Rc<Cell<Instant>>) {
            let now = Rc::new(Cell::new(now));
            (
                Self {
                    now: now.clone(),
                    shutdown_after,
                    wait_count: Cell::new(0),
                    waits: RefCell::new(vec![]),
                },
                now,
            )
        }
    }
    fn config(a: u64, p: u64) -> MonitoringConfig {
        MonitoringConfig::new(Duration::from_secs(a), Duration::from_secs(p)).unwrap()
    }

    #[test]
    fn scheduler_refreshes_both_categories_on_the_first_tick() {
        let (clock, _) = FakeClock::new(Instant::now(), 1);
        let mut scheduler = Scheduler::new(clock, FakeSampler::default(), config(1, 5));
        scheduler.run().unwrap();
        assert_eq!(scheduler.sampler.calls, ["aggregate", "process"]);
    }
    #[test]
    fn independent_intervals_refresh_only_the_due_category() {
        let (clock, _) = FakeClock::new(Instant::now(), 3);
        let mut scheduler = Scheduler::new(clock, FakeSampler::default(), config(2, 3));
        scheduler.run().unwrap();
        assert_eq!(
            scheduler.sampler.calls,
            ["aggregate", "process", "aggregate", "process"]
        );
    }
    #[test]
    fn deadlines_are_recomputed_from_completion_time() {
        let start = Instant::now();
        let (clock, now) = FakeClock::new(start, 1);
        let sampler = FakeSampler {
            aggregate_work: Duration::from_secs(5),
            now: Some(now),
            ..FakeSampler::default()
        };
        let mut scheduler = Scheduler::new(clock, sampler, config(2, 3));
        scheduler.run().unwrap();
        assert_eq!(
            scheduler.clock.waits.borrow()[0],
            start + Duration::from_secs(7)
        );
        assert_eq!(scheduler.sampler.calls, ["aggregate", "process"]);
    }
    #[test]
    fn unrepresentable_deadline_is_reported() {
        assert!(checked_deadline(Instant::now(), Duration::MAX).is_err());
    }
    #[test]
    fn shutdown_outcome_stops_the_run_loop() {
        let (clock, _) = FakeClock::new(Instant::now(), 1);
        let mut scheduler = Scheduler::new(clock, FakeSampler::default(), config(1, 5));
        scheduler.run().unwrap();
        assert_eq!(scheduler.sampler.calls.len(), 2);
    }

    #[test]
    fn reading_the_latest_snapshot_does_not_refresh() {
        let (clock, _) = FakeClock::new(Instant::now(), 1);
        let mut scheduler = Scheduler::new(clock, FakeSampler::default(), config(1, 5));
        scheduler.run().unwrap();
        let calls = scheduler.sampler.calls.len();
        assert!(scheduler.latest().is_some());
        assert!(scheduler.latest().is_some());
        assert_eq!(scheduler.sampler.calls.len(), calls);
    }
}
