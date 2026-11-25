//! orkestr8R: a frontier "chief" writes orders, a local "grunt" carries them out in a git worktree,
//! and orkestr8R checks the work with real commands and reports back.
//!
//! Everything lives in this library; the `ork` binary (`src/main.rs`) only calls [`cli::run`].

pub mod cli;
pub mod config;
pub mod env;
pub mod events;
pub mod git;
pub mod io;
pub mod model;
pub mod orders;
