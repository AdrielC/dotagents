//! **Environment variable policy.**
//!
//! `agentz` reads exactly zero environment variables from library code. The runtime *may* read
//! a small number — always with the `AGENTZ_` prefix — and callers must go through [`var`] so
//! every lookup is auditable and the prefix is never accidentally dropped.
//!
//! Rule of thumb: **command-line flags and config files first, env second, and only for values
//! that must vary per-shell-session** (tmp directory for demos, CI overrides, etc.).
//!
//! ```
//! use agentz::env::{var, PREFIX};
//! assert_eq!(PREFIX, "AGENTZ_");
//! let _ = var("AGENTZ_HOME"); // None in the test env; that's fine.
//! ```

/// Mandatory prefix for every `agentz`-owned environment variable. Lookups without this prefix
/// through [`var`] panic at call time — that's on purpose, so accidentally-namespaced reads
/// (`HOME`, `PATH`, `FOO`) surface during development instead of silently succeeding.
pub const PREFIX: &str = "AGENTZ_";

/// Read an `AGENTZ_*` environment variable. Returns `None` when unset or set to an empty string.
///
/// # Panics
///
/// Panics if `name` does not start with [`PREFIX`]. This is a *programmer error* guard; real
/// user input is never what reaches here.
#[must_use]
pub fn var(name: &str) -> Option<String> {
    assert!(
        name.starts_with(PREFIX),
        "agentz env vars must be prefixed `{PREFIX}` (got `{name}`)"
    );
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Documented `AGENTZ_*` variables. Adding a new one? Add the const here and document why it
/// can't live in config instead.
pub mod keys {
    /// Override the default `~/.agentz` home location. Unused by the library; honoured by demo
    /// binaries and the eventual `agentz` CLI.
    pub const HOME: &str = "AGENTZ_HOME";

    /// Override the target directory for the `cargo run -p agentz --example demo` example.
    /// Example-only — not read by the library.
    pub const DEMO_DIR: &str = "AGENTZ_DEMO_DIR";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "must be prefixed")]
    fn lookup_without_prefix_panics() {
        let _ = var("HOME");
    }

    #[test]
    fn prefixed_lookup_returns_none_when_unset() {
        // Virtually guaranteed to be unset in a fresh test env.
        assert!(var("AGENTZ_DEFINITELY_NOT_SET_XYZ").is_none());
    }
}
