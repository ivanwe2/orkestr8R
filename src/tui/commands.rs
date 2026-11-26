//! The slash commands: `/` opens a command line with suggestions, like a chat box. Pure: the
//! table, matching and parsing; `App` runs them.

/// One command the line knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: &'static str,
    /// What it takes, for the suggestion list ("" for nothing).
    pub args: &'static str,
    pub about: &'static str,
}

pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "apply",
        args: "[job]",
        about: "merge the job into your branch",
    },
    SlashCommand {
        name: "squash",
        args: "[job]",
        about: "the same, as one commit",
    },
    SlashCommand {
        name: "discard",
        args: "[job]",
        about: "drop the job's worktree and branch",
    },
    SlashCommand {
        name: "diff",
        args: "[job]",
        about: "the job's changes",
    },
    SlashCommand {
        name: "logs",
        args: "[job]",
        about: "the job's whole conversation with the model",
    },
    SlashCommand {
        name: "skill",
        args: "",
        about: "teach your agents to delegate (pick them)",
    },
    SlashCommand {
        name: "doctor",
        args: "",
        about: "check the config, the server and tool calling",
    },
    SlashCommand {
        name: "models",
        args: "",
        about: "which model does each role, on which server",
    },
    SlashCommand {
        name: "guide",
        args: "",
        about: "the guide your agent reads",
    },
    SlashCommand {
        name: "theme",
        args: "[dark|light]",
        about: "switch the colours",
    },
    SlashCommand {
        name: "help",
        args: "",
        about: "the keys and the commands",
    },
    SlashCommand {
        name: "quit",
        args: "",
        about: "leave",
    },
];

/// The line being typed after `/`, and which suggestion is highlighted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Palette {
    pub input: String,
    pub selected: usize,
}

impl Palette {
    /// The commands that fit what's typed: the first word as a prefix of the name, while no
    /// argument has been started.
    pub fn suggestions(&self) -> Vec<&'static SlashCommand> {
        let input = self.input.trim_start();
        let word = input.split_whitespace().next().unwrap_or("");
        let typing_args = input.contains(char::is_whitespace);
        COMMANDS
            .iter()
            .filter(|cmd| {
                if typing_args {
                    cmd.name == word
                } else {
                    cmd.name.starts_with(&word.to_ascii_lowercase())
                }
            })
            .collect()
    }

    /// The highlighted suggestion, if any.
    pub fn current(&self) -> Option<&'static SlashCommand> {
        let suggestions = self.suggestions();
        suggestions
            .get(self.selected.min(suggestions.len().saturating_sub(1)))
            .copied()
    }

    pub fn push(&mut self, c: char) {
        self.input.push(c);
        self.selected = 0;
    }

    /// Deletes a character; false when there was none (the line closes).
    pub fn backspace(&mut self) -> bool {
        self.selected = 0;
        self.input.pop().is_some()
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let n = self.suggestions().len();
        self.selected = (self.selected + 1).min(n.saturating_sub(1));
    }

    /// Tab: the highlighted command's name, and a space when it takes something.
    pub fn complete(&mut self) {
        if let Some(cmd) = self.current() {
            self.input = cmd.name.to_string();
            if !cmd.args.is_empty() {
                self.input.push(' ');
            }
            self.selected = 0;
        }
    }

    /// Enter: what to run. A name typed in full runs with what follows it; otherwise the
    /// highlighted suggestion runs without arguments (so `/ap` Enter is `/apply`).
    pub fn parse(&self) -> Parsed {
        let mut words = self.input.split_whitespace();
        let Some(word) = words.next() else {
            return match self.current() {
                Some(cmd) => Parsed::Run {
                    name: cmd.name,
                    arg: None,
                },
                None => Parsed::Nothing,
            };
        };
        let word = word.to_ascii_lowercase();
        let arg = words.next().map(String::from);
        if let Some(cmd) = COMMANDS.iter().find(|cmd| cmd.name == word) {
            return Parsed::Run {
                name: cmd.name,
                arg,
            };
        }
        match (self.current(), arg) {
            (Some(cmd), None) => Parsed::Run {
                name: cmd.name,
                arg: None,
            },
            _ => Parsed::Unknown(word),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Run {
        name: &'static str,
        arg: Option<String>,
    },
    Unknown(String),
    Nothing,
}
