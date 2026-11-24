//! One integration-test binary for the whole crate (one link instead of one per file).
//! Each module is one area; acceptance commands filter by it:
//! `cargo test --all-features --test it output::`.

mod scaffold;
mod support;
