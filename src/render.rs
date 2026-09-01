//! AST → Markdown rendering. One deterministic pass: escape source text once,
//! emit structural markers, pad table columns from typed widths. No regexes,
//! no re-parsing of emitted output.

use crate::ast::{Block, Inline, ListItem};

/// Render a whole document: blocks joined by blank lines, blank runs
/// collapsed (double `<br>` can put one inside a paragraph).
pub fn document(blocks: &[Block]) -> String {
    let parts: Vec<String> = blocks
        .iter()
        .map(render_block)
        .filter(|s| !s.is_empty())
        .collect();
    collapse_blank_runs(&parts.join("\n\n"))
}

/// Collapse whitespace between tokens to single spaces (browser whitespace
/// collapsing, applied to rendered strings in whitespace-hostile contexts:
/// table cells, list items, link display).
pub(crate) fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Render inlines to their Markdown string.
pub(crate) fn inlines_to_string(inlines: &[Inline]) -> String {
    inlines.iter().map(render_inline).collect()
}

/// Trim a multi-line paragraph (one whose `<br>`s became newlines): strip
/// surrounding whitespace on every line so leading spaces from source text
/// nodes don't survive as ragged indentation, while preserving blank lines
/// (from `<br><br>`) between the kept lines. A paragraph that reduces to a
/// single ASCII punctuation char is dropped: it is separator residue, e.g.
/// the ` . ` left between two `<img>` that were removed as decorative media.
pub(crate) fn tidy_paragraph(s: &str) -> String {
    let out = if !s.contains('\n') {
        s.trim().to_string()
    } else {
        s.lines()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    };
    if out.len() == 1 && out.as_bytes()[0].is_ascii_punctuation() {
        return String::new();
    }
    out
}

fn render_block(block: &Block) -> String {
    match block {
        Block::Paragraph { inlines } => tidy_paragraph(&inlines_to_string(inlines)),
        Block::Heading { level, inlines } => {
            // A heading is a single line; fold any `<br>`-newline to a space.
            let s = normalize_ws(&inlines_to_string(inlines));
            if s.is_empty() {
                String::new()
            } else {
                format!("{} {}", "#".repeat(*level), s)
            }
        }
        Block::Rule => "---".to_string(),
        Block::Code { body } => format!("```\n{body}\n```"),
        Block::Quote { blocks } => {
            let inner: Vec<String> = blocks
                .iter()
                .map(render_block)
                .filter(|s| !s.is_empty())
                .collect();
            if inner.is_empty() {
                return String::new();
            }
            inner
                .join("\n\n")
                .lines()
                .map(|l| {
                    if l.is_empty() {
                        ">".to_string()
                    } else {
                        format!("> {l}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Block::List { ordered, items } => render_list(items, *ordered, 0),
        Block::Table { rows } => render_table(rows),
    }
}

fn render_list(items: &[ListItem], ordered: bool, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut lines: Vec<String> = Vec::new();
    for item in items {
        // A list item is one logical line; fold any `<br>`-newline to a space
        // so a continuation doesn't escape the bullet's indentation.
        let text = normalize_ws(&inlines_to_string(&item.inlines));
        if text.is_empty() && item.sub_lists.is_empty() {
            continue;
        }
        let marker = if ordered {
            format!("{}. ", item.number)
        } else {
            "- ".to_string()
        };
        lines.push(format!("{indent}{marker}{text}"));
        for sub in &item.sub_lists {
            // Sub-list already carries its own depth-based indent.
            if let Block::List {
                ordered: sub_ordered,
                items: sub_items,
            } = sub
            {
                for line in render_list(sub_items, *sub_ordered, depth + 1).lines() {
                    lines.push(line.to_string());
                }
            }
        }
    }
    lines.join("\n")
}

fn render_table(rows: &[Vec<Vec<Inline>>]) -> String {
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return String::new();
    }

    // Minimal framing: single space between the pipes and the cells, no
    // column padding. Tables stay narrow and ragged rows don't stretch the
    // frame; terminal concealers don't align raw columns anyway.
    let mut lines: Vec<String> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let mut line = String::from("|");
        for c in 0..ncols {
            let cell = match row.get(c) {
                Some(cell) => normalize_ws(&inlines_to_string(cell)),
                None => String::new(),
            };
            line.push(' ');
            line.push_str(&cell);
            line.push_str(" |");
        }
        lines.push(line);
        if i == 0 {
            let mut sep = String::from("|");
            for _ in 0..ncols {
                sep.push_str(" - |");
            }
            lines.push(sep);
        }
    }
    lines.join("\n")
}

fn render_inline(inline: &Inline) -> String {
    match inline {
        Inline::Text(s) => escape_markdown(s),
        Inline::Raw(s) => s.clone(),
        Inline::Code(s) => format!("`{s}`"),
        Inline::Strong(inner) => emphasis(inner, "**"),
        Inline::Emph(inner) => emphasis(inner, "*"),
        Inline::Link { text, href } => format!("[{text}]({href})"),
        Inline::LineBreak => "\n".to_string(),
    }
}

/// Wrap inline content in an emphasis marker (`**` or `*`), keeping any leading
/// or trailing whitespace *outside* the markers. HTML often splits a phrase
/// across adjacent `<strong>`/`<b>` runs where the only separating space lives
/// at a marker boundary (e.g. `<b>Twenty</b><b>&nbsp;minutes</b>`); trimming it
/// away would fuse the words (`**Twenty****minutes**`).
fn emphasis(inner: &[Inline], marker: &str) -> String {
    let s = inlines_to_string(inner);
    // A `<br>` inside the emphasis (`<b>line1<br>line2</b>`) leaves a newline in
    // `s`; wrap each line on its own so the markers never span a line break
    // (`**line1**\n**line2**`), which would otherwise render the literal `**`.
    if s.contains('\n') {
        return s
            .split('\n')
            .map(|line| {
                let t = line.trim();
                if t.is_empty() {
                    String::new()
                } else {
                    format!("{marker}{t}{marker}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let t = s.trim();
    if t.is_empty() {
        return String::new();
    }
    let lead = if s.starts_with(|c: char| c.is_whitespace()) {
        " "
    } else {
        ""
    };
    let trail = if s.ends_with(|c: char| c.is_whitespace()) {
        " "
    } else {
        ""
    };
    format!("{lead}{marker}{t}{marker}{trail}")
}

/// Escape characters that could create unintended Markdown syntax. Whitespace
/// collapsing and unicode-escape decoding happened at lowering; this only
/// backslash-escapes the structural characters.
fn escape_markdown(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn collapse_blank_runs(s: &str) -> String {
    let mut out = s.to_owned();
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}
