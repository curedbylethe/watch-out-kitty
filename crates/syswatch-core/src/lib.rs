//! Core types and logic for syswatch.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name_matches_package() {
        assert_eq!(env!("CARGO_PKG_NAME"), "syswatch-core");
    }
}
