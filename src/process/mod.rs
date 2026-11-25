//! Running commands: finding them, deciding whether they may run, running them, and keeping
//! their output short.

pub mod find;
pub mod policy;
pub mod runner;
pub mod split;
pub mod truncate;

pub use find::find;
pub use policy::CommandPolicy;
pub use runner::{ProcessResult, ProcessRunner, ProcessSpec};
pub use split::split;
pub use truncate::{HEAD_TAIL_MAX, head_tail};
