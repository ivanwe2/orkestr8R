//! Text helpers shared by `ork ask`, the grunt's tools and its first message.

/// Lines of `text` numbered from `first`, as `{n:>5} │ line`, at most `max_lines` of them.
/// Returns the numbered text and whether lines were left out.
///
/// Numbered lines let the model cite `file:line`, and let it aim `read_file` ranges and
/// `edit_file` matches precisely.
pub fn numbered(text: &str, first: usize, max_lines: usize) -> (String, bool) {
    let mut out = String::new();
    let mut lines = text.lines();
    for (i, line) in lines.by_ref().take(max_lines).enumerate() {
        out.push_str(&format!("{:>5} │ {line}\n", first + i));
    }
    (out, lines.next().is_some())
}

/// `text` cut to at most `max` chars (on a char boundary), and whether it was cut.
pub fn cut(text: &str, max: usize) -> (&str, bool) {
    match text.char_indices().nth(max) {
        Some((at, _)) => (&text[..at], true),
        None => (text, false),
    }
}
