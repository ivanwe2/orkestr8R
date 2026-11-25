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

/// `text` made safe for a terminal: control characters (ESC, a lone CR, DEL, the C1 range with
/// its CSI, …) are escaped, so text from the model can't drive the terminal (move the cursor,
/// rewrite a line, change colours). Newlines, tabs and `\r\n` stay, so CRLF text reads normally.
/// JSON output needs no such step: serde_json escapes control characters itself.
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let keep =
            !c.is_control() || c == '\n' || c == '\t' || (c == '\r' && chars.peek() == Some(&'\n'));
        if keep {
            out.push(c);
        } else {
            out.extend(c.escape_default());
        }
    }
    out
}

/// `text` cut to at most `max` chars (on a char boundary), and whether it was cut.
pub fn cut(text: &str, max: usize) -> (&str, bool) {
    match text.char_indices().nth(max) {
        Some((at, _)) => (&text[..at], true),
        None => (text, false),
    }
}

/// `text` shortened to at most `max` chars: unchanged when it fits, otherwise its first
/// `max - 1` chars followed by `…`. Counts chars, not bytes.
pub fn ellipsize(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        let head: String = text.chars().take(max - 1).collect();
        format!("{head}…")
    } else {
        text.to_string()
    }
}
