//! Layout-table flattening. Most marketing/notification HTML uses `<table>`
//! for column layout; real data tables are rare and kept as pipe tables by
//! the serialiser. This module decides which is which and rewrites layout
//! tables into paragraphs in place.

use kuchikiki::NodeRef;

use crate::clean::{
    attr, depth, local_name_is, make_br, make_paragraph, subtree_has_block, subtree_text,
};

pub fn flatten_tables(root: &NodeRef) {
    // Collect deepest-first so an outer table's cells already contain
    // paragraph rewrites of any inner tables before we look at it.
    let mut tables: Vec<(usize, NodeRef)> = root
        .inclusive_descendants()
        .filter(|n| local_name_is(n, "table"))
        .map(|n| (depth(&n), n))
        .collect();
    tables.sort_by_key(|t| std::cmp::Reverse(t.0));

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
///
/// Explicit `role="presentation"` / `role="none"` always wins as layout, and any
/// nested `<table>` strongly implies layout.
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

pub fn collect_rows(t: &NodeRef) -> Vec<NodeRef> {
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
            let non_blank: Vec<NodeRef> = kids.iter().filter(|k| !is_blank(k)).cloned().collect();
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

    // If every flattened row is a short, single-line inline paragraph, this is a
    // key-value / price-summary / spec table (`Subtotal | 1.780,00 kr`). Join the
    // rows tight with `<br>` so they render as a compact block instead of a stack
    // of blank-line-separated one-liners. Any long or block-bearing row leaves the
    // whole table untouched, so marketing layout tables are never crammed.
    if emitted.len() >= 2 && emitted.iter().all(is_short_inline_paragraph) {
        let group = make_paragraph();
        for (i, n) in emitted.iter().enumerate() {
            if i > 0 {
                group.append(make_br());
            }
            for child in n.children().collect::<Vec<_>>() {
                child.detach();
                group.append(child);
            }
        }
        emitted = vec![group];
    }

    for n in emitted {
        table.insert_before(n);
    }
    table.detach();
}

/// A `<p>` holding a short single line of inline content (no block descendant,
/// no `<br>`): one row of a key-value / spec table.
fn is_short_inline_paragraph(n: &NodeRef) -> bool {
    if !local_name_is(n, "p") || subtree_has_block(n) || subtree_has_br(n) {
        return false;
    }
    let len = subtree_text(n).trim().chars().count();
    len > 0 && len <= 60
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

pub fn is_block_kid(n: &NodeRef) -> bool {
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
