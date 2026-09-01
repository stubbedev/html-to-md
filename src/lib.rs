//! aerc filters for vendor-noisy email → clean, terminal-legible Markdown.
//!
//! Library crate; the single `html-to-md` binary is a thin stdin/stdout
//! wrapper that picks a conversion by flag or sniffs it via [`detect`].
//!
//! `--html` (text/html) pipeline:
//!   1. Pre-process: strip non-comment IE conditionals (<![if …]>…<![endif]>)
//!      before parsing so Outlook bullet spans don't leak into the DOM.
//!   2. Parse with html5ever (via kuchikiki).
//!   3. DOM surgery: strip comments (catches <!--[if mso]> Outlook blocks),
//!      drop namespaced/non-text/hidden elements, normalise text nodes, bubble
//!      <br> to block level, flatten layout tables, demote stat headings,
//!      collapse flex rows, drop decorative/empty anchors.
//!   4. Lower the cleaned DOM into a typed Markdown AST — every heuristic that
//!      used to re-parse rendered strings with regexes is expressed on typed
//!      nodes instead.
//!   5. Render the AST to Markdown in one deterministic pass. Paragraphs are
//!      emitted unwrapped, one line per paragraph; the pager owns
//!      soft-wrapping.
//!
//! `--plain` dewraps RFC 3676 format=flowed parts (plain.rs); `--calendar`
//! renders text/calendar parts (calendar.rs).

mod ast;
pub mod calendar;
pub mod clean;
mod lower;
pub mod plain;
mod render;
pub mod table;
pub mod text;

/// Conversion mode: selected by the binary's flags, or sniffed by [`detect`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Html,
    Plain,
    Calendar,
}

/// Sniff a mail part. ICS declares itself (`BEGIN:VCALENDAR`, after any BOM
/// and whitespace); HTML is detected by common tags in the first 2 KB;
/// everything else is treated as plain text.
pub fn detect(input: &str) -> Format {
    let head = input
        .trim_start()
        .trim_start_matches('\u{FEFF}')
        .trim_start();
    if head.starts_with("BEGIN:VCALENDAR") {
        return Format::Calendar;
    }
    let head: String = input
        .chars()
        .take(2048)
        .collect::<String>()
        .to_ascii_lowercase();
    if [
        "<!doctype html",
        "<html",
        "<body",
        "<div",
        "<p>",
        "<p ",
        "<br",
        "<table",
        "<span",
        "<strong",
        "<em>",
    ]
    .iter()
    .any(|t| head.contains(t))
    {
        Format::Html
    } else {
        Format::Plain
    }
}

/// Convert an HTML email body to Markdown. Paragraphs are emitted unwrapped;
/// the pager owns soft-wrapping.
pub fn convert(html: &str) -> String {
    let doc = clean::clean_doc(html);
    let shift = lower::min_heading_level(&doc).saturating_sub(1);
    let blocks = lower::document(&doc, shift);
    let blocks = ast::drop_empty_sections(blocks);
    let blocks = ast::compress_heading_levels(blocks);
    let blocks = ast::join_link_rows(blocks);
    render::document(&blocks)
        .trim_start_matches('\n')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats() {
        assert!(matches!(
            detect("<!DOCTYPE html><html><body>"),
            Format::Html
        ));
        assert!(matches!(detect("<div>fragment</div>"), Format::Html));
        assert!(matches!(
            detect("\n  \u{FEFF}BEGIN:VCALENDAR\r\nVERSION:2.0"),
            Format::Calendar
        ));
        assert!(matches!(detect("plain prose\nover lines\n"), Format::Plain));
        // Tags past the sniff window don't flip prose to HTML.
        let tail = format!("{}<div>", "x".repeat(3000));
        assert!(matches!(detect(tail.as_str()), Format::Plain));
        // A stray `<` or `<b>` in prose is not HTML.
        assert!(matches!(
            detect("a < b and <b>bold</b> dreams"),
            Format::Plain
        ));
    }
}
