//! RFC 3676 `format=flowed` dewrapping for `text/plain` parts.
//!
//! Flowed mail soft-wraps paragraphs at ~78 columns: a line ending in a
//! single space continues on the next line, a line without one is a hard
//! break. Quoting is `>` markers (each optionally followed by one space), a
//! `-- ` line starts the signature. This module reflows soft-wrapped
//! paragraphs into one line per paragraph — the same dialect the HTML path
//! emits — and renders quote levels as Markdown blockquotes so the pager
//! styles them.
//!
//! Whitespace-stuffing (the single leading space senders add to protect
//! `From ` lines) is deleted; deeper indents survive.

/// Render a text/plain part: passthrough unless `flowed`.
pub fn render(input: &str, flowed: bool) -> String {
    if flowed {
        render_flowed(input)
    } else {
        input.to_string()
    }
}

pub fn render_flowed(input: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    // Accumulated dewrapped text of the current same-depth paragraph run.
    let mut cur = String::new();
    let mut cur_depth: Option<usize> = None;
    let mut in_sig = false;

    for raw in input.lines() {
        if is_conditional_noise(raw) {
            continue;
        }
        if !in_sig && raw.trim_end() == "--" {
            flush(&mut cur, &mut cur_depth, &mut out);
            in_sig = true;
            out.push("-- ".to_string());
            continue;
        }
        if in_sig {
            out.push(raw.to_string());
            continue;
        }

        let (depth, content) = split_quote_depth(raw);
        if depth != cur_depth.unwrap_or(usize::MAX) {
            // Different quote depth ends the paragraph.
            flush(&mut cur, &mut cur_depth, &mut out);
            cur_depth = Some(depth);
        }
        if raw.is_empty() {
            // A blank line ends the paragraph and is the paragraph separator.
            flush(&mut cur, &mut cur_depth, &mut out);
            cur_depth = Some(0);
            continue;
        }
        let trailing = content.chars().rev().take_while(|&c| c == ' ').count();
        let soft = trailing == 1;
        if soft {
            // The trailing space is the join separator; don't let a wrapped
            // continuation's own leading spaces pile onto it.
            let joined = if cur.ends_with(' ') {
                content.trim_start_matches(' ')
            } else {
                content
            };
            cur.push_str(joined);
        } else {
            cur.push_str(content.trim_end_matches(' '));
            flush(&mut cur, &mut cur_depth, &mut out);
            cur_depth = Some(depth);
        }
    }
    flush(&mut cur, &mut cur_depth, &mut out);

    // Paragraphs are blank-separated; collapse runs and trim the edges.
    let mut joined = out.join("\n");
    while joined.contains("\n\n\n") {
        joined = joined.replace("\n\n\n", "\n\n");
    }
    joined.trim().to_string()
}

/// Emit the accumulated paragraph as one line (plus the blank separator),
/// prefixed with the quote level.
fn flush(cur: &mut String, cur_depth: &mut Option<usize>, out: &mut Vec<String>) {
    if !cur.trim().is_empty() {
        let depth = cur_depth.unwrap_or(0);
        let line = cur.trim_end();
        if depth == 0 {
            out.push(line.to_string());
        } else {
            out.push(format!("{} {}", ">".repeat(depth), line));
        }
        out.push(String::new());
    }
    cur.clear();
    *cur_depth = None;
}

/// Outlook conditional comments that vendors leak into the plain part
/// (`<!--[if !mso]><!-->`, `<![endif]-->`, …) are never meaningful content.
fn is_conditional_noise(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("<!--[if")
        || t.starts_with("<![endif")
        || t.starts_with("<!--<![endif")
        || t.starts_with("<!--[endif")
        || t == "<!-->"
}

/// Count quote markers: each `>` consumes one level; an optional single space
/// after a `>` only counts as the inter-marker separator when another `>`
/// follows. One optional space after the final marker is part of the content
/// separator; deeper indentation survives.
fn split_quote_depth(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line;
    while rest.starts_with('>') {
        rest = &rest[1..];
        if rest.starts_with(' ') && rest[1..].starts_with('>') {
            rest = &rest[1..];
        }
        depth += 1;
    }
    if rest.starts_with(' ') {
        rest = &rest[1..];
    }
    // Whitespace-stuffing: a sender-prepended single space protecting `From `
    // lines. One space is deleted; multi-space indents survive.
    if rest.starts_with(' ') && !rest[1..].starts_with(' ') && !rest[1..].is_empty() {
        rest = &rest[1..];
    }
    (depth, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_soft_wrapped_paragraphs() {
        let input = "The quick brown fox \njumped over \nthe lazy dog.\nSecond paragraph \nhere.\n";
        let out = render_flowed(input);
        assert_eq!(
            out,
            "The quick brown fox jumped over the lazy dog.\n\nSecond paragraph here."
        );
    }

    #[test]
    fn renders_quote_levels_as_blockquotes() {
        let input = "> quoted line one \n> quoted line two.\nreply body\n>> deeper \n>> note.\n";
        let out = render_flowed(input);
        assert_eq!(
            out,
            "> quoted line one quoted line two.\n\nreply body\n\n>> deeper note."
        );
    }

    #[test]
    fn hard_lines_end_paragraphs() {
        let input = "line one\nline two  \nline three\n";
        // Two trailing spaces = hard break even though the line ends in a space.
        let out = render_flowed(input);
        assert_eq!(out, "line one\n\nline two\n\nline three");
    }

    #[test]
    fn blank_lines_separate_paragraphs() {
        let input = "one \ntwo\n\nthree\n";
        assert_eq!(render_flowed(input), "one two\n\nthree");
    }

    #[test]
    fn signature_stays_verbatim_and_tight() {
        let input = "hello \nworld.\n-- \nAlex\nsent from my terminal \n";
        let out = render_flowed(input);
        assert_eq!(out, "hello world.\n\n-- \nAlex\nsent from my terminal");
    }

    #[test]
    fn strips_whitespace_stuffing_keeps_indents() {
        let input = " leading space kept minus stuffing \n  intentional indent\nFrom protected \n";
        let out = render_flowed(input);
        // " leading..." loses exactly one stuffing space; the paragraph is
        // soft-joined so the leading text starts the line.
        assert_eq!(
            out,
            "leading space kept minus stuffing intentional indent\n\nFrom protected"
        );
    }

    #[test]
    fn depth_change_breaks_paragraph() {
        let input = "para one \n> quote jumps in \n> and stays.\n";
        let out = render_flowed(input);
        assert_eq!(out, "para one\n\n> quote jumps in and stays.");
    }

    #[test]
    fn passthrough_when_not_flowed() {
        let input = "a \nb\n";
        assert_eq!(render(input, false), input);
        assert_eq!(render(input, true), "a b");
    }

    #[test]
    fn conditional_comment_noise_dropped() {
        let input =
            "<!--[if !mso]><!-->\nreal text \n<!--[if false]><!-->\nmore.\n<!--<![endif]-->\n";
        assert_eq!(render_flowed(input), "real text more.");
    }
}
