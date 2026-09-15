// Package clean: the pre-parse text fix-ups and every in-place cleaning pass
// the lower pass relies on. The pass order in Doc is load-bearing.
package clean

import (
	"regexp"
	"slices"
	"strings"
	"unicode"

	"github.com/stubbe/html-to-md/internal/dom"
	"github.com/stubbe/html-to-md/internal/table"
	"github.com/stubbe/html-to-md/internal/text"
)

// Doc runs steps 1-3 of the pipeline: strip IE conditionals, parse, run
// every DOM surgery pass.
func Doc(input string) *dom.Node {
	input = StripIEConditionals(input)
	docp := dom.Parse(input)

	StripComments(docp)
	// HTML5 parsing keeps Outlook/Word namespaced tags (o:p, w:WordDocument,
	// v:shape, …) as elements with a literal colon in the name.
	DropElements(docp, func(el *dom.Node) bool {
		return strings.Contains(el.Tag, ":")
	})
	// Responsive emails duplicate content: desktop + mobile versions toggled
	// only with CSS. Since stylesheets are stripped, both would render.
	DropElements(docp, func(el *dom.Node) bool { return isHidden(el) })
	DropElements(docp, func(el *dom.Node) bool {
		switch el.Tag {
		case "head", "style", "script", "iframe", "img", "colgroup", "col",
			"figure", "picture", "source", "svg", "canvas", "video", "audio",
			"area", "map", "noscript":
			return true
		}
		return false
	})
	// Must run before DropEmptyAnchors so anchors padded with ZWSPs etc.
	// become text-empty.
	NormaliseTextNodes(docp)
	FlattenLinkText(docp)
	UnwrapPunctuationEmphasis(docp)
	DemoteStatHeadings(docp)
	InlineFlexRowDivs(docp)
	table.Flatten(docp)
	// Marketing emails wrap a brand logo in <a href="…"><img></a>; once the
	// <img> is dropped, the anchor has no visible text left.
	DropEmptyAnchors(docp)
	return docp
}

// StripIEConditionals removes non-comment IE conditionals (<![if …]>…
// <![endif]>) before parsing so Outlook bullet spans and other Outlook-only
// blocks don't appear in the DOM as regular text. Standard comment-form
// conditionals are handled by the parser's bogus comments + StripComments.
var ieRe = regexp.MustCompile(`(?si)<!\[if[^]]*\]>.*?<!\[endif]>`)

func StripIEConditionals(html string) string { return ieRe.ReplaceAllString(html, "") }

func isHidden(el *dom.Node) bool {
	style, ok := el.Attr("style")
	if !ok {
		return false
	}
	s := strings.ToLower(style)
	return strings.Contains(s, "display:none") ||
		strings.Contains(s, "display: none") ||
		strings.Contains(s, "visibility:hidden") ||
		strings.Contains(s, "visibility: hidden")
}

func NormaliseTextNodes(rootp *dom.Node) {
	for _, n := range rootp.Descendants() {
		if n.IsText() {
			n.Text = text.CleanInvisibles(n.Text)
		}
	}
}

func StripComments(rootp *dom.Node) {
	for _, n := range rootp.Descendants() {
		if n.IsComment() {
			n.Detach()
		}
	}
}

func DropElements(rootp *dom.Node, pred func(*dom.Node) bool) {
	for _, n := range rootp.Descendants() {
		if n.IsElement() && pred(n) {
			n.Detach()
		}
	}
}

func DropEmptyAnchors(rootp *dom.Node) {
	for _, a := range rootp.Descendants() {
		if a.IsTag("a") {
			trimmed := strings.TrimSpace(dom.SubtreeText(a))
			if trimmed == "" || text.IsDecorativeGlyph(trimmed) {
				a.Detach()
			}
		}
	}
}

func isEmphTag(tag string) bool {
	switch tag {
	case "em", "i", "strong", "b", "u", "mark", "small":
		return true
	}
	return false
}

// UnwrapPunctuationEmphasis removes emphasis tags whose textual content is
// purely punctuation (≤ 3 chars, no letters/digits). Sentry tag rows wrap a
// literal `=` in <em>, which renders as italic markers around an escaped
// equals — visible noise.
func UnwrapPunctuationEmphasis(rootp *dom.Node) {
	for _, el := range rootp.Descendants() {
		if !el.IsElement() || !isEmphTag(el.Tag) {
			continue
		}
		t := dom.SubtreeText(el)
		trimmed := strings.TrimSpace(t)
		if trimmed == "" {
			// Whitespace-only emphasis (<em> </em>) often glues two adjacent
			// inlines together. Merge the space into an adjacent text sibling
			// so it survives the blank-node filters.
			if t != "" {
				mergeSeparatorSpace(el)
			}
			el.Detach()
			continue
		}
		runes := []rune(trimmed)
		if len(runes) <= 3 && allNonWord(runes) {
			for _, k := range el.Children() {
				el.InsertBefore(k)
			}
			el.Detach()
		}
	}
}

func allNonWord(rs []rune) bool {
	for _, c := range rs {
		if unicode.IsLetter(c) || unicode.IsDigit(c) || unicode.IsSpace(c) {
			return false
		}
	}
	return true
}

// mergeSeparatorSpace pushes a separator space onto an adjacent text sibling
// of el (preferring the previous sibling so a leading space doesn't start a
// new "blank" line), falling back to next sibling, then a new text node.
func mergeSeparatorSpace(el *dom.Node) {
	// Detach to inspect siblings without matching the node itself.
	parent := el.Parent
	if parent == nil {
		return
	}
	var prev, next *dom.Node
	for _, c := range parent.Children() {
		if c == el {
			continue
		}
		if el.PrevSibling != nil && isBefore(c, el) {
			prev = c
		} else if el.NextSibling != nil && isBefore(el, c) {
			if next == nil {
				next = c
			}
		}
	}
	_ = prev
	_ = next
	// Simpler direct walk:
	p, n := directSiblings(el)
	if p != nil && p.IsText() {
		if !strings.HasSuffix(p.Text, " ") {
			p.Text += " "
		}
		return
	}
	if n != nil && n.IsText() {
		if !strings.HasPrefix(n.Text, " ") {
			n.Text = " " + n.Text
		}
		return
	}
	el.InsertBefore(dom.NewText(" "))
}

// directSiblings finds the previous/next real siblings of el within its
// parent (nil when absent).
func directSiblings(el *dom.Node) (prev, next *dom.Node) {
	var lastReal *dom.Node
	seen := false
	for _, c := range el.Parent.Children() {
		if c == el {
			prev = lastReal
			seen = true
			continue
		}
		if seen {
			next = c
			return
		}
		lastReal = c
	}
	return
}

func isBefore(a, b *dom.Node) bool { return false } // unused placeholder

// DemoteStatHeadings rewrites short numeric-only heading content
// (<h1>471k</h1>) to <p><strong>…</strong></p> so the level normaliser
// ignores it.
func DemoteStatHeadings(rootp *dom.Node) {
	for _, h := range rootp.Descendants() {
		if !h.IsElement() || !isHeadingTag(h.Tag) {
			continue
		}
		trimmed := strings.TrimSpace(dom.SubtreeText(h))
		const maxWidth = 12
		if trimmed == "" || text.Width(trimmed) > maxWidth {
			continue
		}
		if !isStatText(trimmed) {
			continue
		}
		para := dom.MakeParagraph()
		strong := dom.NewElement("strong")
		para.AppendChild(strong)
		for _, k := range h.Children() {
			strong.AppendChild(k)
		}
		h.InsertBefore(para)
		h.Detach()
	}
}

func isHeadingTag(tag string) bool {
	switch tag {
	case "h1", "h2", "h3", "h4", "h5", "h6":
		return true
	}
	return false
}

func isStatText(s string) bool {
	sawDigit := false
	for _, c := range s {
		if c >= '0' && c <= '9' {
			sawDigit = true
			continue
		}
		switch c {
		case '.', ',', ' ', 'k', 'K', 'M', 'B', 'm', 's', 'µ', 'h':
		default:
			return false
		}
	}
	return sawDigit
}

// InlineFlexRowDivs unwraps <div> children of CSS flex-row containers that
// hold only inline content, so each row collapses to a single paragraph.
func InlineFlexRowDivs(rootp *dom.Node) {
	const maxFlexDirectChildren = 8
	var flexParents []*dom.Node
	for _, n := range rootp.Descendants() {
		if !isFlexDiv(n) {
			continue
		}
		if hasFlexAncestor(n) {
			continue
		}
		direct := 0
		for _, c := range n.Children() {
			if c.IsElement() {
				direct++
			}
		}
		if direct >= 2 && direct <= maxFlexDirectChildren {
			flexParents = append(flexParents, n)
		}
	}

	type target struct {
		depth int
		node  *dom.Node
	}
	var targets []target
	for _, parent := range flexParents {
		for _, d := range parent.Descendants() {
			if d.IsTag("div") {
				targets = append(targets, target{dom.Depth(d), d})
			}
		}
	}
	// Deepest first (children unmounted before parents are examined).
	slices.SortFunc(targets, func(a, b target) int { return b.depth - a.depth })
	for _, t := range targets {
		d := t.node
		if d.Parent == nil {
			continue
		}
		if subtreeHasBlockElem(d) {
			continue
		}
		for _, c := range d.Children() {
			d.InsertBefore(c)
		}
		d.InsertBefore(dom.NewText(" "))
		d.Detach()
	}
}

func hasFlexAncestor(n *dom.Node) bool {
	for p := n.ParentElement(); p != nil; p = p.ParentElement() {
		if isFlexDiv(p) {
			return true
		}
	}
	return false
}

func isFlexDiv(n *dom.Node) bool {
	if !n.IsTag("div") {
		return false
	}
	style, ok := n.Attr("style")
	if !ok {
		return false
	}
	return strings.Contains(style, "display: flex") || strings.Contains(style, "display:flex")
}

func subtreeHasBlockElem(n *dom.Node) bool {
	return n.SubtreeHasBlock()
}

// FlattenLinkText squashes any newline/tab inside <a> text to a single
// space: multi-line link display breaks the atomic [text](href) token.
func FlattenLinkText(rootp *dom.Node) {
	for _, a := range rootp.Descendants() {
		if !a.IsTag("a") {
			continue
		}
		for _, t := range a.Descendants() {
			if t.IsText() && (strings.Contains(t.Text, "\n") || strings.Contains(t.Text, "\t")) {
				t.Text = strings.Map(func(c rune) rune {
					if c == '\n' || c == '\t' {
						return ' '
					}
					return c
				}, t.Text)
			}
		}
	}
}
