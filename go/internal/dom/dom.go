// Package dom is a minimal mutable HTML tree built on top of
// golang.org/x/net/html's spec-compliant parser. x/net/html's tree is
// read-only (no parent pointer, no detach), so we convert it once into our
// own structure, which the cleaning passes mutate in place.
package dom

import (
	"strings"

	"golang.org/x/net/html"
)

type NodeType uint8

const (
	ElementNode NodeType = iota
	TextNode
	CommentNode
)

type Node struct {
	Type        NodeType
	Tag         string
	Attrs       []html.Attribute
	Text        string
	Parent      *Node
	FirstChild  *Node
	LastChild   *Node
	PrevSibling *Node
	NextSibling *Node
}

// Parse parses HTML input into a full-document tree (html/head/body as
// html5ever does), so the lower pass can find <body>.
func Parse(input string) *Node {
	parsed, err := html.Parse(strings.NewReader(input))
	if err != nil {
		// html.Parse never fails; guard anyway.
		return &Node{Type: ElementNode}
	}
	return convert(parsed)
}

func convert(n *html.Node) *Node {
	var t NodeType
	switch n.Type {
	case html.ElementNode:
		t = ElementNode
	case html.TextNode:
		t = TextNode
	case html.CommentNode:
		t = CommentNode
	case html.DocumentNode:
	case html.DoctypeNode:
		return nil
	default:
		return nil
	}
	nd := &Node{Type: t, Attrs: n.Attr}
	if t == ElementNode {
		nd.Tag = n.Data
	} else {
		nd.Text = n.Data
	}
	for c := n.FirstChild; c != nil; c = c.NextSibling {
		if k := convert(c); k != nil {
			k.Parent = nd
			if nd.LastChild != nil {
				nd.LastChild.NextSibling = k
				k.PrevSibling = nd.LastChild
				nd.LastChild = k
			} else {
				nd.FirstChild = k
				nd.LastChild = k
			}
		}
	}
	return nd
}

// Children returns a snapshot of the children.
func (n *Node) Children() []*Node {
	var out []*Node
	for c := n.FirstChild; c != nil; c = c.NextSibling {
		out = append(out, c)
	}
	return out
}

// Descendants returns this node and all descendants, depth-first pre-order
// (matching kuchikiki's inclusive_descendants).
func (n *Node) Descendants() []*Node {
	out := []*Node{n}
	for c := n.FirstChild; c != nil; c = c.NextSibling {
		out = append(out, c.Descendants()...)
	}
	return out
}

func (n *Node) IsElement() bool { return n.Type == ElementNode }
func (n *Node) IsText() bool    { return n.Type == TextNode }
func (n *Node) IsComment() bool { return n.Type == CommentNode }

// ParentElement returns the nearest real element ancestor (skipping the
// pseudo #document wrapper).
func (n *Node) ParentElement() *Node {
	for p := n.Parent; p != nil; p = p.Parent {
		if p.IsElement() {
			return p
		}
	}
	return nil
}

// IsTag reports whether n is an element with the given tag name.
func (n *Node) IsTag(name string) bool {
	return n.Type == ElementNode && n.Tag == name
}

// Attr looks up an attribute by name (names are lowercased by the parser).
// ok is false when absent.
func (n *Node) Attr(name string) (string, bool) {
	for _, a := range n.Attrs {
		if a.Key == name {
			return a.Val, true
		}
	}
	return "", false
}

// HasBlockDescendant reports whether a strict descendant is a block
// element (matching clean.rs subtree_has_block, which excludes self).
func (n *Node) HasBlockDescendant() bool {
	return n.SubtreeHasBlock()
}

func (n *Node) SubtreeHasBlock() bool {
	for _, c := range n.Children() {
		if c.IsBlockName() || c.SubtreeHasBlock() {
			return true
		}
	}
	return false
}

// Detach removes n from its parent without touching the subtree.
func (n *Node) Detach() {
	if n.Parent == nil {
		return
	}
	if n.Parent.FirstChild == n {
		n.Parent.FirstChild = n.NextSibling
	}
	if n.Parent.LastChild == n {
		n.Parent.LastChild = n.PrevSibling
	}
	if n.PrevSibling != nil {
		n.PrevSibling.NextSibling = n.NextSibling
	}
	if n.NextSibling != nil {
		n.NextSibling.PrevSibling = n.PrevSibling
	}
	n.Parent = nil
	n.PrevSibling = nil
	n.NextSibling = nil
}

// AppendChild appends ch as n's last child.
func (n *Node) AppendChild(ch *Node) {
	ch.Detach()
	ch.Parent = n
	ch.PrevSibling = n.LastChild
	ch.NextSibling = nil
	if n.LastChild != nil {
		n.LastChild.NextSibling = ch
		n.LastChild = ch
	} else {
		n.FirstChild = ch
		n.LastChild = ch
	}
}

// AppendText appends a text node, merging into the existing last text child.
func (n *Node) AppendText(s string) {
	for _, c := range n.Children() {
		if c.IsText() {
			c.Text += s
			return
		}
	}
	n.AppendChild(&Node{Type: TextNode, Text: s})
}

// InsertBefore inserts new node(s) immediately before n.
func InsertBefore(n *Node, news ...*Node) {
	for _, m := range news {
		n.InsertBefore(m)
	}
}

func (n *Node) InsertBefore(m *Node) {
	m.Detach()
	m.Parent = n.Parent
	m.PrevSibling = n.PrevSibling
	m.NextSibling = n
	if n.PrevSibling != nil {
		n.PrevSibling.NextSibling = m
	} else if n.Parent != nil {
		n.Parent.FirstChild = m
	}
	n.PrevSibling = m
}

func NewElement(tag string) *Node {
	return &Node{Type: ElementNode, Tag: tag}
}

func NewText(text string) *Node {
	return &Node{Type: TextNode, Text: text}
}
