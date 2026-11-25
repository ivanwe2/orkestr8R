//! Running commands: finding them, deciding whether they may run, running them. T07 adds the
//! rest; `find` is here first because `ork doctor` needs it.

pub mod find;

pub use find::find;
