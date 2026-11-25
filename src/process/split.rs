//! Splitting a command line into arguments, the way a shell would for simple cases, so commands
//! can run without a shell.

use anyhow::bail;

/// `cargo test -- "a b" 'c d'` → `["cargo", "test", "--", "a b", "c d"]`.
///
/// - Whitespace separates arguments.
/// - `"…"` and `'…'` group, and may touch other text: `--name="a b"` → `--name=a b`.
/// - Inside double quotes, `\"` is a literal `"`; every other backslash is kept as is (Windows
///   paths).
/// - An unclosed quote is an error.
pub fn split(line: &str) -> anyhow::Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    // Whether an argument has started: `""` is an (empty) argument, not nothing.
    let mut started = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\\') if chars.peek() == Some(&'"') => {
                            chars.next();
                            current.push('"');
                        }
                        Some('"') => break,
                        Some(c) => current.push(c),
                        None => bail!("unclosed \" in: {line}"),
                    }
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => bail!("unclosed ' in: {line}"),
                    }
                }
            }
            c => {
                started = true;
                current.push(c);
            }
        }
    }
    if started {
        args.push(current);
    }
    Ok(args)
}
