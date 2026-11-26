//! The colours, from docs/brand.md: green grunts at work, the amber chief's turn, on the dark
//! ground (or the light paper). `NO_COLOR` gets the plain theme: the terminal's own colours, with
//! bold and reverse for what stands out.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeName {
    Dark,
    Light,
    Plain,
}

impl ThemeName {
    /// Dark ↔ light. Plain stays plain: someone asked for no colours.
    pub fn toggled(self) -> Self {
        match self {
            ThemeName::Dark => ThemeName::Light,
            ThemeName::Light => ThemeName::Dark,
            ThemeName::Plain => ThemeName::Plain,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: ThemeName,
    pub bg: Color,
    /// The selected row, the modal.
    pub raised: Color,
    pub line: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    /// skin: the grunts, what works and passes.
    pub green: Color,
    /// accent: the chief, what waits for you.
    pub amber: Color,
    pub red: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Theme {
    pub fn new(name: ThemeName) -> Self {
        match name {
            ThemeName::Dark => Theme {
                name,
                bg: rgb(0x12160F),
                raised: rgb(0x1F261A),
                line: rgb(0x34402D),
                text: rgb(0xF3EBD2),
                muted: rgb(0xA9B3A0),
                faint: rgb(0x75806C),
                green: rgb(0x5DAE3F),
                amber: rgb(0xFFB627),
                red: rgb(0xEF6A5B),
            },
            ThemeName::Light => Theme {
                name,
                bg: rgb(0xF4F1E8),
                raised: rgb(0xE6E1D3),
                line: rgb(0xC9C2B0),
                text: rgb(0x1C2118),
                muted: rgb(0x4A5244),
                faint: rgb(0x737B6A),
                green: rgb(0x3F7F2A),
                amber: rgb(0xA86400),
                red: rgb(0xB3362A),
            },
            ThemeName::Plain => Theme {
                name,
                bg: Color::Reset,
                raised: Color::Reset,
                line: Color::Reset,
                text: Color::Reset,
                muted: Color::Reset,
                faint: Color::Reset,
                green: Color::Reset,
                amber: Color::Reset,
                red: Color::Reset,
            },
        }
    }

    pub fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.bg)
    }

    pub fn fg(&self, color: Color) -> Style {
        Style::new().fg(color)
    }

    /// Section titles: small, upper case, quiet.
    pub fn kicker(&self) -> Style {
        Style::new().fg(self.faint).add_modifier(Modifier::BOLD)
    }

    pub fn border(&self) -> Style {
        Style::new().fg(self.line)
    }

    /// The selected row. Plain has no background colour to give, so it reverses.
    pub fn selected(&self) -> Style {
        match self.name {
            ThemeName::Plain => Style::new().add_modifier(Modifier::REVERSED),
            _ => Style::new().bg(self.raised).add_modifier(Modifier::BOLD),
        }
    }
}
