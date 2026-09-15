package table

import (
	"slices"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/stubbe/html-to-md/internal/dom"
	"github.com/stubbe/html-to-md/internal/text"
)

// FlattenOne rewrites a single layout table into paragraphs, inserted right
// before the table, which is then detached.
//
// Each row's cells are walked: inline runs (text + inline elements, plus
// cells that wrap their content in a single <p>/<div>) accumulate into one
// paragraph spanning the row, joined by single spaces. Block kids — a
// <table>, lists, headings, multiple sibling <p>s — emit as standalone
// siblings so their structure survives.
func FlattenOne(table *dom.Node) {
	rows := CollectRows(table)
	var emitted []*dom.Node
	for _, tr := range rows {
		var cells []*dom.Node
		for _, k := range tr.Children() {
			if k.IsTag("td") || k.IsTag("th") {
				cells = append(cells, k)
			}
		}
		if len(cells) == 0 {
			continue
		}
		var rowP *dom.Node
		flushRowP := func() {
			if rowP != nil {
				if tr := dom.SubtreeText(rowP); trimIsNonEmpty(tr) {
					emitted = append(emitted, rowP)
				}
				rowP = nil
			}
		}
		for _, cell := range cells {
			var kids = cell.Children()
			var nonBlank []*dom.Node
			for _, k := range kids {
				if !dom.IsBlank(k) {
					nonBlank = append(nonBlank, k)
				}
			}
			if len(nonBlank) == 0 {
				continue
			}
			switch mode, items := classifyCell(nonBlank); mode {
			case cellInline:
				if rowP == nil {
					rowP = dom.MakeParagraph()
				}
				if !endsWithWhitespace(rowP) && rowP.FirstChild != nil {
					rowP.AppendChild(dom.NewText(" "))
				}
				// Preserve whitespace-only separators between retained nodes
				// (they keep "X (n)" from glueing to "Y" in vendor legends).
				for _, n := range includeInlineWhitespace(kids, items) {
					rowP.AppendChild(n)
				}
			case cellBlocks:
				flushRowP()
				emitted = append(emitted, items...)
			case cellParagraph:
				flushRowP()
				p := dom.MakeParagraph()
				for _, n := range items {
					p.AppendChild(n)
				}
				emitted = append(emitted, p)
			}
		}
		flushRowP()
	}

	// If every flattened row is a short single-line inline paragraph, this is
	// a key-value / price-summary / spec table: join rows tight with <br> so
	// they render as a compact block. Any long or block-bearing row leaves
	// the whole table as a stack of paragraphs.
	if len(emitted) >= 2 && allShortInline(emitted) {
		group := dom.MakeParagraph()
		for i, n := range emitted {
			if i > 0 {
				group.AppendChild(dom.MakeBr())
			}
			for _, child := range n.Children() {
				group.AppendChild(child)
			}
		}
		emitted = []*dom.Node{group}
	}

	for _, n := range emitted {
		table.InsertBefore(n)
	}
	table.Detach()
}

func trimIsNonEmpty(s string) bool {
	return strings.TrimSpace(s) != ""
}

func allShortInline(nodes []*dom.Node) bool {
	for _, n := range nodes {
		if !isShortInlineParagraph(n) {
			return false
		}
	}
	return true
}

const maxShortLine = 60 // display-width bound for a one-line spec-table row

// isShortInlineParagraph: a <p> holding one short line of inline content
// (no block descendant, no <br>): one row of a key-value/spec table.
func isShortInlineParagraph(n *dom.Node) bool {
	if !n.IsTag("p") || n.HasBlockDescendant() || subtreeHasBr(n) {
		return false
	}
	// Bounded by display width, not character count, so CJK-heavy rows
	// (often wider than they are long) don't slip past the cap.
	body := strings.TrimSpace(dom.SubtreeText(n))
	w := text.Width(body)
	return w > 0 && w <= maxShortLine
}

type cellMode uint8

const (
	cellInline cellMode = iota
	cellBlocks
	// cellParagraph: the cell's inline content spans multiple <br> lines;
	// emit it as one standalone multi-line paragraph, kept tight and not
	// merged with siblings.
	cellParagraph
)

func classifyCell(nonBlank []*dom.Node) (cellMode, []*dom.Node) {
	if slices.ContainsFunc(nonBlank, subtreeHasBr) {
		return cellParagraph, nonBlank
	}
	allInline := !slices.ContainsFunc(nonBlank, dom.IsBlockKid)
	if allInline {
		return cellInline, nonBlank
	}
	if len(nonBlank) == 1 && nonBlank[0].IsElement() {
		only := nonBlank[0]
		if only.Tag == "p" || only.Tag == "div" {
			grandkids := only.Children()
			var gk []*dom.Node
			for _, k := range grandkids {
				if !dom.IsBlank(k) {
					gk = append(gk, k)
				}
			}
			if !slices.ContainsFunc(gk, dom.IsBlockKid) {
				return cellInline, grandkids
			}
			return cellBlocks, gk
		}
	}
	return cellBlocks, nonBlank
}

// includeInlineWhitespace re-threads whitespace-only text nodes from kids
// into items whenever they sit between two retained nodes: inline runs need
// their inter-element whitespace or the serialiser glues neighbouring text
// together. When items came from a wrapper's grandchildren (classifyCell
// unwrapping <p>/<div>) it is not a subsequence of kids; fall back to items
// untouched.
func includeInlineWhitespace(kids, items []*dom.Node) []*dom.Node {
	if len(items) == 0 {
		return nil
	}
	isSubseq := func() bool {
		i := 0
		for _, k := range kids {
			if i < len(items) && items[i] == k {
				i++
			}
		}
		return i == len(items)
	}()
	if !isSubseq {
		return items
	}
	var out []*dom.Node
	i := 0
	started := false
	lastWasItem := false
	for _, k := range kids {
		if i < len(items) && items[i] == k {
			out = append(out, items[i])
			i++
			started = true
			lastWasItem = true
		} else if started && lastWasItem && dom.IsBlank(k) {
			if i < len(items) {
				out = append(out, k)
			}
			lastWasItem = false
		}
	}
	return out
}

func subtreeHasBr(n *dom.Node) bool {
	for _, d := range n.Descendants() {
		if d.IsTag("br") {
			return true
		}
	}
	return false
}

// endsWithWhitespace reports whether n's rendered content ends in
// whitespace (an empty node needs no separator).
func endsWithWhitespace(n *dom.Node) bool {
	last := n.LastChild
	if last == nil {
		return true
	}
	if last.IsText() {
		r, _ := utf8.DecodeLastRuneInString(last.Text)
		return unicode.IsSpace(r)
	}
	return false
}
