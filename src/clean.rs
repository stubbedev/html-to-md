//! DOM surgery: the pre-parse text fix-ups and every in-place cleaning pass
//! the serialiser relies on. The pass order in [`clean_doc`] is load-bearing —
//! several passes only see the tree the previous ones produced.

use kuchikiki::traits::*;
use kuchikiki::{parse_html, NodeRef};
use regex::Regex;
use std::sync::OnceLock;

use crate::table::flatten_tables;
use crate::text::{clean_invisibles, is_decorative_glyph};

/// Steps 1–3 of the pipeline: strip IE conditionals, parse, run every DOM
/// surgery pass. Returned tree is what the serialiser expects as input.
pub fn clean_doc(input: &str) -> NodeRef {
    // Strip non-comment IE conditionals before parsing so Outlook bullet
    // spans (<![if !supportLists]><span>·</span><![endif]>) and other
    // Outlook-only blocks don't appear in the DOM as regular text nodes.
    // Standard <!--[if mso]>…<![endif]--> comment-form conditionals are
    // already handled by html5ever's bogus-comment parser + strip_comments.
    let input = strip_ie_conditionals(input);
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
    doc
}

fn ie_conditional_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // Non-comment IE conditionals: <![if ...]>...<![endif]>
    // Distinct from <!--[if ...]--> comment form which html5ever handles natively.
    // These appear in Outlook/Word HTML for list bullets, VML fallbacks, etc.
    R.get_or_init(|| Regex::new(r"(?si)<!\[if[^\]]*\]>.*?<!\[endif\]>").unwrap())
}

pub fn strip_ie_conditionals(html: &str) -> String {
    ie_conditional_re().replace_all(html, "").into_owned()
}

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

pub fn strip_comments(root: &NodeRef) {
    let comments: Vec<NodeRef> = root
        .inclusive_descendants()
        .filter(|n| n.as_comment().is_some())
        .collect();
    for c in comments {
        c.detach();
    }
}

pub fn drop_elements<F>(root: &NodeRef, predicate: F)
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

pub fn drop_empty_anchors(root: &NodeRef) {
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

pub fn subtree_text(n: &NodeRef) -> String {
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
        } else if !matches!(
            c,
            '.' | ',' | ' ' | 'k' | 'K' | 'M' | 'B' | 'm' | 's' | 'µ' | 'h'
        ) {
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
            let direct = n.children().filter(|c| c.as_element().is_some()).count();
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
    targets.sort_by_key(|t| std::cmp::Reverse(t.0));

    for (_, d) in targets {
        if d.parent().is_none() {
            continue;
        }
        let has_block = d.descendants().any(|c| {
            c.as_element()
                .map(|el| {
                    matches!(
                        &*el.name.local,
                        "table"
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
                            | "p"
                            | "div"
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

pub fn local_name_is(n: &NodeRef, name: &str) -> bool {
    n.as_element()
        .map(|el| &*el.name.local == name)
        .unwrap_or(false)
}

pub fn attr(n: &NodeRef, name: &str) -> Option<String> {
    n.as_element()
        .and_then(|el| el.attributes.borrow().get(name).map(str::to_owned))
}

pub fn depth(n: &NodeRef) -> usize {
    let mut d = 0;
    let mut p = n.parent();
    while let Some(parent) = p {
        d += 1;
        p = parent.parent();
    }
    d
}

pub fn make_paragraph() -> NodeRef {
    // Parse a tiny fragment rather than calling NodeRef::new_element directly
    // to avoid coupling to kuchikiki's internal markup5ever version.
    parse_html()
        .one("<p></p>")
        .descendants()
        .find(|n| local_name_is(n, "p"))
        .expect("kuchikiki always materialises the parsed <p>")
}

pub fn make_br() -> NodeRef {
    parse_html()
        .one("<br>")
        .descendants()
        .find(|n| local_name_is(n, "br"))
        .expect("kuchikiki always materialises the parsed <br>")
}

pub fn is_block_name(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "hr"
            | "pre"
            | "blockquote"
            | "table"
            | "header"
            | "footer"
            | "section"
            | "article"
            | "main"
            | "aside"
            | "nav"
            | "center"
    )
}

pub fn has_block_child(n: &NodeRef) -> bool {
    n.children().any(|c| {
        c.as_element()
            .map(|el| is_block_name(&el.name.local))
            .unwrap_or(false)
    })
}

pub fn subtree_has_block(n: &NodeRef) -> bool {
    n.descendants().any(|c| {
        c.as_element()
            .map(|el| is_block_name(&el.name.local))
            .unwrap_or(false)
    })
}
