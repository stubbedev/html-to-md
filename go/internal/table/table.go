// Package table: layout-table flattening. Most marketing/notification HTML
// uses <table> for column layout; real data tables are kept as pipe tables.
// This module decides which is which and rewrites layout tables in place.
package table

import (
	"slices"
	"strconv"
	"strings"

	"github.com/stubbe/html-to-md/internal/dom"
)

// Flatten rewrites layout tables into paragraphs in place. Tables are
// visited deepest-first so an outer table's cells already contain the
// paragraph rewrites of inner tables before we look at it.
// Flatten collects the tables deepest-first so an outer table's cells
// already contain paragraph rewrites of any inner tables before we look
// at the outer one.
// Flatten collects the tables deepest-first and rewrites layout tables in
// place, swapping their cells for paragraphs.
func Flatten(rootp *dom.Node) {
	type entry struct {
		depth int
		node  *dom.Node
	}
	var tables []entry
	for _, n := range rootp.Descendants() {
		if n.IsTag("table") {
			tables = append(tables, entry{dom.Depth(n), n})
		}
	}
	slices.SortFunc(tables, func(a, b entry) int { return b.depth - a.depth })
	for _, t := range tables {
		switch {
		case t.node.Parent == nil:
			// Already swallowed by an outer rewrite.
		case strings.TrimSpace(dom.SubtreeText(t.node)) == "":
			t.node.Detach()
		case IsDataTable(t.node):
			// Keep data tables.
		default:
			FlattenOne(t.node)
		}
	}
}

// IsDataTable implements the layout/data heuristic: default to layout and
// treat tables as data only when there's positive evidence — <thead> or
// <caption> belonging to this table, or uniform ≥2-cell rows with a real
// border attribute. role="presentation"/"none" always wins as layout.
func IsDataTable(t *dom.Node) bool {
	if role, ok := t.Attr("role"); ok {
		r := strings.ToLower(strings.TrimSpace(role))
		if r == "presentation" || r == "none" {
			return false
		}
	}
	if hasOwnDescendant(t, "thead") || hasOwnDescendant(t, "caption") {
		return true
	}
	if hasNestedTable(t) {
		return false
	}
	rows := CollectRows(t)
	if len(rows) < 2 {
		return false
	}
	mx, mn := 0, 1<<62
	for _, tr := range rows {
		c := CountCells(tr)
		if c > mx {
			mx = c
		}
		if c < mn {
			mn = c
		}
	}
	if mx < 2 {
		return false
	}
	border, ok := t.Attr("border")
	if !ok || border == "" {
		return false
	}
	hasBorder, err := strconv.Atoi(border)
	if err != nil {
		return true
	}
	return mn == mx && hasBorder > 0
}

// hasOwnDescendant matches descendants whose nearest <table> ancestor is
// root itself; a layout wrapper must not pick up <th>/<thead>/<caption>
// from a nested data table.
func hasOwnDescendant(rootp *dom.Node, tag string) bool {
	found := false
	for _, n := range rootp.Descendants() {
		if found {
			break
		}
		if n.IsTag(tag) && nearestTableAncestor(n) == rootp {
			found = true
		}
	}
	return found
}

func hasNestedTable(t *dom.Node) bool {
	for _, n := range t.Descendants() {
		if n.IsTag("table") {
			return true
		}
	}
	return false
}

// CollectRows returns the <tr>s whose nearest <table> ancestor is t itself,
// so an outer layout table doesn't sweep in the rows of a nested data table.
func CollectRows(t *dom.Node) []*dom.Node {
	var rows []*dom.Node
	for _, n := range t.Descendants() {
		if n.IsTag("tr") && nearestTableAncestor(n) == t {
			rows = append(rows, n)
		}
	}
	return rows
}

func nearestTableAncestor(n *dom.Node) *dom.Node {
	for p := n.ParentElement(); p != nil; p = p.ParentElement() {
		if p.IsTag("table") {
			return p
		}
	}
	return nil
}

// CountCells counts direct <td>/<th> children of a row.
func CountCells(tr *dom.Node) int {
	c := 0
	for _, k := range tr.Children() {
		if k.IsTag("td") || k.IsTag("th") {
			c++
		}
	}
	return c
}
