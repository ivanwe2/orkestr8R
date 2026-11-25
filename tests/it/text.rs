use orkestr8r::text;

#[test]
fn numbered_lines_are_right_aligned_with_a_separator() {
    let (out, cut) = text::numbered("a\nb", 1, 10);

    assert_eq!(out, "    1 │ a\n    2 │ b\n");
    assert!(!cut);
}

#[test]
fn numbered_starts_at_the_given_line() {
    let (out, cut) = text::numbered("a\nb", 10, 10);

    assert_eq!(out, "   10 │ a\n   11 │ b\n");
    assert!(!cut);
}

#[test]
fn numbered_keeps_at_most_max_lines_and_reports_the_cut() {
    let (out, cut) = text::numbered("a\nb\nc", 1, 2);

    assert_eq!(out, "    1 │ a\n    2 │ b\n");
    assert!(cut);
}

#[test]
fn cut_keeps_short_text_whole() {
    let (out, cut) = text::cut("abc", 5);

    assert_eq!(out, "abc");
    assert!(!cut);
}

#[test]
fn cut_cuts_on_a_char_boundary() {
    let (out, cut) = text::cut("aéü", 2);

    assert_eq!(out, "aé");
    assert!(cut);
}

#[test]
fn clean_escapes_esc_lone_cr_and_del_but_keeps_newline_tab_and_crlf() {
    let out = text::clean("\u{1b}[31m\r\n\t\u{7f}\r");

    assert_eq!(out, "\\u{1b}[31m\r\n\t\\u{7f}\\r");
}
