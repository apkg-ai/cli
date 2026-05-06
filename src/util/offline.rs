use std::sync::OnceLock;

static OFFLINE: OnceLock<bool> = OnceLock::new();

/// Record whether `--offline` was passed. Call once from `main` before
/// dispatching. Subsequent calls are ignored (the `OnceLock` is written at
/// most once per process).
pub fn set_from_cli(enabled: bool) {
    let _ = OFFLINE.set(enabled);
}

/// Returns true if offline mode is active. Either the `--offline` flag
/// (recorded via `set_from_cli`) or `APKG_OFFLINE=1` / `APKG_OFFLINE=true`
/// enables it.
pub fn is_offline() -> bool {
    if OFFLINE.get().copied().unwrap_or(false) {
        return true;
    }
    std::env::var("APKG_OFFLINE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::env_lock;

    #[test]
    fn test_env_var_enables_offline() {
        let _lock = env_lock();
        std::env::set_var("APKG_OFFLINE", "1");
        assert!(is_offline());
        std::env::set_var("APKG_OFFLINE", "true");
        assert!(is_offline());
        std::env::set_var("APKG_OFFLINE", "TRUE");
        assert!(is_offline());
        std::env::set_var("APKG_OFFLINE", "0");
        assert!(!is_offline());
        std::env::remove_var("APKG_OFFLINE");
        assert!(!is_offline());
    }
}
