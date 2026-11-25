//! One integration-test binary for the whole crate (one link instead of one per file).
//! Each module is one area; acceptance commands filter by it:
//! `cargo test --all-features --test it output::`.

mod ask;
mod checks;
mod config;
mod doctor;
mod git;
mod grunt;
mod model;
mod output;
// runs ork-testtool, which is only built with --features testtool
#[cfg(feature = "testtool")]
mod processes;
mod run;
mod scaffold;
mod support;
mod tools;
