//! The environment as a value, so `cli::run` can be given a fake one in tests.

use std::collections::BTreeMap;

/// Environment variables. Names are case-insensitive on Windows, as the OS treats them.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// key → (name as given, value). The key is the name, upper-cased on Windows.
    vars: BTreeMap<String, (String, String)>,
}

impl Env {
    /// The current process's environment. Values that aren't valid UTF-8 are converted lossily.
    pub fn from_process() -> Self {
        Self::from_pairs(std::env::vars_os().map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        }))
    }

    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        let mut env = Self::default();
        for (name, value) in pairs {
            env.set(name, value);
        }
        env
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(&key(name)).map(|(_, value)| value.as_str())
    }

    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.vars.insert(key(&name), (name, value.into()));
    }

    /// Builder form of [`Env::set`], handy in tests.
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set(name, value);
        self
    }

    /// Every variable, as (name, value).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars
            .values()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

fn key(name: &str) -> String {
    if cfg!(windows) {
        name.to_uppercase()
    } else {
        name.to_string()
    }
}
