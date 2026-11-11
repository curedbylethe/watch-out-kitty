use syswatch_core::{AggregateSnapshot, Collector, ProcessHistory, ProcessSnapshot};

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessScan {
    pub processes: Vec<ProcessSnapshot>,
    pub observed_at_ms: u64,
}

pub trait Sampler {
    fn refresh_aggregate(&mut self) -> AggregateSnapshot;
    fn refresh_processes(&mut self) -> ProcessScan;
}

pub struct LiveSampler {
    collector: Collector,
    history: ProcessHistory,
}

impl LiveSampler {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            collector: Collector::new().map_err(|error| error.to_string())?,
            history: ProcessHistory::new(),
        })
    }

    pub fn history(&self) -> &ProcessHistory {
        &self.history
    }
}

impl Sampler for LiveSampler {
    fn refresh_aggregate(&mut self) -> AggregateSnapshot {
        self.collector.refresh_aggregate()
    }

    fn refresh_processes(&mut self) -> ProcessScan {
        let processes = self.collector.refresh_processes().to_vec();
        let observed_at_ms = self
            .collector
            .processes_updated_at_ms()
            .expect("process refresh sets timestamp");
        self.history.record_scan(observed_at_ms, &processes);
        ProcessScan {
            processes,
            observed_at_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LiveSampler, Sampler};

    #[test]
    fn live_sampler_records_every_process_scan_in_history() {
        let mut sampler = LiveSampler::new().unwrap();
        let scan = sampler.refresh_processes();
        assert_eq!(
            sampler.history().len(),
            scan.processes
                .len()
                .min(sampler.history().limits().max_processes)
        );
        let len = sampler.history().len();
        sampler.refresh_aggregate();
        assert_eq!(sampler.history().len(), len);
        sampler.refresh_processes();
    }
}
