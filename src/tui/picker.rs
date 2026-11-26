//! The agent picker: which agents do you use? ork installs the skill for the ones ticked. The
//! TUI shows it on its first run and on `/skill`; `ork skill install` with no agents named shows
//! it inline in the terminal. Pure: rows and keys.

use ratatui::crossterm::event::KeyCode;

use crate::skill::{Agent, Installed, Target};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub rows: Vec<PickerRow>,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    pub agent: Agent,
    pub found: bool,
    pub installed: Installed,
    pub checked: bool,
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// Still choosing.
    Choosing,
    /// Esc: nothing installed.
    Cancel,
    /// Enter: install for these.
    Install(Vec<Agent>),
}

impl Picker {
    /// Agents found on this machine, and those that have the skill already, start ticked. Nothing
    /// is installed until Enter.
    pub fn new(targets: &[Target]) -> Self {
        let rows: Vec<PickerRow> = targets
            .iter()
            .map(|target| PickerRow {
                agent: target.agent,
                found: target.found,
                installed: target.installed,
                checked: target.found || target.installed != Installed::No,
            })
            .collect();
        let cursor = rows.iter().position(|row| row.checked).unwrap_or(0);
        Picker { rows, cursor }
    }

    pub fn chosen(&self) -> Vec<Agent> {
        self.rows
            .iter()
            .filter(|row| row.checked)
            .map(|row| row.agent)
            .collect()
    }

    pub fn on_key(&mut self, code: KeyCode) -> Picked {
        let last = self.rows.len().saturating_sub(1);
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Char(' ' | 'x') => {
                if let Some(row) = self.rows.get_mut(self.cursor) {
                    row.checked = !row.checked;
                }
            }
            KeyCode::Enter => {
                // Nothing ticked: Enter means the one under the cursor.
                let chosen = self.chosen();
                if chosen.is_empty() {
                    return match self.rows.get(self.cursor) {
                        Some(row) => Picked::Install(vec![row.agent]),
                        None => Picked::Cancel,
                    };
                }
                return Picked::Install(chosen);
            }
            KeyCode::Esc | KeyCode::Char('q') => return Picked::Cancel,
            _ => {}
        }
        Picked::Choosing
    }
}

/// The picker's name for an agent: `other` is everything that reads the shared folder.
pub fn label(agent: Agent) -> &'static str {
    match agent {
        Agent::Other => "Any other agent",
        agent => agent.name(),
    }
}
