package lower

import (
	"strings"

	"github.com/stubbe/html-to-md/internal/ast"
	"github.com/stubbe/html-to-md/internal/dom"
	"github.com/stubbe/html-to-md/internal/render"
)

// list lowers a <ul>/<ol>: collect inline text and nested sub-lists from
// each <li>; skipped empty items keep their number.
func list(listNode *dom.Node, ordered bool) []ast.Block {
	var items []ast.ListItem
	n := 0
	for _, child := range listNode.Children() {
		if !child.IsTag("li") {
			continue
		}
		n++
		var inlines []ast.Inline
		var subLists []ast.Block

		for _, kid := range child.Children() {
			switch {
			case kid.IsTag("ul"):
				subLists = append(subLists, list(kid, false)...)
			case kid.IsTag("ol"):
				subLists = append(subLists, list(kid, true)...)
			case dom.IsBlockKid(kid):
				// <p>/<div> inside li: gather as inline text
				s := render.InlinesToString(inlineChildren(kid))
				if t := trimNormalized(s); t != "" {
					inlines = append(inlines, ast.R(t))
				}
			default:
				inlines = append(inlines, inlineNode(kid)...)
			}
		}

		textEmpty := render.NormalizeWs(render.InlinesToString(inlines)) == ""
		if textEmpty && len(subLists) == 0 {
			continue
		}
		items = append(items, ast.ListItem{Number: n, Inlines: inlines, SubLists: subLists})
	}
	if len(items) == 0 {
		return nil
	}
	return []ast.Block{{Kind: ast.KindList, Ordered: ordered, Items: items}}
}

// tableBlock lowers a data table; fewer than two rows is layout residue or
// an empty frame, and renders nothing.
func tableBlock(tableNode *dom.Node) []ast.Block {
	rowsDom := tableRows(tableNode)
	if len(rowsDom) < 2 {
		return nil
	}

	var parsed ast.Table
	for _, tr := range rowsDom {
		var row [][]ast.Inline
		for _, k := range tr.Children() {
			if k.IsTag("td") || k.IsTag("th") {
				// A <br>-newline inside a cell would break the table row;
				// FlattenLinkText already squashed those to spaces, and cell
				// rendering collapses all whitespace runs. Links never
				// contain spaces, so this can't split one.
				row = append(row, inlineChildren(k))
			}
		}
		parsed = append(parsed, row)
	}

	ncols := 0
	for _, r := range parsed {
		ncols = max(ncols, len(r))
	}
	if ncols == 0 {
		return nil
	}

	// Drop columns where every cell is empty.
	var keep []int
	for c := range ncols {
		for _, row := range parsed {
			if c < len(row) && render.NormalizeWs(render.InlinesToString(row[c])) != "" {
				keep = append(keep, c)
				break
			}
		}
	}
	if len(keep) < ncols && len(keep) > 0 {
		var pruned ast.Table
		for _, row := range parsed {
			var nr [][]ast.Inline
			for _, c := range keep {
				nr = append(nr, row[c])
			}
			pruned = append(pruned, nr)
		}
		parsed = pruned
	}
	ncols = 0
	for _, r := range parsed {
		ncols = max(ncols, len(r))
	}
	if ncols == 0 {
		return nil
	}

	return []ast.Block{{Kind: ast.KindTable, Rows: parsed}}
}

// tableRows returns the <tr>s whose nearest <table> ancestor is t itself,
// so an outer layout table doesn't sweep in rows from a nested data table.
func tableRows(t *dom.Node) []*dom.Node {
	var rows []*dom.Node
	for _, n := range t.Descendants() {
		if n.IsTag("tr") && nearestTable(n) == t {
			rows = append(rows, n)
		}
	}
	return rows
}

func nearestTable(n *dom.Node) *dom.Node {
	for p := n.Parent; p != nil; p = p.Parent {
		if p.IsTag("table") {
			return p
		}
	}
	return nil
}

func trimNormalized(s string) string {
	return strings.TrimSpace(render.NormalizeWs(s))
}
