//! The typed Markdown AST and the document-level transforms that run on it.
//!
//! Everything the old string pipeline re-parsed with regexes (heading levels,
//! link-only blocks, visible widths, blank runs) is expressed structurally
//! here, so the renderer below can emit output in one deterministic pass.

/// A block-level Markdown node. `Table` rows are already column-pruned; the
/// first row is the header row.
#[derive(Clone, Debug)]
pub enum Block {
    Heading { level: usize, inlines: Vec<Inline> },
    Paragraph { inlines: Vec<Inline> },
    List { ordered: bool, items: Vec<ListItem> },
    Table { rows: Vec<Vec<Vec<Inline>>> },
    Code { body: String },
    Quote { blocks: Vec<Block> },
    Rule,
}

#[derive(Clone, Debug)]
pub struct ListItem {
    /// 1-based position among the source `<li>` children; empty items keep
    /// their number so ordered lists number like the source HTML does.
    pub number: usize,
    pub inlines: Vec<Inline>,
    pub sub_lists: Vec<Block>,
}

/// An inline Markdown node. `Text` holds normalised source text (unicode
/// escapes decoded, whitespace collapsed) that the renderer escapes;
/// `Raw` holds already-rendered Markdown that is emitted verbatim.
#[derive(Clone, Debug)]
pub enum Inline {
    Text(String),
    Raw(String),
    Code(String),
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
    Link {
        /// Normalised, already-escaped display text.
        text: String,
        href: String,
    },
    LineBreak,
}

/// Drop headings that introduce nothing: a heading whose following block is
/// another heading at the same or shallower level (e.g. an empty `## REVIEWERS`
/// section in a Bitbucket PR sitting right before `## NEW ACTIVITY`). Headings
/// followed by content, by a deeper sub-heading, or at end of document are kept.
pub fn drop_empty_sections(blocks: Vec<Block>) -> Vec<Block> {
    let levels: Vec<Option<usize>> = blocks
        .iter()
        .map(|b| match b {
            Block::Heading { level, .. } => Some(*level),
            _ => None,
        })
        .collect();
    blocks
        .into_iter()
        .enumerate()
        .filter(|(i, _)| match levels[*i] {
            None => true,
            Some(lvl) => !matches!(levels.get(i + 1), Some(Some(next)) if *next <= lvl),
        })
        .map(|(_, b)| b)
        .collect()
}

/// Remap the distinct heading levels actually present to a contiguous `1..n`
/// range. Emails routinely jump levels — a sidebar promo marked up as `<h1>`
/// with the real sections as `<h4>` — which the uniform shift preserves as a
/// jarring `#` → `####`. Rank-mapping renders that as `#` → `##`.
pub fn compress_heading_levels(blocks: Vec<Block>) -> Vec<Block> {
    let mut used: Vec<usize> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Heading { level, .. } => Some(*level),
            _ => None,
        })
        .collect();
    used.sort_unstable();
    used.dedup();
    if used.len() < 2 {
        return blocks;
    }
    blocks
        .into_iter()
        .map(|b| match b {
            Block::Heading { level, inlines } => {
                let rank = used.iter().position(|&l| l == level).unwrap() + 1;
                Block::Heading {
                    level: rank,
                    inlines,
                }
            }
            other => other,
        })
        .collect()
}

/// Join a run of consecutive blocks that are each a single short link into one
/// ` · `-separated line. Vendor emails stack nav bars, framework-badge rows and
/// footers (`[Next.js]`, `[Nuxt]`, … / `[Unsubscribe]`, `[Privacy notice]`)
/// vertically — one block per link — which renders as a tall column of links.
/// The short-text cap keeps article/post-title link lists (whose text is long)
/// on their own lines.
pub fn join_link_rows(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut run: Vec<Block> = Vec::new();
    for b in blocks {
        if is_short_link(&b) {
            run.push(b);
        } else {
            flush_run(&mut run, &mut out);
            out.push(b);
        }
    }
    flush_run(&mut run, &mut out);
    out
}

fn flush_run(run: &mut Vec<Block>, out: &mut Vec<Block>) {
    if run.len() < 2 {
        out.append(run);
        return;
    }
    // Vendor nav bars and footers sometimes repeat the same CTA link twice
    // in one row; a repeated (label, href) pair in a joined row is dropped.
    let mut merged: Vec<Inline> = Vec::new();
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for b in run.iter() {
        let Block::Paragraph { inlines } = b else {
            continue;
        };
        for inline in inlines {
            let Inline::Link { text, href } = inline else {
                continue;
            };
            let key = (text.as_str(), href.as_str());
            if seen.contains(&key) {
                continue;
            }
            if !merged.is_empty() {
                merged.push(Inline::Raw(" · ".to_string()));
            }
            seen.push(key);
            merged.push(inline.clone());
        }
    }
    run.clear();
    if !merged.is_empty() {
        out.push(Block::Paragraph { inlines: merged });
    }
}

fn is_short_link(b: &Block) -> bool {
    match b {
        Block::Paragraph { inlines } => match inlines.as_slice() {
            [Inline::Link { text, .. }] => text.chars().count() <= 30,
            _ => false,
        },
        _ => false,
    }
}
