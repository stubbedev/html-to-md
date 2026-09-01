//! Lowering: walk the cleaned DOM and build the typed Markdown AST. This
//! replaces the string-based serialiser; every decision the old code made on
//! rendered strings (link fallbacks, cell classification, list numbering) is
//! made here on typed nodes.

use kuchikiki::{ElementData, NodeRef};

use crate::ast::{Block, Inline, ListItem};
use crate::clean::{
    has_block_child, is_block_name, local_name_is, subtree_has_block, subtree_text,
};
use crate::render::{inlines_to_string, normalize_ws, tidy_paragraph};
use crate::table::{collect_rows, is_block_kid};
use crate::text::{decode_unicode_escapes, is_decorative_glyph, strip_tracking_params};

/// Find the shallowest heading level with non-empty text content.
pub fn min_heading_level(root: &NodeRef) -> usize {
    root.inclusive_descendants()
        .filter_map(|n| {
            let el = n.as_element()?;
            let lvl = match &*el.name.local {
                "h1" => 1usize,
                "h2" => 2,
                "h3" => 3,
                "h4" => 4,
                "h5" => 5,
                "h6" => 6,
                _ => return None,
            };
            if subtree_text(&n).trim().is_empty() {
                return None;
            }
            Some(lvl)
        })
        .min()
        .unwrap_or(7)
}

/// Lower the document starting from `<body>` (or the root fragment).
pub fn document(root: &NodeRef, shift: usize) -> Vec<Block> {
    let start = root
        .inclusive_descendants()
        .find(|n| local_name_is(n, "body"))
        .unwrap_or_else(|| root.clone());
    children_blocks(&start, shift)
}

/// Lower an element's children: group consecutive inline children into one
/// paragraph; emit block children on their own. Without this, a container
/// that mixes inline content with a block child —
/// `...for the account <strong>x</strong>.<ul>…</ul>` — scatters each text
/// run, `<strong>` and trailing `.` into separate blocks instead of keeping
/// the sentence together as one paragraph.
fn children_blocks(node: &NodeRef, shift: usize) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut run: Vec<Inline> = Vec::new();
    for c in node.children() {
        let is_block = c
            .as_element()
            .map(|el| is_block_name(&el.name.local))
            .unwrap_or(false)
            || subtree_has_block(&c);
        if is_block {
            flush_run(&mut run, &mut out);
            out.extend(block(&c, shift));
        } else {
            run.extend(inline_node(&c));
        }
    }
    flush_run(&mut run, &mut out);
    out
}

/// Serialise an accumulated run of inline sibling nodes into a single
/// paragraph block (if it has visible content) and clear the run.
fn flush_run(run: &mut Vec<Inline>, out: &mut Vec<Block>) {
    if run.is_empty() {
        return;
    }
    let s = tidy_paragraph(&inlines_to_string(run));
    if !s.is_empty() {
        out.push(Block::Paragraph {
            inlines: std::mem::take(run),
        });
    } else {
        run.clear();
    }
}

fn heading_level_of(tag: &str) -> usize {
    match tag {
        "h1" => 1,
        "h2" => 2,
        "h3" => 3,
        "h4" => 4,
        "h5" => 5,
        _ => 6,
    }
}

fn block(node: &NodeRef, shift: usize) -> Vec<Block> {
    let el = match node.as_element() {
        Some(e) => e,
        None => return vec![],
    };

    match &*el.name.local {
        "html" | "body" => children_blocks(node, shift),

        // Structural containers (and <p>): recurse when block children present, else paragraph.
        "p" | "div" | "center" | "header" | "footer" | "section" | "article" | "main" | "aside"
        | "nav" | "form" | "fieldset" => {
            if has_block_child(node) {
                children_blocks(node, shift)
            } else {
                paragraph_from_children(node)
            }
        }

        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = heading_level_of(&el.name.local)
                .saturating_sub(shift)
                .clamp(1, 6);
            // A heading is already visually prominent: emphasis markers inside
            // it are dropped, and any `<br>`-newline is folded to a single
            // space so the heading renders as one consistent line.
            let inlines = strip_emphasis(inline_children(node));
            let s = normalize_ws(&inlines_to_string(&inlines));
            if s.is_empty() {
                vec![]
            } else {
                vec![Block::Heading { level, inlines }]
            }
        }

        "hr" => vec![Block::Rule],

        "pre" => {
            // Strip per-line trailing whitespace: source <pre> often pads every
            // line out to a fixed column, which renders as ragged trailing space
            // inside the fenced block. Leading whitespace (indentation) is kept.
            let text = subtree_text(node);
            let body = text
                .lines()
                .map(|l| l.trim_end())
                .collect::<Vec<_>>()
                .join("\n");
            let body = body.trim_matches('\n').to_string();
            if body.is_empty() {
                vec![]
            } else {
                vec![Block::Code { body }]
            }
        }

        "blockquote" => {
            let inner = children_blocks(node, shift);
            if inner.is_empty() {
                vec![]
            } else {
                vec![Block::Quote { blocks: inner }]
            }
        }

        "ul" => list(node, false),
        "ol" => list(node, true),

        "table" => table_block(node),

        // Unknown/inline element at block level: recurse if any descendant is
        // a block element (handles schema.org/microdata spans that wrap the
        // entire email layout), otherwise treat as inline paragraph.
        _ => {
            if subtree_has_block(node) {
                children_blocks(node, shift)
            } else {
                paragraph_from_children(node)
            }
        }
    }
}

fn paragraph_from_children(node: &NodeRef) -> Vec<Block> {
    let inlines = inline_children(node);
    let s = tidy_paragraph(&inlines_to_string(&inlines));
    if s.is_empty() {
        vec![]
    } else {
        vec![Block::Paragraph { inlines }]
    }
}

/// Recursively drop emphasis wrappers, keeping their content. Used for
/// headings: `#` already carries the visual weight, and `# **text**` is
/// marker noise in the raw Markdown.
fn strip_emphasis(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .flat_map(|i| match i {
            Inline::Strong(inner) => strip_emphasis(inner),
            Inline::Emph(inner) => strip_emphasis(inner),
            other => vec![other],
        })
        .collect()
}

// ─── Inline lowering ─────────────────────────────────────────────────────────

fn inline_node(node: &NodeRef) -> Vec<Inline> {
    if let Some(t) = node.as_text() {
        return vec![Inline::Text(normalise_text(&t.borrow()))];
    }
    let el = match node.as_element() {
        Some(e) => e,
        None => return vec![],
    };
    match &*el.name.local {
        "a" => link_inline(node, el),
        "strong" | "b" => vec![Inline::Strong(inline_children(node))],
        "em" | "i" => vec![Inline::Emph(inline_children(node))],
        "code" => {
            let s = subtree_text(node).trim().to_string();
            if s.is_empty() {
                vec![]
            } else {
                vec![Inline::Code(s)]
            }
        }
        // `<br>` is an intentional line break — rendered as a real newline so
        // signatures, address blocks and log dumps stay tight (consecutive
        // lines) instead of reflowing onto one line or exploding into
        // blank-line-separated paragraphs. Two in a row become `\n\n` (a blank
        // line), matching the source's intent. Contexts where a raw newline is
        // harmful sanitise it: emphasis/links split or fold it so markers
        // never span lines; table cells, headings and list items flatten it.
        "br" => vec![Inline::LineBreak],
        _ => inline_children(node),
    }
}

/// Lower the children of one element into an inline run. This mirrors the old
/// `children_inline`: the marker-adjacency merge applies *within* one
/// element's children, never across separately-lowered sibling nodes.
fn inline_children(node: &NodeRef) -> Vec<Inline> {
    let parts: Vec<Inline> = node.children().flat_map(|c| inline_node(&c)).collect();
    merge_marker_runs(parts)
}

/// Adjacent same-marker emphasis with no separating whitespace — e.g.
/// `<b>So, w</b><b>atch this</b>` splitting a word mid-token — concatenates
/// to `**So, w****atch this**`. The empty `****` is noise (escaped literal
/// asterisks are `\*`, so bare `****` only ever comes from marker adjacency);
/// merging the runs yields `**So, watch this**`. Runs whose rendered side
/// carries a space (`<b>Twenty</b><b>&nbsp;minutes</b>`) are left alone — the
/// space lives outside the markers.
fn merge_marker_runs(mut parts: Vec<Inline>) -> Vec<Inline> {
    // Empty-rendering emphasis contributes nothing; drop it so adjacency is
    // literal.
    parts.retain(|p| !inlines_to_string(std::slice::from_ref(p)).is_empty());
    let mut i = 0;
    while i + 1 < parts.len() {
        let ends_marker =
            matches!(&parts[i], Inline::Strong(_)) && render_str(&parts, i).ends_with("**");
        let starts_marker = matches!(&parts[i + 1], Inline::Strong(_))
            && render_str(&parts, i + 1).starts_with("**");
        if ends_marker && starts_marker {
            let b = parts.remove(i + 1);
            let a = parts.remove(i);
            if let (Inline::Strong(mut ai), Inline::Strong(bi)) = (a, b) {
                ai.extend(bi);
                parts.insert(i, Inline::Strong(ai));
            }
            continue; // re-check the merged run against its successor
        }
        i += 1;
    }
    parts
}

fn render_str(parts: &[Inline], i: usize) -> String {
    inlines_to_string(std::slice::from_ref(&parts[i]))
}

fn link_inline(node: &NodeRef, el: &ElementData) -> Vec<Inline> {
    let text_inlines = inline_children(node);
    // A link is a single line; fold any `<br>`-newline in the text to a
    // space so `[multi\nline](url)` doesn't break the link syntax.
    let inner = normalize_ws(&inlines_to_string(&text_inlines));
    if inner.is_empty() || is_decorative_glyph(&inner) {
        return vec![];
    }
    let href = el
        .attributes
        .borrow()
        .get("href")
        .map(strip_tracking_params)
        .unwrap_or_default();
    if href.is_empty() {
        return vec![Inline::Raw(inner)];
    }
    // Garbage href: broken templates sometimes stuff text or markup
    // into the attribute (e.g. href="Legaldesk.dk<br>Njalsgade 21F..").
    // A real URL has no whitespace or angle brackets — drop the link
    // syntax and keep the visible text rather than emit a broken
    // [text](url with <br> and spaces).
    if href.contains(|c: char| c.is_whitespace() || c == '<' || c == '>') {
        return vec![Inline::Raw(inner)];
    }
    // [url](url) → bare url; [email](mailto:email) → bare email
    let bare_href = href.trim_start_matches("mailto:").trim_end_matches('/');
    if inner.trim_end_matches('/') == bare_href {
        return vec![Inline::Raw(inner)];
    }
    // If the display already contains markdown link syntax, use the plain
    // subtree text to avoid nested [[...](url)](url) which breaks parsers.
    if inner.contains("](") {
        let fallback = subtree_text(node)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fallback.is_empty() {
            return vec![];
        }
        return vec![Inline::Raw(fallback)];
    }
    vec![Inline::Link { text: inner, href }]
}

/// Decode literal unicode escape sequences and collapse whitespace runs,
/// matching the browser's whitespace collapsing behaviour. Markdown escaping
/// happens later, at render time.
fn normalise_text(s: &str) -> String {
    let s = decode_unicode_escapes(s);
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{00A0}') {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            prev_space = false;
            out.push(c);
        }
    }
    out
}

// ─── List lowering ───────────────────────────────────────────────────────────

fn list(list_node: &NodeRef, ordered: bool) -> Vec<Block> {
    let mut items: Vec<ListItem> = Vec::new();
    let mut n = 0usize;

    for child in list_node.children() {
        if !local_name_is(&child, "li") {
            continue;
        }
        n += 1;
        // Collect inline text and any nested sub-lists from li's children.
        let mut inlines: Vec<Inline> = Vec::new();
        let mut sub_lists: Vec<Block> = Vec::new();

        for kid in child.children() {
            if local_name_is(&kid, "ul") {
                sub_lists.extend(list(&kid, false));
            } else if local_name_is(&kid, "ol") {
                sub_lists.extend(list(&kid, true));
            } else if is_block_kid(&kid) {
                // <p>/<div> inside li: gather as inline text
                let s = inlines_to_string(&inline_children(&kid));
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    inlines.push(Inline::Raw(trimmed.to_string()));
                }
            } else {
                inlines.extend(inline_node(&kid));
            }
        }

        let text_empty = normalize_ws(&inlines_to_string(&inlines)).is_empty();
        if text_empty && sub_lists.is_empty() {
            continue;
        }
        items.push(ListItem {
            number: n,
            inlines,
            sub_lists,
        });
    }

    if items.is_empty() {
        vec![]
    } else {
        vec![Block::List { ordered, items }]
    }
}

// ─── Table lowering ──────────────────────────────────────────────────────────

fn table_block(table: &NodeRef) -> Vec<Block> {
    let rows_dom = collect_rows(table);
    if rows_dom.len() < 2 {
        return vec![];
    }

    // Lower every cell to inline Markdown.
    let mut parsed: Vec<Vec<Vec<Inline>>> = rows_dom
        .iter()
        .map(|tr| {
            tr.children()
                .filter(|n| local_name_is(n, "td") || local_name_is(n, "th"))
                // A `<br>`-newline inside a cell would break the table row;
                // collapse all whitespace runs (incl. those newlines) to single
                // spaces. Links never contain spaces, so this can't split one.
                .map(|cell| inline_children(&cell))
                .collect()
        })
        .collect();

    let ncols = parsed.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return vec![];
    }

    // Drop columns where every cell is empty.
    let keep: Vec<usize> = (0..ncols)
        .filter(|&c| {
            parsed.iter().any(|row| {
                row.get(c)
                    .map(|cell| !normalize_ws(&inlines_to_string(cell)).is_empty())
                    .unwrap_or(false)
            })
        })
        .collect();
    if keep.len() < ncols && !keep.is_empty() {
        parsed = parsed
            .into_iter()
            .map(|row| {
                keep.iter()
                    .map(|&c| row.get(c).cloned().unwrap_or_default())
                    .collect()
            })
            .collect();
    }
    let ncols = parsed.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return vec![];
    }

    vec![Block::Table { rows: parsed }]
}
