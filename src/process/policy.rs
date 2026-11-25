//! Which commands may run: the grunt's `run` tool and every check go through here.

use super::split::split;

/// The characters a shell would act on. Commands run without a shell, so these would do nothing
/// useful; refusing them makes that plain instead of passing `|` to a program as an argument.
const METACHARACTERS: &[char] = &[';', '&', '|', '<', '>', '$', '`', '\n', '\r'];

/// The allow-list from `allow_commands`. An entry allows any command line that starts with all of
/// its arguments, token by token: `cargo test` allows `cargo test --test it`, not `cargo run`.
#[derive(Debug, Clone, Default)]
pub struct CommandPolicy {
    entries: Vec<(String, Vec<String>)>,
}

impl CommandPolicy {
    pub fn new(allow: &[String]) -> Self {
        let entries = allow
            .iter()
            .filter_map(|entry| {
                let tokens = split(entry).ok()?;
                (!tokens.is_empty()).then(|| (entry.clone(), tokens))
            })
            .collect();
        Self { entries }
    }

    /// The allow-list, as written in the config.
    pub fn allowed(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|(entry, _)| entry.as_str())
            .collect()
    }

    /// The command's arguments if it may run, or why not (written for the model to read).
    pub fn check(&self, cmd: &str) -> Result<Vec<String>, String> {
        if let Some(c) = cmd.chars().find(|c| METACHARACTERS.contains(c)) {
            let shown = c.escape_default();
            return Err(format!(
                "refused: `{shown}` is a shell metacharacter; commands run without a shell, one at a time"
            ));
        }
        let argv = split(cmd).map_err(|err| format!("refused: {err}"))?;
        if argv.is_empty() {
            return Err("refused: the command is empty".into());
        }
        let allowed = self.entries.iter().any(|(_, tokens)| {
            argv.len() >= tokens.len() && argv.iter().zip(tokens).all(|(arg, token)| arg == token)
        });
        if allowed {
            Ok(argv)
        } else if self.entries.is_empty() {
            Err("refused: no commands are allowed (allow_commands in ork.json is empty)".into())
        } else {
            Err(format!(
                "refused: `{cmd}` is not allowed; allowed: {}",
                self.allowed().join(", ")
            ))
        }
    }
}
