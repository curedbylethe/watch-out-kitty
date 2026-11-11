//! Validated polling intervals for a local desktop monitor.
//!
//! [`MonitoringConfig`] defaults to aggregate polling every second and
//! process polling every five seconds. This 5x ratio keeps full process scans
//! less frequent than aggregate CPU and memory reads. Construction rejects
//! zero intervals, intervals below sysinfo's CPU sampling minimum, process
//! polling more frequent than aggregate polling, and intervals that cannot be
//! added to [`Instant::now`]. A scheduler must still use `checked_add` when it
//! creates each deadline because representability can change over time.

use std::{
    error::Error,
    fmt,
    time::{Duration, Instant},
};

/// Validated aggregate and process polling intervals.
///
/// Defaults poll aggregate CPU and memory every second and processes every
/// five seconds. The 5x ratio limits full process scans while keeping process
/// data reasonably fresh. Both intervals must be nonzero and at least
/// [`sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`], and process polling cannot be
/// more frequent than aggregate polling. Each interval must also be addable
/// to the current [`Instant`]. A future scheduler must repeat that checked
/// addition when it creates deadlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitoringConfig {
    aggregate_interval: Duration,
    process_interval: Duration,
}

impl MonitoringConfig {
    /// Default aggregate CPU and memory polling interval: one second.
    pub const DEFAULT_AGGREGATE_INTERVAL: Duration = Duration::from_secs(1);

    /// Default process polling interval: five seconds, five times the aggregate interval.
    pub const DEFAULT_PROCESS_INTERVAL: Duration = Duration::from_secs(5);

    /// Creates configuration after validating both polling intervals.
    ///
    /// CPU intervals must meet sysinfo's minimum because CPU usage needs two
    /// samples. Process polling must be at least as slow as aggregate polling.
    /// Zero or intervals that cannot be added to the current `Instant` are
    /// rejected. A scheduler must check deadline additions again when used.
    pub fn new(
        aggregate_interval: Duration,
        process_interval: Duration,
    ) -> Result<Self, ConfigError> {
        validate_intervals(
            aggregate_interval,
            process_interval,
            sysinfo::MINIMUM_CPU_UPDATE_INTERVAL,
        )?;

        Ok(Self {
            aggregate_interval,
            process_interval,
        })
    }

    /// Returns the aggregate CPU and memory polling interval.
    pub fn aggregate_interval(&self) -> Duration {
        self.aggregate_interval
    }

    /// Returns the process polling interval.
    pub fn process_interval(&self) -> Duration {
        self.process_interval
    }
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self::new(
            Self::DEFAULT_AGGREGATE_INTERVAL,
            Self::DEFAULT_PROCESS_INTERVAL,
        )
        .expect("default monitoring intervals are valid")
    }
}

/// Why a [`MonitoringConfig`] interval is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// Aggregate polling interval is zero.
    ZeroAggregateInterval,
    /// Process polling interval is zero.
    ZeroProcessInterval,
    /// Aggregate interval is below sysinfo's CPU sampling minimum.
    AggregateBelowCpuMinimum(Duration),
    /// Process interval is below sysinfo's CPU sampling minimum.
    ProcessBelowCpuMinimum(Duration),
    /// Process polling would run more often than aggregate polling.
    ProcessIntervalShorterThanAggregate,
    /// An interval cannot be added to the current [`Instant`].
    IntervalTooLarge,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroAggregateInterval => write!(formatter, "aggregate interval must not be zero"),
            Self::ZeroProcessInterval => write!(formatter, "process interval must not be zero"),
            Self::AggregateBelowCpuMinimum(minimum) => write!(
                formatter,
                "aggregate interval is below the CPU sampling minimum of {minimum:?}"
            ),
            Self::ProcessBelowCpuMinimum(minimum) => write!(
                formatter,
                "process interval is below the CPU sampling minimum of {minimum:?}"
            ),
            Self::ProcessIntervalShorterThanAggregate => write!(
                formatter,
                "process interval must not be shorter than aggregate interval"
            ),
            Self::IntervalTooLarge => {
                write!(formatter, "interval cannot be added to the current instant")
            }
        }
    }
}

impl Error for ConfigError {}

fn validate_intervals(
    aggregate: Duration,
    process: Duration,
    cpu_minimum: Duration,
) -> Result<(), ConfigError> {
    if aggregate.is_zero() {
        return Err(ConfigError::ZeroAggregateInterval);
    }
    if process.is_zero() {
        return Err(ConfigError::ZeroProcessInterval);
    }
    if aggregate < cpu_minimum {
        return Err(ConfigError::AggregateBelowCpuMinimum(cpu_minimum));
    }
    if process < cpu_minimum {
        return Err(ConfigError::ProcessBelowCpuMinimum(cpu_minimum));
    }
    if process < aggregate {
        return Err(ConfigError::ProcessIntervalShorterThanAggregate);
    }

    if Instant::now().checked_add(process).is_none() {
        return Err(ConfigError::IntervalTooLarge);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ConfigError, MonitoringConfig, validate_intervals};

    const CPU_MINIMUM: Duration = Duration::from_millis(200);

    #[test]
    fn default_intervals_are_one_and_five_seconds() {
        let config = MonitoringConfig::default();

        assert_eq!(config.aggregate_interval(), Duration::from_secs(1));
        assert_eq!(config.process_interval(), Duration::from_secs(5));
        assert!(config.process_interval() > config.aggregate_interval());
    }

    #[test]
    fn default_configuration_is_valid() {
        assert!(
            MonitoringConfig::new(
                MonitoringConfig::DEFAULT_AGGREGATE_INTERVAL,
                MonitoringConfig::DEFAULT_PROCESS_INTERVAL,
            )
            .is_ok()
        );
    }

    #[test]
    fn accepts_exactly_the_cpu_minimum() {
        assert_eq!(
            validate_intervals(CPU_MINIMUM, CPU_MINIMUM, CPU_MINIMUM),
            Ok(())
        );
    }

    #[test]
    fn rejects_aggregate_below_cpu_minimum() {
        let below = CPU_MINIMUM
            .checked_sub(Duration::from_nanos(1))
            .expect("test CPU minimum is nonzero");

        assert_eq!(
            validate_intervals(below, CPU_MINIMUM, CPU_MINIMUM),
            Err(ConfigError::AggregateBelowCpuMinimum(CPU_MINIMUM))
        );
    }

    #[test]
    fn rejects_process_below_cpu_minimum() {
        let below = CPU_MINIMUM
            .checked_sub(Duration::from_nanos(1))
            .expect("test CPU minimum is nonzero");

        assert_eq!(
            validate_intervals(CPU_MINIMUM, below, CPU_MINIMUM),
            Err(ConfigError::ProcessBelowCpuMinimum(CPU_MINIMUM))
        );
    }

    #[test]
    fn rejects_zero_aggregate_interval() {
        assert_eq!(
            validate_intervals(Duration::ZERO, CPU_MINIMUM, CPU_MINIMUM),
            Err(ConfigError::ZeroAggregateInterval)
        );
    }

    #[test]
    fn rejects_zero_process_interval() {
        assert_eq!(
            validate_intervals(CPU_MINIMUM, Duration::ZERO, CPU_MINIMUM),
            Err(ConfigError::ZeroProcessInterval)
        );
    }

    #[test]
    fn rejects_process_interval_shorter_than_aggregate() {
        assert_eq!(
            validate_intervals(Duration::from_secs(2), Duration::from_secs(1), CPU_MINIMUM,),
            Err(ConfigError::ProcessIntervalShorterThanAggregate)
        );
    }

    #[test]
    fn accepts_equal_intervals() {
        let interval = Duration::from_secs(1);

        assert_eq!(validate_intervals(interval, interval, CPU_MINIMUM), Ok(()));
    }

    #[test]
    fn rejects_unrepresentable_interval() {
        assert_eq!(
            validate_intervals(Duration::from_secs(1), Duration::MAX, CPU_MINIMUM),
            Err(ConfigError::IntervalTooLarge)
        );
    }

    #[test]
    fn accessors_return_constructed_values() {
        let aggregate = Duration::from_secs(2);
        let process = Duration::from_secs(4);
        let config = MonitoringConfig::new(aggregate, process).unwrap();

        assert_eq!(config.aggregate_interval(), aggregate);
        assert_eq!(config.process_interval(), process);
    }
}
