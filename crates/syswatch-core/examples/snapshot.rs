//! Diagnostic snapshot example, not a permanent CLI. Run with `cargo run -p syswatch-core --example snapshot`.

use syswatch_core::{collector::Collector, report};

const TOP_N: usize = 10;

fn main() -> Result<(), sysinfo::Error> {
    let mut collector = Collector::new()?;
    collector.refresh();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    let snapshot = collector.refresh();

    println!(
        "CPU: {}",
        report::format_cpu_percent(snapshot.cpu_usage_percent)
    );
    println!(
        "Memory: {} / {} ({:.1}%)",
        report::format_bytes(snapshot.memory.used_bytes),
        report::format_bytes(snapshot.memory.total_bytes),
        snapshot.memory.usage_percent()
    );
    println!("Processes: {}", snapshot.processes.len());
    println!();
    println!("{}", report::render_table(&snapshot.processes, TOP_N));

    Ok(())
}
