//! Core types and logic for syswatch.

pub mod collector;
pub mod snapshot;
pub use collector::Collector;
pub use snapshot::{MemorySnapshot, ProcessSnapshot, SystemSnapshot};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name_matches_package() {
        assert_eq!(env!("CARGO_PKG_NAME"), "syswatch-core");
    }
}
