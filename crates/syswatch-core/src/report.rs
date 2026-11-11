//! Formatting helpers for human-readable process reports.

use crate::snapshot::ProcessSnapshot;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProcessSort {
    #[default]
    Memory,
    Cpu,
}

/// Formats a byte count with binary units, from bytes through tebibytes.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1023.95 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Formats per-process CPU usage as a percentage, or `n/a` when unavailable.
/// Per-process CPU can exceed 100% because one fully used core counts as 100%.
pub fn format_cpu_percent(value: f32) -> String {
    if value.is_finite() {
        format!("{value:.1}%")
    } else {
        "n/a".to_owned()
    }
}

/// Renders up to `limit` processes in the selected order.
pub fn render_table(processes: &[ProcessSnapshot], sort: ProcessSort, limit: usize) -> String {
    let rows = top_by_sort(processes, sort, limit);
    if rows.is_empty() {
        return "(no processes observed)".to_owned();
    }

    let mut lines = vec![format!(
        "{:<7} {:<24} {:>10} {:>8}",
        "PID", "NAME", "MEMORY", "CPU"
    )];
    lines.extend(rows.into_iter().map(format_row));
    lines.join("\n")
}

pub fn format_process_age(now_ms: u64, processes_updated_at_ms: Option<u64>) -> String {
    match processes_updated_at_ms {
        Some(timestamp_ms) => format!(
            "process data: {:.1} s old",
            now_ms.saturating_sub(timestamp_ms) as f64 / 1000.0
        ),
        None => "process data: unavailable".to_owned(),
    }
}

fn top_by_sort(
    processes: &[ProcessSnapshot],
    sort: ProcessSort,
    limit: usize,
) -> Vec<&ProcessSnapshot> {
    let mut rows: Vec<_> = processes.iter().collect();
    rows.sort_by(|a, b| {
        let primary = match sort {
            ProcessSort::Memory => b.memory_bytes.cmp(&a.memory_bytes),
            ProcessSort::Cpu => match (
                a.cpu_usage_percent.is_finite(),
                b.cpu_usage_percent.is_finite(),
            ) {
                (true, true) => b.cpu_usage_percent.total_cmp(&a.cpu_usage_percent),
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => std::cmp::Ordering::Equal,
            },
        };
        primary
            .then_with(|| a.pid.cmp(&b.pid))
            .then_with(|| a.start_time_secs.cmp(&b.start_time_secs))
    });
    rows.truncate(limit);
    rows
}

fn format_row(process: &ProcessSnapshot) -> String {
    format!(
        "{:<7} {:<24} {:>10} {:>8}",
        process.pid,
        truncate_name(&process.name),
        format_bytes(process.memory_bytes),
        format_cpu_percent(process.cpu_usage_percent),
    )
}

fn truncate_name(name: &str) -> String {
    let name = if name.trim().is_empty() {
        "<unknown>"
    } else {
        name
    };
    name.chars().take(24).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ProcessSort, format_bytes, format_cpu_percent, format_process_age, render_table,
        top_by_sort, truncate_name,
    };
    use crate::snapshot::ProcessSnapshot;

    fn process(pid: u32, name: &str, memory_bytes: u64, cpu_usage_percent: f32) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: None,
            name: name.to_owned(),
            cpu_usage_percent,
            memory_bytes,
            start_time_secs: 0,
        }
    }

    #[test]
    fn top_by_memory_sorts_limits_and_breaks_ties_by_pid() {
        let processes = [
            process(4, "four", 10, 0.0),
            process(3, "three", 20, 0.0),
            process(1, "one", 20, 0.0),
        ];

        assert_eq!(
            top_by_sort(&processes, ProcessSort::Memory, 2)
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(top_by_sort(&processes, ProcessSort::Memory, 0).is_empty());
        assert!(top_by_sort(&[], ProcessSort::Memory, 10).is_empty());
    }

    #[test]
    fn top_by_cpu_sorts_descending_and_preserves_values_above_100() {
        let processes = [
            process(4, "four", 10, 55.0),
            process(3, "three", 20, 125.0),
            process(1, "one", 20, 80.0),
        ];

        assert_eq!(
            top_by_sort(&processes, ProcessSort::Cpu, 2)
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            [3, 1]
        );
        assert_eq!(
            format_cpu_percent(top_by_sort(&processes, ProcessSort::Cpu, 1)[0].cpu_usage_percent),
            "125.0%"
        );
    }

    #[test]
    fn top_by_cpu_ranks_nonfinite_values_last_and_breaks_identity_ties() {
        let mut same_pid_later = process(7, "later", 0, f32::NAN);
        same_pid_later.start_time_secs = 20;
        let mut same_pid_earlier = process(7, "earlier", 0, f32::INFINITY);
        same_pid_earlier.start_time_secs = 10;
        let processes = [
            same_pid_later,
            process(8, "finite", 0, 1.0),
            same_pid_earlier,
        ];

        assert_eq!(
            top_by_sort(&processes, ProcessSort::Cpu, 3)
                .iter()
                .map(|process| process.name.as_str())
                .collect::<Vec<_>>(),
            ["finite", "earlier", "later"]
        );
        let table = render_table(&processes, ProcessSort::Cpu, 3);
        assert!(table.lines().skip(2).all(|line| line.ends_with("n/a")));
    }

    #[test]
    fn render_table_handles_empty_and_sorts_rows() {
        assert_eq!(
            render_table(&[], ProcessSort::Memory, 10),
            "(no processes observed)"
        );
        let table = render_table(
            &[process(2, "two", 1024, 1.5), process(1, "one", 2048, 0.0)],
            ProcessSort::Memory,
            10,
        );
        assert_eq!(
            table,
            concat!(
                "PID     NAME                         MEMORY      CPU\n",
                "1       one                         2.0 KiB     0.0%\n",
                "2       two                         1.0 KiB     1.5%"
            )
        );
    }

    #[test]
    fn format_process_age_reports_cached_data_age() {
        assert_eq!(format_process_age(5000, None), "process data: unavailable");
        assert_eq!(format_process_age(0, Some(0)), "process data: 0.0 s old");
        assert_eq!(format_process_age(4250, Some(0)), "process data: 4.2 s old");
        assert_eq!(format_process_age(0, Some(1)), "process data: 0.0 s old");
    }

    #[test]
    fn format_bytes_uses_binary_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(1_048_575), "1.0 MiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
        assert_eq!(format_bytes(1_099_511_627_776), "1.0 TiB");
    }

    #[test]
    fn format_cpu_percent_handles_unavailable_values() {
        assert_eq!(format_cpu_percent(0.0), "0.0%");
        assert_eq!(format_cpu_percent(125.5), "125.5%");
        assert_eq!(format_cpu_percent(f32::NAN), "n/a");
        assert_eq!(format_cpu_percent(f32::INFINITY), "n/a");
    }

    #[test]
    fn truncate_name_handles_empty_and_multibyte_names() {
        assert_eq!(truncate_name(""), "<unknown>");
        assert_eq!(truncate_name("   "), "<unknown>");
        assert_eq!(
            truncate_name("abcdefghijklmnopqrstuvwxyz1234"),
            "abcdefghijklmnopqrstuvwx"
        );
        assert_eq!(truncate_name("猫".repeat(30).as_str()), "猫".repeat(24));
    }
}
