// Package lower: walk the cleaned DOM and build the typed Markdown AST.
// Every heuristic the old string pipeline re-parsed with regexes (link
// fallbacks, cell classification, list numbering) is made here on typed
// nodes.
package lower

import (
	"strings"

	"github.com/stubbe/html-to-md/internal/ast"
	"github.com/stubbe/html-to-md/internal/dom"
	"github.com/stubbe/html-to-md/internal/render"
)

// MinHeadingLevel finds the shallowest heading level with non-empty text
// content; 7 when there is none.
func MinHeadingLevel(rootp *dom.Node) int {
	min := 7
	for _, n := range rootp.Descendants() {
		lvl, ok := headingLevel(n)
		if !ok {
			continue
		}
		if strings.TrimSpace(dom.SubtreeText(n)) == "" {
			continue
		}
		if lvl < min {
			min = lvl
		}
	}
	return min
}

func headingLevel(n *dom.Node) (int, bool) {
	if !n.IsElement() {
		return 0, false
	}
	switch n.Tag {
	case "h1", "h2", "h3", "h4", "h5", "h6":
		return headingLevelOf(n.Tag), true
	}
	return 0, false
}

// Document lowers the document starting from <body> (or the root fragment).
func Document(rootp *dom.Node, shift int) []ast.Block {
	start := dom.FindBody(rootp)
	return childrenBlocks(start, shift)
}

// childrenBlocks lowers an element's children: consecutive inline children
// group into one paragraph; block children emit on their own.
func childrenBlocks(node *dom.Node, shift int) []ast.Block {
	var out []ast.Block
	var run []ast.Inline
	flush := func() {
		if len(run) == 0 {
			return
		}
		s := render.TidyParagraph(render.InlinesToString(run))
		if s != "" {
			out = append(out, ast.Block{Kind: ast.KindParagraph, Inlines: run})
		}
		run = nil
	}
	for _, c := range node.Children() {
		isBlock := c.IsBlockName() || subtreeHasBlock2(c)
		if isBlock {
			flush()
			out = append(out, block(c, shift)...)
		} else {
			run = append(run, inlineNode(c)...)
		}
	}
	flush()
	return out
}

func subtreeHasBlock2(n *dom.Node) bool {
	return n.SubtreeHasBlock()
}

func headingLevelOf(tag string) int {
	switch tag {
	case "h1":
		return 1
	case "h2":
		return 2
	case "h3":
		return 3
	case "h4":
		return 4
	case "h5":
		return 5
	}
	return 6
}

func clamp(v, lo, hi int) int {
	if v < lo {
		return lo
	}
	if v > hi {
		return hi
	}
	return v
}

func block(node *dom.Node, shift int) []ast.Block {
	if !node.IsElement() {
		return nil
	}
	switch node.Tag {
	case "html", "body":
		return childrenBlocks(node, shift)

	case "p", "div", "center", "header", "footer", "section", "article",
		"main", "aside", "nav", "form", "fieldset":
		if node.HasBlockChild() {
			return childrenBlocks(node, shift)
		}
		return paragraphFromChildren(node)

	case "h1", "h2", "h3", "h4", "h5", "h6":
		level := clamp(headingLevelOf(node.Tag)-shift, 1, 6)
		// A heading is already visually prominent: emphasis markers inside
		// it are dropped and any <br> newline folds to a single space.
		inlines := stripEmphasis(inlineChildren(node))
		if s := render.NormalizeWs(render.InlinesToString(inlines)); s != "" {
			return []ast.Block{{Kind: ast.KindHeading, Level: level, Inlines: inlines}}
		}
		return nil

	case "hr":
		return []ast.Block{{Kind: ast.KindRule}}

	case "pre":
		// Strip per-line trailing whitespace: source <pre> pads every line
		// to a fixed column, rendering as ragged trailing space. Leading
		// indentation is kept.
		txt := dom.SubtreeText(node)
		var lines []string
		for _, l := range splitLines(txt) {
			lines = append(lines, trimRightKeep(l))
		}
		body := strings.TrimSpace(strings.Join(lines, "\n"))
		if body == "" {
			return nil
		}
		return []ast.Block{{Kind: ast.KindCode, Body: body}}

	case "blockquote":
		inner := childrenBlocks(node, shift)
		if len(inner) == 0 {
			return nil
		}
		return []ast.Block{{Kind: ast.KindQuote, Blocks: inner}}

	case "ul":
		return list(node, false)
	case "ol":
		return list(node, true)

	case "table":
		return tableBlock(node)

	default:
		if subtreeHasBlock2(node) {
			return childrenBlocks(node, shift)
		}
		return paragraphFromChildren(node)
	}
}

func paragraphFromChildren(node *dom.Node) []ast.Block {
	inlines := inlineChildren(node)
	s := render.TidyParagraph(render.InlinesToString(inlines))
	if s == "" {
		return nil
	}
	return []ast.Block{{Kind: ast.KindParagraph, Inlines: inlines}}
}

// stripEmphasis recursively drops emphasis wrappers, keeping content. Used
// for headings: `#` already carries visual weight.
func stripEmphasis(inlines []ast.Inline) []ast.Inline {
	out := make([]ast.Inline, 0, len(inlines))
	for _, i := range inlines {
		switch i.Kind {
		case ast.IStrong, ast.IEmph:
			out = append(out, stripEmphasis(i.Inner)...)
		default:
			out = append(out, i)
		}
	}
	return out
}

func splitLines(s string) []string {
	s = strings.TrimSuffix(s, "\n")
	if s == "" {
		return nil
	}
	return strings.Split(s, "\n")
}

func trimRightKeep(l string) string {
	return strings.TrimRight(l, " \t")
}
