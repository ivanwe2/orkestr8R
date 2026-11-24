//! Finding an executable the way a shell would, without a shell.

use std::path::{Path, PathBuf};

use crate::env::Env;

/// The full path of `name`, searched on the `PATH` in `env` (with `PATHEXT` on Windows, so `cargo`
/// finds `cargo.exe`). A name with a directory in it is resolved against `cwd` instead.
pub fn find(name: &str, env: &Env, cwd: &Path) -> Option<PathBuf> {
    which::which_in(name, env.get("PATH"), cwd).ok()
}
