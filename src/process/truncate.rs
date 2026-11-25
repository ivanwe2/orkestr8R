//! Keeping long output short enough for the model's context.

/// What `head_tail` keeps by default: a test run's output easily runs to megabytes.
pub const HEAD_TAIL_MAX: usize = 4_000;

/// `text` if it has at most `max` chars; otherwise its first and last `max / 2` chars with
/// `…[N chars cut]…` between. The head shows what ran; the tail usually holds the error and the
/// summary. Counted in chars and cut on char boundaries, so multi-byte text is safe.
pub fn head_tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let half = max / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = text.chars().skip(count - half).collect();
    format!("{head}\n…[{} chars cut]…\n{tail}", count - 2 * half)
}
