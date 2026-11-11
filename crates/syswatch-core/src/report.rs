//! Formatting helpers for human-readable process reports.

use crate::snapshot::ProcessSnapshot;

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

/// Renders up to `limit` processes, ordered by memory descending and PID ascending.
pub fn render_table(processes: &[ProcessSnapshot], limit: usize) -> String {
    let rows = top_by_memory(processes, limit);
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

fn top_by_memory(processes: &[ProcessSnapshot], limit: usize) -> Vec<&ProcessSnapshot> {
    let mut rows: Vec<_> = processes.iter().collect();
    rows.sort_by(|a, b| {
        b.memory_bytes
            .cmp(&a.memory_bytes)
            .then_with(|| a.pid.cmp(&b.pid))
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
    use super::{format_bytes, format_cpu_percent, render_table, top_by_memory, truncate_name};
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
            top_by_memory(&processes, 2)
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(top_by_memory(&processes, 0).is_empty());
        assert!(top_by_memory(&[], 10).is_empty());
    }

    #[test]
    fn render_table_handles_empty_and_sorts_rows() {
        assert_eq!(render_table(&[], 10), "(no processes observed)");
        let table = render_table(
            &[process(2, "two", 1024, 1.5), process(1, "one", 2048, 0.0)],
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
