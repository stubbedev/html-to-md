//! aerc text/html filter — clean up vendor-noisy HTML, then convert to Markdown.
//!
//! Single binary, pipeline:
//!   1. Pre-process: strip non-comment IE conditionals (<![if …]>…<![endif]>)
//!      before parsing so Outlook bullet spans don't leak into the DOM.
//!   2. Parse with html5ever (via kuchikiki).
//!   3. DOM surgery: strip comments (catches <!--[if mso]> Outlook blocks),
//!      drop namespaced/non-text/hidden elements, normalise text nodes, bubble
//!      <br> to block level, flatten layout tables, demote stat headings,
//!      collapse flex rows, drop decorative/empty anchors.
//!   4. Walk the cleaned DOM with a custom serialiser → Markdown (no htmd, no
//!      regex post-processing).
//!   5. Hard-wrap lines, collapse blank runs.
use std::error::Error;
use std::io::{self, Read, Write};
use std::sync::OnceLock;

use kuchikiki::traits::*;
use kuchikiki::{parse_html, NodeRef};
use regex::Regex;

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    // Strip non-comment IE conditionals before parsing so Outlook bullet
    // spans (<![if !supportLists]><span>·</span><![endif]>) and other
    // Outlook-only blocks don't appear in the DOM as regular text nodes.
    // Standard <!--[if mso]>…<![endif]--> comment-form conditionals are
    // already handled by html5ever's bogus-comment parser + strip_comments.
    let input = strip_ie_conditionals(&input);
    let doc = parse_html().one(input);

    strip_comments(&doc);
    // HTML5 parsing keeps Outlook/Word namespaced tags (o:p, w:WordDocument,
    // v:shape, …) as elements with a literal colon in `local`; the parser
    // doesn't populate `prefix` for non-XHTML input. Match on either.
    drop_elements(&doc, |el| {
        el.name.prefix.is_some() || el.name.local.contains(':')
    });
    // Responsive emails duplicate content: one version visible on desktop,
    // one on mobile, toggled via CSS. Since we strip stylesheets, both
    // render. Drop any element whose inline style hides it.
    drop_elements(&doc, |el| {
        el.attributes
            .borrow()
            .get("style")
            .map(|s| {
                let s = s.to_ascii_lowercase();
                s.contains("display:none")
                    || s.contains("display: none")
                    || s.contains("visibility:hidden")
                    || s.contains("visibility: hidden")
            })
            .unwrap_or(false)
    });
    drop_elements(&doc, |el| {
        matches!(
            &*el.name.local,
            "head"
                | "style"
                | "script"
                | "iframe"
                | "img"
                | "colgroup"
                | "col"
                | "figure"
                | "picture"
                | "source"
                | "svg"
                | "canvas"
                | "video"
                | "audio"
                | "area"
                | "map"
                | "noscript"
        )
    });
    // Must run before drop_empty_anchors so anchors padded with ZWSPs etc.
    // become text-empty.
    normalise_text_nodes(&doc);
    flatten_link_text(&doc);
    unwrap_punctuation_emphasis(&doc);
    demote_stat_headings(&doc);
    inline_flex_row_divs(&doc);
    flatten_tables(&doc);
    // Marketing emails wrap a brand logo in <a href="…"><img></a>; once we
    // drop the <img>, the anchor has no visible text. Strip those empty anchors.
    drop_empty_anchors(&doc);

    let md = to_markdown(&doc, wrap_width());
    let md = md.trim_start_matches('\n').to_string();

    io::stdout().write_all(md.as_bytes())?;
    Ok(())
}

// ─── DOM helpers ────────────────────────────────────────────────────────────

/// Replace zero-width / format chars with nothing and NBSP-class spaces with
/// a regular space inside every text node. Done in-place on the live tree so
/// later passes (drop_empty_anchors, table-cell blankness checks) see the
/// cleaned text.
fn normalise_text_nodes(root: &NodeRef) {
    let texts: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| n.as_text().is_some())
        .collect();
    for t in texts {
        let txt = t.as_text().unwrap();
        let cleaned = clean_invisibles(&txt.borrow());
        *txt.borrow_mut() = cleaned;
    }
}

fn clean_invisibles(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // Zero-width / format characters that emails use as preview-text
            // padding. Drop entirely.
            '\u{00AD}' // soft hyphen
            | '\u{034F}' // combining grapheme joiner (Klaviyo et al.)
            | '\u{061C}' // arabic letter mark
            | '\u{115F}' // hangul choseong filler
            | '\u{1160}' // hangul jungseong filler
            | '\u{17B4}' // khmer vowel inherent aq
            | '\u{17B5}' // khmer vowel inherent aa
            | '\u{180E}' // mongolian vowel separator
            | '\u{200B}' // zero-width space
            | '\u{200C}' // ZWNJ
            | '\u{200D}' // ZWJ
            | '\u{200E}' // LRM
            | '\u{200F}' // RLM
            | '\u{202A}'..='\u{202E}' // bidi formatting
            | '\u{2060}' // word joiner
            | '\u{2061}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}' // bidi isolates
            | '\u{3164}' // hangul filler
            | '\u{FE00}'..='\u{FE0F}' // variation selectors
            | '\u{FEFF}' // BOM / zero-width nbsp
            | '\u{FFA0}' // halfwidth hangul filler
            | '\u{E0020}'..='\u{E007F}' // tag characters
            => {}
            // NBSP-class horizontal whitespace → plain space so post-processing
            // can collapse runs and trim() works as expected.
            '\u{00A0}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

fn strip_comments(root: &NodeRef) {
    let comments: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| n.as_comment().is_some())
        .collect();
    for c in comments {
        c.detach();
    }
}

fn drop_elements<F>(root: &NodeRef, predicate: F)
where
    F: Fn(&kuchikiki::ElementData) -> bool,
{
    let victims: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| n.as_element().map(&predicate).unwrap_or(false))
        .collect();
    for v in victims {
        v.detach();
    }
}

fn drop_empty_anchors(root: &NodeRef) {
    let anchors: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| local_name_is(n, "a"))
        .collect();
    for a in anchors {
        let text = subtree_text(&a);
        let trimmed = text.trim();
        if trimmed.is_empty() || is_decorative_glyph(trimmed) {
            a.detach();
        }
    }
}

/// Single non-alphanumeric character links (›, », →, ▸, etc.) are decorative
/// icon links that add noise in a text/terminal reader.
fn is_decorative_glyph(s: &str) -> bool {
    let mut chars = s.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_alphanumeric())
}

fn subtree_text(n: &NodeRef) -> String {
    let mut buf = String::new();
    for d in n.inclusive_descendants() {
        if let Some(t) = d.as_text() {
            buf.push_str(&t.borrow());
        }
    }
    buf
}

/// Unwrap emphasis tags whose textual content is purely punctuation (≤ 3
/// chars, no letters/digits). Sentry tag rows wrap a literal `=` in `<em>`,
/// which htmd serialises as `*\=*` — italic markers around a backslash-
/// escaped equals — and renders as visible noise. Same for `<strong>:</strong>`
/// and similar single-symbol decorations.
fn unwrap_punctuation_emphasis(root: &NodeRef) {
    let candidates: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| {
            n.as_element()
                .map(|el| {
                    matches!(
                        &*el.name.local,
                        "em" | "i" | "strong" | "b" | "u" | "mark" | "small"
                    )
                })
                .unwrap_or(false)
        })
        .collect();
    for el in candidates {
        let text = subtree_text(&el);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            // Whitespace-only emphasis (`<em> </em>`) often glues two
            // adjacent inlines together. Detaching loses the space and
            // mashes neighbours. Merge the space into an adjacent text
            // sibling so flatten_tables' is_blank filter (which discards
            // standalone whitespace text nodes) can't strip it.
            if !text.is_empty() {
                merge_separator_space(&el);
            }
            el.detach();
            continue;
        }
        if trimmed.chars().count() <= 3
            && trimmed
                .chars()
                .all(|c| !c.is_alphanumeric() && !c.is_whitespace())
        {
            let kids: Vec<NodeRef> = el.children().collect();
            for k in kids {
                k.detach();
                el.insert_before(k);
            }
            el.detach();
        }
    }
}

/// Push a separator space onto an adjacent text sibling of `el`. Prefers the
/// previous sibling (so a leading space doesn't accidentally start a new
/// "blank" line); falls back to the next sibling. If neither sibling is a
/// text node, inserts a standalone text node (which will survive non-table
/// contexts).
fn merge_separator_space(el: &NodeRef) {
    if let Some(prev) = el.previous_sibling() {
        if let Some(t) = prev.as_text() {
            let mut s = t.borrow_mut();
            if !s.ends_with(' ') {
                s.push(' ');
            }
            return;
        }
    }
    if let Some(next) = el.next_sibling() {
        if let Some(t) = next.as_text() {
            let mut s = t.borrow_mut();
            if !s.starts_with(' ') {
                s.insert(0, ' ');
            }
            return;
        }
    }
    el.insert_before(NodeRef::new_text(" "));
}

/// Sentry's weekly digest dumps headline stats as `<h1>471k</h1>` purely for
/// visual scale. Treating those as h1 means the whole document inherits a
/// numeric heading level and the stat itself renders as an oversized header.
/// Detect short numeric-only heading content (≤ 12 chars, digits + optional
/// unit suffix like `k`, `M`, `ms`) and rewrite the element to a paragraph
/// with bold so the level normaliser ignores it.
fn demote_stat_headings(root: &NodeRef) {
    let candidates: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| {
            n.as_element()
                .map(|el| matches!(&*el.name.local, "h1" | "h2" | "h3" | "h4" | "h5" | "h6"))
                .unwrap_or(false)
        })
        .collect();
    for h in candidates {
        let text = subtree_text(&h);
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 12 {
            continue;
        }
        if !is_stat_text(trimmed) {
            continue;
        }
        // Replace with `<p><strong>…</strong></p>` so the bold stat keeps
        // visible scale without skewing the heading hierarchy and remains a
        // block-level node so flatten_tables won't glue it to a neighbouring
        // inline `<a>View All …</a>` link.
        let para = parse_html()
            .one("<p><strong></strong></p>")
            .descendants()
            .find(|n| local_name_is(n, "p"))
            .expect("kuchikiki always materialises the parsed <p>");
        let strong = para
            .first_child()
            .expect("freshly-parsed <p> contains a <strong>");
        let kids: Vec<NodeRef> = h.children().collect();
        for k in kids {
            k.detach();
            strong.append(k);
        }
        h.insert_before(para);
        h.detach();
    }
}

fn is_stat_text(s: &str) -> bool {
    let mut saw_digit = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else if !matches!(c, '.' | ',' | ' ' | 'k' | 'K' | 'M' | 'B' | 'm' | 's' | 'µ' | 'h') {
            return false;
        }
    }
    saw_digit
}

/// Sentry's "Issues with the most errors" / "Most frequent transactions"
/// rows are CSS flex containers (`<div style="display: flex; ...">`) wrapping
/// 3 inline-feeling children: a count, a link block, a status pill. htmd
/// treats every `<div>` as a paragraph, so each row explodes into 4–5
/// blank-separated paragraphs. Detect flex-row parents and unwrap their
/// `<div>` children that hold only inline content, so the row collapses to
/// a single paragraph joined by spaces.
fn inline_flex_row_divs(root: &NodeRef) {
    // Sentry rows are flex containers with a small handful of direct
    // children (a count, a link wrapper, a status pill — usually 3–4).
    // Marketing emails that use flex for full-page layout typically have
    // 1 main column wrapping thousands of nested elements. Use direct
    // child count as the row-vs-page differentiator: a row has few direct
    // children, a page-wrapper has either one or dozens of mixed blocks.
    const MAX_FLEX_DIRECT_CHILDREN: usize = 8;
    let flex_parents: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| {
            if !is_flex_div(n) {
                return false;
            }
            let mut p = n.parent();
            while let Some(parent) = p {
                if is_flex_div(&parent) {
                    return false;
                }
                p = parent.parent();
            }
            let direct = n
                .children()
                .filter(|c| c.as_element().is_some())
                .count();
            (2..=MAX_FLEX_DIRECT_CHILDREN).contains(&direct)
        })
        .collect();
    let mut targets: Vec<(usize, NodeRef)> = Vec::new();
    for parent in flex_parents {
        for d in parent.descendants() {
            if local_name_is(&d, "div") {
                targets.push((depth(&d), d));
            }
        }
    }
    targets.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, d) in targets {
        if d.parent().is_none() {
            continue;
        }
        let has_block = d.descendants().any(|c| {
            c.as_element()
                .map(|el| {
                    matches!(
                        &*el.name.local,
                        "table" | "ul" | "ol" | "li" | "h1" | "h2" | "h3" | "h4" | "h5"
                            | "h6" | "pre" | "blockquote" | "hr" | "p" | "div"
                    )
                })
                .unwrap_or(false)
        });
        if has_block {
            continue;
        }
        let inner: Vec<NodeRef> = d.children().collect();
        for c in inner {
            c.detach();
            d.insert_before(c);
        }
        d.insert_before(NodeRef::new_text(" "));
        d.detach();
    }
}

fn is_flex_div(n: &NodeRef) -> bool {
    n.as_element()
        .map(|el| {
            &*el.name.local == "div"
                && el
                    .attributes
                    .borrow()
                    .get("style")
                    .map(|s| s.contains("display: flex") || s.contains("display:flex"))
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Squash any newline/tab inside `<a>` text (whether from the source HTML's
/// `<br>` substitution or raw whitespace) to a single space. Markdown link
/// text on multiple physical lines breaks rendering for many readers and
/// confuses our wrap pass (each line is processed in isolation, splitting
/// the atomic `[…](…)` token). Applied after `replace_brs` so substituted
/// newlines are also normalised.
fn flatten_link_text(root: &NodeRef) {
    let anchors: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| local_name_is(n, "a"))
        .collect();
    for a in anchors {
        let texts: Vec<NodeRef> = a
            .inclusive_descendants()
            .filter(|n| n.as_text().is_some())
            .collect();
        for t in texts {
            let cell = t.as_text().unwrap();
            let s = cell.borrow().clone();
            if s.contains('\n') || s.contains('\t') {
                let cleaned: String = s
                    .chars()
                    .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
                    .collect();
                *cell.borrow_mut() = cleaned;
            }
        }
    }
}


fn local_name_is(n: &NodeRef, name: &str) -> bool {
    n.as_element()
        .map(|el| &*el.name.local == name)
        .unwrap_or(false)
}

fn attr(n: &NodeRef, name: &str) -> Option<String> {
    n.as_element()
        .and_then(|el| el.attributes.borrow().get(name).map(str::to_owned))
}

fn depth(n: &NodeRef) -> usize {
    let mut d = 0;
    let mut p = n.parent();
    while let Some(parent) = p {
        d += 1;
        p = parent.parent();
    }
    d
}

// ─── Table flattening ───────────────────────────────────────────────────────

fn flatten_tables(root: &NodeRef) {
    // Collect deepest-first so an outer table's cells already contain
    // paragraph rewrites of any inner tables before we look at it.
    let mut tables: Vec<(usize, NodeRef)> = root
        .inclusive_descendants()
        .filter(|n| local_name_is(n, "table"))
        .map(|n| (depth(&n), n))
        .collect();
    tables.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, table) in tables {
        if table.parent().is_none() {
            continue; // already swallowed by an outer rewrite
        }
        if subtree_text(&table).trim().is_empty() {
            table.detach();
            continue;
        }
        if is_data_table(&table) {
            continue;
        }
        flatten_one_table(&table);
    }
}

/// Heuristic: most marketing/notification HTML uses `<table>` purely for
/// column layout, so we default to "layout" and only treat tables as data
/// when there's positive evidence:
///   * has `<th>` anywhere, or
///   * has `<thead>` / `<caption>`, or
///   * uniform >=2-cell rows with a real `border` attribute.
/// Explicit `role="presentation"` / `role="none"` always wins as layout,
/// and any nested `<table>` strongly implies layout.
fn is_data_table(t: &NodeRef) -> bool {
    if let Some(role) = attr(t, "role") {
        let r = role.trim().to_ascii_lowercase();
        if r == "presentation" || r == "none" {
            return false;
        }
    }
    // `<thead>` or `<caption>` is a strong semantic signal of a real data
    // table. Bare `<th>` (without `<thead>`) is not — Steam, Mailchimp et al
    // routinely use `<th class="column-…">` purely for column layout, where
    // the `<th>` cells are siblings of `<td>` data cells in the same `<tr>`.
    if has_own_descendant(t, "thead") || has_own_descendant(t, "caption") {
        return true;
    }
    if has_nested_table(t) {
        return false;
    }

    let rows = collect_rows(t);
    if rows.len() < 2 {
        return false;
    }
    let counts: Vec<usize> = rows.iter().map(count_cells).collect();
    let max_c = *counts.iter().max().unwrap_or(&0);
    let min_c = *counts.iter().min().unwrap_or(&0);
    if max_c < 2 {
        return false;
    }

    let border = attr(t, "border").unwrap_or_default();
    let has_border = border
        .parse::<i32>()
        .map(|n| n > 0)
        .unwrap_or(!border.is_empty());

    min_c == max_c && has_border
}

/// Like `find descendant by tag`, but only matches descendants whose nearest
/// `<table>` ancestor is `root`. Without this, `is_data_table` for a layout
/// wrapper sees `<th>` / `<thead>` / `<caption>` from a nested data table
/// and falsely marks the wrapper as data — leaving the wrapper unflattened
/// so all its content (including the nested data table itself) gets emitted
/// as a giant unstructured blob.
fn has_own_descendant(root: &NodeRef, tag: &str) -> bool {
    root.descendants()
        .filter(|n| local_name_is(n, tag))
        .any(|n| nearest_table_ancestor(&n).as_ref() == Some(root))
}

fn has_nested_table(t: &NodeRef) -> bool {
    t.descendants().any(|n| local_name_is(&n, "table"))
}

fn collect_rows(t: &NodeRef) -> Vec<NodeRef> {
    // Only `<tr>`s whose nearest `<table>` ancestor is `t` itself. Without
    // this, an outer layout table sweeps in `<tr>`s from any nested data
    // table (e.g. Bitbucket PR notification's `commits-table`) and flattens
    // those rows into paragraphs, destroying the inner table.
    t.descendants()
        .filter(|n| local_name_is(n, "tr"))
        .filter(|tr| nearest_table_ancestor(tr).as_ref() == Some(t))
        .collect()
}

fn nearest_table_ancestor(n: &NodeRef) -> Option<NodeRef> {
    let mut p = n.parent();
    while let Some(parent) = p {
        if local_name_is(&parent, "table") {
            return Some(parent);
        }
        p = parent.parent();
    }
    None
}

fn count_cells(tr: &NodeRef) -> usize {
    tr.children()
        .filter(|n| local_name_is(n, "td") || local_name_is(n, "th"))
        .count()
}

fn make_paragraph() -> NodeRef {
    // Parse a tiny fragment rather than calling NodeRef::new_element directly
    // to avoid coupling to kuchikiki's internal markup5ever version.
    parse_html()
        .one("<p></p>")
        .descendants()
        .find(|n| local_name_is(n, "p"))
        .expect("kuchikiki always materialises the parsed <p>")
}

fn flatten_one_table(table: &NodeRef) {
    let rows = collect_rows(table);
    let mut emitted: Vec<NodeRef> = Vec::new();

    for tr in rows {
        let cells: Vec<NodeRef> = tr
            .children()
            .filter(|n| local_name_is(n, "td") || local_name_is(n, "th"))
            .collect();
        if cells.is_empty() {
            continue;
        }

        // Walk the row's cells. Inline runs (text + inline elements, plus
        // cells that wrap their content in a single `<p>`/`<div>`) accumulate
        // into one paragraph spanning the row, joined by single spaces. Block
        // kids — `<table>`, lists, headings, multiple sibling `<p>`s — emit
        // as standalone siblings so their structure survives. This keeps a
        // Bitbucket PR row that contains [title-`<p>`, desc-`<p>`, branch-`<p>`]
        // as three separate paragraphs while still collapsing the
        // [feature][→][develop] branch lozenges into a single line.
        let mut row_p: Option<NodeRef> = None;
        for cell in cells {
            let kids: Vec<NodeRef> = cell.children().collect();
            let non_blank: Vec<NodeRef> =
                kids.iter().filter(|k| !is_blank(k)).cloned().collect();
            if non_blank.is_empty() {
                continue;
            }

            let inline_content = classify_cell(&non_blank);
            match inline_content {
                CellMode::Inline(items) => {
                    let p = row_p.get_or_insert_with(make_paragraph).clone();
                    if !ends_with_whitespace(&p) && p.first_child().is_some() {
                        p.append(NodeRef::new_text(" "));
                    }
                    // If classify_cell returned the cell's full inline run
                    // (text + inline elements with whitespace-only text nodes
                    // sandwiched between), preserve those separators —
                    // marketing legends emit `<span></span>X<span> (n)</span>
                    // \n<span></span>Y` and the inter-element whitespace text
                    // is the only thing keeping `X (n)` from glueing to `Y`.
                    let with_ws = include_inline_whitespace(&kids, &items);
                    for n in with_ws {
                        n.detach();
                        p.append(n);
                    }
                }
                CellMode::Blocks(blocks) => {
                    if let Some(p) = row_p.take() {
                        if !subtree_text(&p).trim().is_empty() {
                            emitted.push(p);
                        }
                    }
                    for b in blocks {
                        b.detach();
                        emitted.push(b);
                    }
                }
                CellMode::Paragraph(nodes) => {
                    if let Some(p) = row_p.take() {
                        if !subtree_text(&p).trim().is_empty() {
                            emitted.push(p);
                        }
                    }
                    let p = make_paragraph();
                    for n in nodes {
                        n.detach();
                        p.append(n);
                    }
                    emitted.push(p);
                }
            }
        }
        if let Some(p) = row_p {
            if !subtree_text(&p).trim().is_empty() {
                emitted.push(p);
            }
        }
    }

    for n in emitted {
        table.insert_before(n);
    }
    table.detach();
}

enum CellMode {
    Inline(Vec<NodeRef>),
    Blocks(Vec<NodeRef>),
    /// The cell's inline content spans multiple `<br>` lines; emit it as one
    /// standalone multi-line paragraph (kept tight, not merged with siblings).
    Paragraph(Vec<NodeRef>),
}

/// Re-thread whitespace-only text nodes from `kids` into `items` whenever
/// they sit between two retained nodes. Filtering blanks earlier was right
/// for cells with stray empty text padding, but inline runs need their
/// inter-element whitespace preserved or the serialiser glues neighbouring
/// text straight together (`Escalating (7)Regressed (12)`). When `items`
/// came from a wrapper's grandchildren (classify_cell unwrapping `<p>`/`<div>`)
/// it is not a subsequence of `kids`; in that case fall back to `items`
/// untouched.
fn include_inline_whitespace(kids: &[NodeRef], items: &[NodeRef]) -> Vec<NodeRef> {
    if items.is_empty() {
        return Vec::new();
    }
    let is_subseq = {
        let mut it = items.iter();
        let mut next = it.next();
        for k in kids {
            if let Some(want) = next {
                if *want == *k {
                    next = it.next();
                }
            } else {
                break;
            }
        }
        next.is_none()
    };
    if !is_subseq {
        return items.to_vec();
    }
    let mut out: Vec<NodeRef> = Vec::with_capacity(items.len());
    let mut item_iter = items.iter().peekable();
    let mut started = false;
    let mut last_was_item = false;
    for k in kids {
        if item_iter.peek().map(|i| **i == *k).unwrap_or(false) {
            out.push(item_iter.next().unwrap().clone());
            started = true;
            last_was_item = true;
        } else if started && last_was_item && is_blank(k) {
            if item_iter.peek().is_some() {
                out.push(k.clone());
            }
            last_was_item = false;
        }
    }
    out
}

fn classify_cell(non_blank: &[NodeRef]) -> CellMode {
    // A cell holding a <br> is multi-line (a benefit card `<strong>Title</strong>
    // <br>desc`, an address block, a title+subtitle). Merging it inline with the
    // next cell would splice that cell onto this one's tail line; emitting one
    // block per child would scatter the lines with blank gaps. Keep it as a
    // single tight multi-line paragraph instead.
    if non_blank.iter().any(subtree_has_br) {
        return CellMode::Paragraph(non_blank.to_vec());
    }
    let all_inline = non_blank.iter().all(|k| !is_block_kid(k));
    if all_inline {
        return CellMode::Inline(non_blank.to_vec());
    }
    if non_blank.len() == 1 {
        let only = &non_blank[0];
        let is_wrapper = only
            .as_element()
            .map(|el| matches!(&*el.name.local, "p" | "div"))
            .unwrap_or(false);
        if is_wrapper {
            let grandkids: Vec<NodeRef> = only.children().collect();
            let gk_non_blank: Vec<NodeRef> =
                grandkids.iter().filter(|n| !is_blank(n)).cloned().collect();
            if gk_non_blank.iter().all(|k| !is_block_kid(k)) {
                return CellMode::Inline(grandkids);
            }
            return CellMode::Blocks(gk_non_blank);
        }
    }
    CellMode::Blocks(non_blank.to_vec())
}

fn subtree_has_br(n: &NodeRef) -> bool {
    n.inclusive_descendants().any(|d| local_name_is(&d, "br"))
}

fn is_block_kid(n: &NodeRef) -> bool {
    n.as_element()
        .map(|el| {
            matches!(
                &*el.name.local,
                "p" | "div"
                    | "table"
                    | "ul"
                    | "ol"
                    | "li"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "pre"
                    | "blockquote"
                    | "hr"
            )
        })
        .unwrap_or(false)
}

fn ends_with_whitespace(n: &NodeRef) -> bool {
    let last = match n.last_child() {
        Some(c) => c,
        None => return true, // empty parent — no separator needed
    };
    if let Some(t) = last.as_text() {
        t.borrow()
            .chars()
            .last()
            .map(|c| c.is_whitespace())
            .unwrap_or(true)
    } else {
        false
    }
}

fn is_blank(n: &NodeRef) -> bool {
    if let Some(t) = n.as_text() {
        return t.borrow().trim().is_empty();
    }
    n.as_comment().is_some()
}

// ─── Markdown output helpers ─────────────────────────────────────────────────

fn structural_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^(#{1,6}\s|[-*+]\s|\d+\.\s|>\s|[-*_]{3,}\s*$)").unwrap()
    })
}

fn ref_link_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^\[[^\]]+\]:\s").unwrap())
}

fn blank_runs_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\n{3,}").unwrap())
}

fn collapse_blank_runs(md: &str) -> String {
    blank_runs_re().replace_all(md, "\n\n").into_owned()
}

fn list_marker_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^(?:[-*+]\s+|\d+\.\s+)").unwrap())
}

fn token_re() -> &'static Regex {
    // A wrap token is any whitespace-separated chunk, but markdown links
    // (`[text](url)`, `![alt](src)`) and inline code (`` `code` ``) embed
    // spaces that must not split the chunk. Tokenise as one-or-more of
    // (link | code | non-space char) so trailing punctuation like `,` after
    // a link is absorbed into the same token instead of orphaning onto its
    // own line when the link itself fills the wrap budget. Inside link
    // text and URL, `\\.` consumes any escaped char so that JSON-shaped
    // payloads with `\[` and `\]` don't prematurely close the match.
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?:!?\[(?:\\.|[^\]])*\]\((?:\\.|[^)])*\)|`[^`]*`|[^\s])+").unwrap()
    })
}

fn wrap_width() -> usize {
    // Allow override for terminals wider/narrower than 80.
    std::env::var("AERC_FILTER_WIDTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&w: &usize| (40..=240).contains(&w))
        .unwrap_or(80)
}

/// Hard-wrap paragraphs at `width` columns on word boundaries. Markdown
/// links/images and inline code are kept intact even if a single token
/// exceeds the limit. Fenced code, indented code, headings, tables, and
/// reference-link definitions are left untouched. Blockquote and list
/// continuation lines preserve their leading prefix/indent.
fn wrap_lines(md: &str, width: usize) -> String {
    let mut out = String::with_capacity(md.len());
    let mut in_fence = false;
    for (i, line) in md.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let starts_fence = line.starts_with("```") || line.starts_with("~~~");
        if starts_fence {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence
            || line.starts_with("    ")
            || line.starts_with('\t')
            || line.trim().is_empty()
        {
            out.push_str(line);
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#')
            || trimmed.starts_with('|')
            || ref_link_re().is_match(trimmed)
            || structural_re()
                .find(trimmed)
                .map(|m| m.as_str().contains(['_', '*', '-']) && trimmed.chars().all(|c| matches!(c, '-' | '*' | '_' | ' ')))
                .unwrap_or(false)
        {
            out.push_str(line);
            continue;
        }

        let leading: String = line.chars().take_while(|c| *c == ' ').collect();
        let after_indent = &line[leading.len()..];

        let mut quote_end = 0;
        for (idx, ch) in after_indent.char_indices() {
            if ch == '>' || ch == ' ' {
                quote_end = idx + ch.len_utf8();
            } else {
                break;
            }
        }
        let quote_prefix = if after_indent[..quote_end].contains('>') {
            &after_indent[..quote_end]
        } else {
            ""
        };
        let body = &after_indent[quote_prefix.len()..];

        let (list_marker, content) = if let Some(m) = list_marker_re().find(body) {
            (&body[..m.end()], &body[m.end()..])
        } else {
            ("", body)
        };
        let cont_indent = " ".repeat(list_marker.chars().count());
        let first_prefix = format!("{}{}{}", leading, quote_prefix, list_marker);
        let cont_prefix = format!("{}{}{}", leading, quote_prefix, cont_indent);

        let tokens: Vec<&str> = token_re().find_iter(content).map(|m| m.as_str()).collect();
        if tokens.is_empty() {
            out.push_str(line);
            continue;
        }

        out.push_str(&first_prefix);
        let mut col = first_prefix.chars().count();
        let mut at_line_start = true;
        for tok in tokens {
            // Wrap on *visible* width: the pager rewrites `[text](url)` to bare
            // `text`, so a token's on-screen cost is its link text, not the
            // hundreds of raw chars a tracking URL adds. Measuring raw width
            // here made short `[Reply] · [Like] · [date]` footers (tiny once
            // rendered) look oversized and either explode across lines or glue a
            // whole paragraph after a long leading link.
            let tlen = visible_width(tok);
            let would_overflow = col + 1 + tlen > width;
            // Suppress the wrap only when it cannot help: the token's visible
            // text is itself wider than the budget (a long error string or
            // unbreakable atom), so moving it to a fresh line gains nothing. A
            // token that *does* fit fresh always gets to wrap, even when the
            // current line is already over budget from a preceding oversized
            // atom — the first wrap resets `col`, so following words flow
            // normally instead of orphaning one-per-line.
            let useless_wrap = tlen > width;
            if !at_line_start && would_overflow && !useless_wrap {
                out.push('\n');
                out.push_str(&cont_prefix);
                col = cont_prefix.chars().count();
                at_line_start = true;
            }
            if !at_line_start {
                out.push(' ');
                col += 1;
            }
            out.push_str(tok);
            col += tlen;
            at_line_start = false;
        }
    }
    out
}

fn link_strip_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\[([^\]\n]+)\]\([^)\n]*\)").unwrap())
}

/// Visible width = chars the user sees once the pager rewrites `[text](url)`
/// to bare `text`.
fn visible_width(s: &str) -> usize {
    link_strip_re().replace_all(s.trim(), "$1").chars().count()
}

fn ie_conditional_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // Non-comment IE conditionals: <![if ...]>...<![endif]>
    // Distinct from <!--[if ...]--> comment form which html5ever handles natively.
    // These appear in Outlook/Word HTML for list bullets, VML fallbacks, etc.
    R.get_or_init(|| Regex::new(r"(?si)<!\[if[^\]]*\]>.*?<!\[endif\]>").unwrap())
}

fn strip_ie_conditionals(html: &str) -> String {
    ie_conditional_re().replace_all(html, "").into_owned()
}

// ─── Markdown serialiser ─────────────────────────────────────────────────────

/// Walk the cleaned DOM and emit Markdown. Replaces htmd + all regex post-
/// processing passes. Heading levels are normalised so the shallowest heading
/// in the document becomes `#`. Tables use visible-width column padding.
fn to_markdown(root: &NodeRef, width: usize) -> String {
    let shift = min_heading_level(root).saturating_sub(1);
    let blocks = drop_empty_sections(doc_blocks(root, shift));
    let md = blocks.join("\n\n");
    let md = collapse_blank_runs(&md);
    wrap_lines(&md, width)
}

/// Leading-`#` level of a block, if it is an ATX heading (`## Foo`).
fn block_heading_level(block: &str) -> Option<usize> {
    let h = block.trim_start();
    let hashes = h.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && h[hashes..].starts_with(' ') {
        Some(hashes)
    } else {
        None
    }
}

/// Drop headings that introduce nothing: a heading whose following block is
/// another heading at the same or shallower level (e.g. an empty `## REVIEWERS`
/// section in a Bitbucket PR sitting right before `## NEW ACTIVITY`). Headings
/// followed by content, by a deeper sub-heading, or at end of document are kept.
fn drop_empty_sections(blocks: Vec<String>) -> Vec<String> {
    let levels: Vec<Option<usize>> = blocks.iter().map(|b| block_heading_level(b)).collect();
    blocks
        .iter()
        .enumerate()
        .filter(|(i, _)| match levels[*i] {
            None => true,
            Some(lvl) => !matches!(levels.get(i + 1), Some(Some(next)) if *next <= lvl),
        })
        .map(|(_, b)| b.clone())
        .collect()
}

/// Find the shallowest heading level with non-empty text content.
fn min_heading_level(root: &NodeRef) -> usize {
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

fn doc_blocks(root: &NodeRef, shift: usize) -> Vec<String> {
    // Start from <body> if present, else root.
    let start = root
        .inclusive_descendants()
        .find(|n| local_name_is(n, "body"))
        .unwrap_or_else(|| root.clone());
    node_blocks(&start, shift)
}

fn node_blocks(node: &NodeRef, shift: usize) -> Vec<String> {
    node.children()
        .flat_map(|c| child_to_blocks(&c, shift))
        .collect()
}

fn has_block_child(n: &NodeRef) -> bool {
    n.children()
        .any(|c| c.as_element().map(|el| is_block_name(&el.name.local)).unwrap_or(false))
}

fn is_block_name(name: &str) -> bool {
    matches!(
        name,
        "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "ul" | "ol" | "hr" | "pre" | "blockquote" | "table"
            | "header" | "footer" | "section" | "article" | "main"
            | "aside" | "nav" | "center"
    )
}

fn subtree_has_block(n: &NodeRef) -> bool {
    n.descendants()
        .any(|c| c.as_element().map(|el| is_block_name(&el.name.local)).unwrap_or(false))
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

fn child_to_blocks(node: &NodeRef, shift: usize) -> Vec<String> {
    if let Some(t) = node.as_text() {
        let s = t.borrow();
        let trimmed = s.trim();
        return if trimmed.is_empty() {
            vec![]
        } else {
            vec![escape_text(trimmed)]
        };
    }

    let el = match node.as_element() {
        Some(e) => e,
        None => return vec![],
    };

    match &*el.name.local {
        "html" | "body" => node_blocks(node, shift),

        // Structural containers (and <p>): recurse when block children present, else paragraph.
        "p" | "div" | "center" | "header" | "footer" | "section" | "article" | "main"
        | "aside" | "nav" | "form" | "fieldset" => {
            if has_block_child(node) {
                node_blocks(node, shift)
            } else {
                let s = tidy_inline_block(&children_inline(node));
                if s.is_empty() {
                    vec![]
                } else {
                    vec![s]
                }
            }
        }

        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let lvl = heading_level_of(&el.name.local)
                .saturating_sub(shift)
                .max(1)
                .min(6);
            // A heading is a single line; fold any `<br>`-newline to a space.
            let s = children_inline(node).replace('\n', " ").trim().to_string();
            if s.is_empty() {
                return vec![];
            }
            vec![format!("{} {}", "#".repeat(lvl), s)]
        }

        "hr" => vec!["---".to_string()],

        "pre" => {
            let text = subtree_text(node);
            let trimmed = text.trim_end();
            if trimmed.is_empty() {
                return vec![];
            }
            vec![format!("```\n{}\n```", trimmed)]
        }

        "blockquote" => {
            let inner = node_blocks(node, shift);
            if inner.is_empty() {
                return vec![];
            }
            let joined = inner.join("\n\n");
            let quoted = joined
                .lines()
                .map(|l| {
                    if l.is_empty() {
                        ">".to_string()
                    } else {
                        format!("> {}", l)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            vec![quoted]
        }

        "ul" => {
            let s = serialize_list(node, false, 0, shift);
            if s.is_empty() {
                vec![]
            } else {
                vec![s]
            }
        }
        "ol" => {
            let s = serialize_list(node, true, 0, shift);
            if s.is_empty() {
                vec![]
            } else {
                vec![s]
            }
        }

        "table" => {
            let s = serialize_table(node);
            if s.is_empty() {
                vec![]
            } else {
                vec![s]
            }
        }

        // Unknown/inline element at block level: recurse if any descendant is
        // a block element (handles schema.org/microdata spans that wrap the
        // entire email layout), otherwise treat as inline paragraph.
        _ => {
            if subtree_has_block(node) {
                node_blocks(node, shift)
            } else {
                let s = tidy_inline_block(&node_inline(node));
                if s.is_empty() {
                    vec![]
                } else {
                    vec![s]
                }
            }
        }
    }
}

// ─── Inline serialisation ────────────────────────────────────────────────────

fn node_inline(node: &NodeRef) -> String {
    if let Some(t) = node.as_text() {
        return escape_text(&t.borrow());
    }
    let el = match node.as_element() {
        Some(e) => e,
        None => return String::new(),
    };
    match &*el.name.local {
        "a" => {
            // A link is a single line; fold any `<br>`-newline in the text to a
            // space so `[multi\nline](url)` doesn't break the link syntax.
            let inner = children_inline(node).replace('\n', " ");
            let inner = inner.split_whitespace().collect::<Vec<_>>().join(" ");
            if inner.is_empty() || is_decorative_glyph(&inner) {
                return String::new();
            }
            let href = el
                .attributes
                .borrow()
                .get("href")
                .map(str::to_owned)
                .unwrap_or_default();
            if href.is_empty() {
                return inner;
            }
            // Garbage href: broken templates sometimes stuff text or markup
            // into the attribute (e.g. href="Legaldesk.dk<br>Njalsgade 21F..").
            // A real URL has no whitespace or angle brackets — drop the link
            // syntax and keep the visible text rather than emit a broken
            // [text](url with <br> and spaces).
            if href.contains(|c: char| c.is_whitespace() || c == '<' || c == '>') {
                return inner;
            }
            // [url](url) → bare url
            if inner.trim_end_matches('/') == href.trim_end_matches('/') {
                return inner;
            }
            // If inner already contains markdown link syntax, use plain text
            // to avoid nested [[...](url)](url) which breaks parsers.
            let display = if inner.contains("](") {
                subtree_text(node)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                inner
            };
            if display.is_empty() {
                return String::new();
            }
            format!("[{}]({})", display, href)
        }
        "strong" | "b" => emphasis(node, "**"),
        "em" | "i" => emphasis(node, "*"),
        "code" => {
            let s = subtree_text(node);
            let s = s.trim();
            if s.is_empty() {
                return String::new();
            }
            format!("`{}`", s)
        }
        // `<br>` is an intentional line break — emit a real newline so
        // signatures, address blocks and log dumps stay tight (consecutive
        // lines) instead of reflowing onto one line or exploding into
        // blank-line-separated paragraphs. Two in a row become `\n\n` (a blank
        // line), matching the source's intent. Contexts where a raw newline is
        // harmful sanitise it: `emphasis`/links split or fold it so markers
        // never span lines; table cells, headings and list items flatten it.
        "br" => "\n".to_string(),
        _ => children_inline(node),
    }
}

/// Trim a multi-line inline block (a paragraph whose `<br>`s became newlines):
/// strip surrounding whitespace on every line so leading spaces from source
/// text nodes (`...<br>\n  Diff: ...`) don't survive as ragged indentation,
/// while preserving blank lines (from `<br><br>`) between the kept lines.
fn tidy_inline_block(s: &str) -> String {
    if !s.contains('\n') {
        return s.trim().to_string();
    }
    s.lines()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn children_inline(node: &NodeRef) -> String {
    let s: String = node.children().map(|c| node_inline(&c)).collect();
    // Adjacent same-marker emphasis with no separating whitespace — e.g.
    // `<b>So, w</b><b>atch this</b>` splitting a word mid-token — concatenates
    // to `**So, w****atch this**`. The empty `****` is noise (escaped literal
    // asterisks are `\*`, so bare `****` only ever comes from marker adjacency);
    // dropping it merges the runs into `**So, watch this**`.
    if s.contains("****") {
        s.replace("****", "")
    } else {
        s
    }
}

/// Wrap inline content in an emphasis marker (`**` or `*`), keeping any leading
/// or trailing whitespace *outside* the markers. HTML often splits a phrase
/// across adjacent `<strong>`/`<b>` runs where the only separating space lives
/// at a marker boundary (e.g. `<b>Twenty</b><b>&nbsp;minutes</b>`); trimming it
/// away would fuse the words (`**Twenty****minutes**`).
fn emphasis(node: &NodeRef, marker: &str) -> String {
    let inner = children_inline(node);
    // A `<br>` inside the emphasis (`<b>line1<br>line2</b>`) leaves a newline in
    // `inner`; wrap each line on its own so the markers never span a line break
    // (`**line1**\n**line2**`), which would otherwise render the literal `**`.
    if inner.contains('\n') {
        return inner
            .split('\n')
            .map(|line| {
                let s = line.trim();
                if s.is_empty() {
                    String::new()
                } else {
                    format!("{marker}{s}{marker}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let s = inner.trim();
    if s.is_empty() {
        return String::new();
    }
    let lead = if inner.starts_with(|c: char| c.is_whitespace()) { " " } else { "" };
    let trail = if inner.ends_with(|c: char| c.is_whitespace()) { " " } else { "" };
    format!("{lead}{marker}{s}{marker}{trail}")
}

/// Escape characters that could create unintended Markdown syntax. Collapses
/// inline whitespace runs (tabs, newlines from HTML source) to a single space,
/// matching the browser's whitespace collapsing behaviour.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_space = false;
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{00A0}') {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            prev_space = false;
            match c {
                '\\' | '`' | '*' | '_' | '[' | ']' => {
                    out.push('\\');
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
    }
    out
}

// ─── List serialisation ──────────────────────────────────────────────────────

fn serialize_list(list: &NodeRef, ordered: bool, depth: usize, shift: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut items: Vec<String> = Vec::new();
    let mut n = 1usize;

    for child in list.children() {
        if !local_name_is(&child, "li") {
            continue;
        }

        let marker = if ordered {
            format!("{}. ", n)
        } else {
            "- ".to_string()
        };
        n += 1;
        // Collect inline text and any nested sub-lists from li's children.
        let mut inline_parts: Vec<String> = Vec::new();
        let mut sub_lists: Vec<String> = Vec::new();

        for kid in child.children() {
            if local_name_is(&kid, "ul") {
                sub_lists.push(serialize_list(&kid, false, depth + 1, shift));
            } else if local_name_is(&kid, "ol") {
                sub_lists.push(serialize_list(&kid, true, depth + 1, shift));
            } else if is_block_kid(&kid) {
                // <p>/<div> inside li: gather as inline text
                let s = children_inline(&kid).trim().to_string();
                if !s.is_empty() {
                    inline_parts.push(s);
                }
            } else {
                inline_parts.push(node_inline(&kid));
            }
        }

        // A list item is one logical line; fold any `<br>`-newline to a space so
        // a continuation doesn't escape the bullet's indentation.
        let item_text = inline_parts
            .join("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if item_text.is_empty() && sub_lists.is_empty() {
            continue;
        }

        let mut item_lines = vec![format!("{}{}{}", indent, marker, item_text)];
        for sub in sub_lists {
            // Sub-list already carries its own depth-based indent.
            for line in sub.lines() {
                item_lines.push(line.to_string());
            }
        }
        items.push(item_lines.join("\n"));
    }

    items.join("\n")
}

// ─── Table serialisation ─────────────────────────────────────────────────────

fn serialize_table(table: &NodeRef) -> String {
    let rows = collect_rows(table);
    if rows.len() < 2 {
        return String::new();
    }

    // Serialise every cell to inline Markdown.
    let mut parsed: Vec<Vec<String>> = rows
        .iter()
        .map(|tr| {
            tr.children()
                .filter(|n| local_name_is(n, "td") || local_name_is(n, "th"))
                // A `<br>`-newline inside a cell would break the table row;
                // collapse all whitespace runs (incl. those newlines) to single
                // spaces. Links never contain spaces, so this can't split one.
                .map(|cell| children_inline(&cell).split_whitespace().collect::<Vec<_>>().join(" "))
                .collect()
        })
        .collect();

    let ncols = parsed.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return String::new();
    }

    // Drop columns where every cell is empty.
    let keep: Vec<usize> = (0..ncols)
        .filter(|&c| {
            parsed
                .iter()
                .any(|row| row.get(c).map(|s| !s.trim().is_empty()).unwrap_or(false))
        })
        .collect();
    if keep.len() < ncols && !keep.is_empty() {
        for row in &mut parsed {
            *row = keep
                .iter()
                .map(|&c| row.get(c).cloned().unwrap_or_default())
                .collect();
        }
    }
    let ncols = parsed.first().map(|r| r.len()).unwrap_or(0);
    if ncols == 0 {
        return String::new();
    }

    // Compute column widths from visible text (strips link syntax).
    let mut widths = vec![3usize; ncols];
    for row in &parsed {
        for (c, cell) in row.iter().enumerate() {
            let w = visible_width(cell);
            if w > widths[c] {
                widths[c] = w;
            }
        }
    }

    // Emit: row 0 is always the header, separator follows, then data rows.
    let mut lines: Vec<String> = Vec::new();
    for (i, row) in parsed.iter().enumerate() {
        let mut line = String::from("|");
        for c in 0..ncols {
            let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
            let vis = visible_width(cell);
            let pad = widths[c].saturating_sub(vis);
            line.push(' ');
            line.push_str(cell);
            if pad > 0 {
                line.push_str(&" ".repeat(pad));
            }
            line.push_str(" |");
        }
        lines.push(line);
        if i == 0 {
            let mut sep = String::from("|");
            for &w in &widths {
                sep.push(' ');
                sep.push_str(&"-".repeat(w));
                sep.push_str(" |");
            }
            lines.push(sep);
        }
    }
    lines.join("\n")
}
