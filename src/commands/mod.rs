//! User-facing commands for the CLI dispatch layer.
//!
//! Each file here implements one top-level subcommand as plain async functions
//! that take a `&dyn PsExecutor` so every branch is unit-testable on Linux.

pub mod add;
pub mod drivers;
pub mod remove;
