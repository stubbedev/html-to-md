package dom

import "strings"

// Shared DOM queries used by the cleaning, table and lowering passes.

// SubtreeText concatenates every text node in n's subtree.
func SubtreeText(n *Node) string {
	var b strings.Builder
	for _, d := range n.Descendants() {
		if d.IsText() {
			b.WriteString(d.Text)
		}
	}
	return b.String()
}

// Depth counts ancestors (0 for a root node), matching clean.rs depth().
func Depth(n *Node) int {
	d := 0
	for p := n.Parent; p != nil; p = p.Parent {
		d++
	}
	return d
}

func IsBlockName(name string) bool {
	switch name {
	case "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "hr",
		"pre", "blockquote", "table", "header", "footer", "section",
		"article", "main", "aside", "nav", "center":
		return true
	}
	return false
}

func (n *Node) IsBlockName() bool {
	return n.IsElement() && IsBlockName(n.Tag)
}

// HasBlockChild reports whether n has an element child whose tag is a block
// name.
func (n *Node) HasBlockChild() bool {
	for _, c := range n.Children() {
		if c.IsBlockName() {
			return true
		}
	}
	return false
}

func IsBlockKid(n *Node) bool {
	if !n.IsElement() {
		return false
	}
	switch n.Tag {
	case "p", "div", "table", "ul", "ol", "li", "h1", "h2", "h3", "h4",
		"h5", "h6", "pre", "blockquote", "hr":
		return true
	}
	return false
}

func MakeParagraph() *Node {
	return NewElement("p")
}

func MakeBr() *Node {
	return NewElement("br")
}

// FindBody returns the <body> element among descendants, or n itself.
func FindBody(n *Node) *Node {
	for _, d := range n.Descendants() {
		if d.IsTag("body") {
			return d
		}
	}
	return n
}

func IsBlank(n *Node) bool {
	if n.IsText() {
		return strings.TrimSpace(n.Text) == ""
	}
	return n.IsComment()
}
