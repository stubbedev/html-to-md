// Package ast is the typed Markdown AST and the document-level transforms
// that run on it, mirroring src/ast.rs.
package ast

import (
	"github.com/stubbe/html-to-md/internal/text"
)

type Table = [][][]Inline // rows of cells of inlines

type BlockKind uint8

const (
	KindHeading BlockKind = iota
	KindParagraph
	KindList
	KindTable
	KindCode
	KindQuote
	KindRule
)

type Block struct {
	Kind    BlockKind
	Level   int // heading level
	Inlines []Inline
	Ordered bool
	Items   []ListItem
	Rows    Table
	Body    string // code body
	Blocks  []Block
}

type ListItem struct {
	// Number is the 1-based position among the source <li> children; empty
	// items keep their number so ordered lists number like the source.
	Number   int
	Inlines  []Inline
	SubLists []Block
}

// Inline: Text carries normalised source text (escapes decoded, whitespace
// collapsed) that the renderer escapes; Raw holds already-rendered Markdown
// emitted verbatim.
type Inline struct {
	Kind uint8
	// Text/Raw/Code content (Code and Link text are rendered-verbatim rules)
	S     string
	Inner []Inline
	// Text/Href for links
	LinkText string
	Href     string
}

const (
	IText uint8 = iota
	IRaw
	ICode
	IStrong
	IEmph
	ILink
	ILineBreak
)

func T(s string) Inline { return Inline{Kind: IText, S: s} }
func R(s string) Inline { return Inline{Kind: IRaw, S: s} }
func C(s string) Inline { return Inline{Kind: ICode, S: s} }
func Brk() Inline       { return Inline{Kind: ILineBreak} }

// DropEmptySections drops headings that introduce nothing: a heading whose
// following block is another heading at the same or shallower level (e.g. an
// empty `## REVIEWERS` section right before `## NEW ACTIVITY`). Headings
// followed by content, a deeper sub-heading, or end of document are kept.
func DropEmptySections(blocks []Block) []Block {
	levels := make([]int, len(blocks)) // 0 = not a heading
	for i, b := range blocks {
		if b.Kind == KindHeading {
			levels[i] = b.Level
		} else {
			levels[i] = 0
		}
	}
	out := make([]Block, 0, len(blocks))
	for i, b := range blocks {
		lvl := levels[i]
		if lvl == 0 {
			out = append(out, b)
			continue
		}
		if i+1 < len(blocks) && levels[i+1] != 0 && levels[i+1] <= lvl {
			continue
		}
		out = append(out, b)
	}
	return out
}

// CompressHeadingLevels remaps the distinct heading levels actually present
// to a contiguous 1..n range.
func CompressHeadingLevels(blocks []Block) []Block {
	seen := map[int]bool{}
	var used []int
	for _, b := range blocks {
		if b.Kind == KindHeading && !seen[b.Level] {
			seen[b.Level] = true
			used = append(used, b.Level)
		}
	}
	sortInts(used)
	if len(used) < 2 {
		return blocks
	}
	out := make([]Block, len(blocks))
	for i, b := range blocks {
		if b.Kind == KindHeading {
			b.Level = indexOf(used, b.Level) + 1
		}
		out[i] = b
	}
	return out
}

func sortInts(s []int) {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
}

func indexOf(s []int, v int) int {
	for i, x := range s {
		if x == v {
			return i
		}
	}
	return -1
}

// JoinLinkRows joins a run of consecutive blocks that are each a single
// short link into one " · "-separated line (vendor nav bars, badge rows,
// footer link columns). The short-text cap keeps article/post-title link
// lists on their own lines.
func JoinLinkRows(blocks []Block) []Block {
	out := make([]Block, 0, len(blocks))
	var run []Block
	flush := func() {
		if len(run) >= 2 {
			if merged := mergeLinkRun(run); merged != nil {
				out = append(out, Block{Kind: KindParagraph, Inlines: merged})
			}
		} else {
			out = append(out, run...)
		}
		run = nil
	}
	for _, b := range blocks {
		if isShortLink(&b) {
			run = append(run, b)
		} else {
			flush()
			out = append(out, b)
		}
	}
	flush()
	return out
}

func isShortLink(b *Block) bool {
	if b.Kind != KindParagraph || len(b.Inlines) != 1 {
		return false
	}
	in := &b.Inlines[0]
	if in.Kind != ILink {
		return false
	}
	// Bound by display width, not raw characters: CJK and emoji occupy
	// twice the terminal columns, so a 30 *code-point* cap would let wide
	// glyphs wrap a joined nav bar.
	return text.Width(in.LinkText) <= 30
}

// mergeLinkRun concatenates the links of the run; a repeated (label, href)
// pair is dropped (vendors repeat the same CTA twice in one row).
func mergeLinkRun(run []Block) []Inline {
	var merged []Inline
	type key struct{ t, h string }
	seen := map[key]bool{}
	for _, b := range run {
		for _, in := range b.Inlines {
			if in.Kind != ILink {
				continue
			}
			k := key{in.LinkText, in.Href}
			if seen[k] {
				continue
			}
			if len(merged) > 0 {
				merged = append(merged, R(" · "))
			}
			seen[k] = true
			merged = append(merged, in)
		}
	}
	if len(merged) == 0 {
		return nil
	}
	return merged
}
