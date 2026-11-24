//! The process exit codes. A contract: see docs/cli.md.

pub const OK: i32 = 0;
pub const ERROR: i32 = 1;
/// Waiting for a human. Reserved in v0.1.
pub const WAITING: i32 = 3;
/// Finished, but the checks failed or the grunt gave up.
pub const CHECKS_FAILED: i32 = 4;
