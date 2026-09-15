package lower

import (
	"strings"
	"unicode"

	"github.com/stubbe/html-to-md/internal/ast"
	"github.com/stubbe/html-to-md/internal/dom"
	"github.com/stubbe/html-to-md/internal/render"
	"github.com/stubbe/html-to-md/internal/text"
)

// inlineChildren lowers the children of one element into an inline run.
// The marker-adjacency merge applies within one element's children, never
// across separately-lowered sibling nodes.
func inlineChildren(node *dom.Node) []ast.Inline {
	var parts []ast.Inline
	for _, c := range node.Children() {
		parts = append(parts, inlineNode(c)...)
	}
	return mergeMarkerRuns(parts)
}

func inlineNode(node *dom.Node) []ast.Inline {
	if node.IsText() {
		return []ast.Inline{ast.T(normaliseText(node.Text))}
	}
	if !node.IsElement() {
		return nil
	}
	switch node.Tag {
	case "a":
		return linkInline(node)
	case "strong", "b":
		return []ast.Inline{{Kind: ast.IStrong, Inner: inlineChildren(node)}}
	case "em", "i":
		return []ast.Inline{{Kind: ast.IEmph, Inner: inlineChildren(node)}}
	case "code":
		if s := strings.TrimSpace(dom.SubtreeText(node)); s != "" {
			return []ast.Inline{ast.C(s)}
		}
		return nil
	// <br> is an intentional line break — rendered as a real newline so
	// signatures, address blocks and log dumps stay tight. Two in a row
	// become a blank line, matching the source's intent. Contexts where a
	// raw newline is harmful sanitise it: emphasis/links split or fold it;
	// table cells, headings and list items flatten it.
	case "br":
		return []ast.Inline{ast.Brk()}
	default:
		return inlineChildren(node)
	}
}

// linkInline lowers an <a>. The display is a single line (any <br> folded
// to a space), garbage hrefs degrade to plain text, and self-referential
// links render bare.
func linkInline(node *dom.Node) []ast.Inline {
	inner := render.NormalizeWs(render.InlinesToString(inlineChildren(node)))
	if inner == "" || text.IsDecorativeGlyph(inner) {
		return nil
	}
	rawHref, ok := node.Attr("href")
	href := ""
	if ok {
		href = text.StripTrackingParams(rawHref)
	}
	if href == "" {
		return []ast.Inline{ast.R(inner)}
	}
	// Garbage href: broken templates stuff text or markup into the
	// attribute. A real URL has no whitespace or angle brackets — drop the
	// link syntax and keep the visible text rather than emit a broken
	// [text](url with <br> and spaces).
	if strings.ContainsFunc(href, func(c rune) bool {
		return isHarmful(c)
	}) {
		return []ast.Inline{ast.R(inner)}
	}
	// [url](url) → bare url; [email](mailto:email) → bare email
	bareHref := strings.TrimSuffix(strings.TrimPrefix(href, "mailto:"), "/")
	if strings.TrimSuffix(inner, "/") == bareHref {
		return []ast.Inline{ast.R(inner)}
	}
	// If the display already contains markdown link syntax, use the plain
	// subtree text to avoid nested [[...](url)](url).
	if strings.Contains(inner, "](") {
		fallback := strings.Join(strings.Fields(dom.SubtreeText(node)), " ")
		if fallback == "" {
			return nil
		}
		return []ast.Inline{ast.R(fallback)}
	}
	return []ast.Inline{{Kind: ast.ILink, LinkText: inner, Href: href}}
}

func isHarmful(c rune) bool {
	return unicode.IsSpace(c) || c == '<' || c == '>'
}

// normaliseText decodes literal unicode escape sequences and collapses
// whitespace runs, matching the browser's whitespace collapsing. Markdown
// escaping happens later, at render time.
func normaliseText(s string) string {
	s = text.DecodeUnicodeEscapes(s)
	var b strings.Builder
	b.Grow(len(s))
	prevSpace := false
	for _, c := range s {
		switch c {
		case ' ', '\t', '\n', '\r', '\u00A0':
			if !prevSpace {
				b.WriteByte(' ')
				prevSpace = true
			}
		default:
			prevSpace = false
			b.WriteRune(c)
		}
	}
	return b.String()
}

// mergeMarkerRuns concatenates adjacent same-marker emphasis with no
// separating whitespace — `<b>So, w</b><b>atch this</b>` splitting a word
// mid-token — merging to `**So, watch this**`. Runs whose rendered side
// carries a space are left alone: the space lives outside the markers.
func mergeMarkerRuns(parts []ast.Inline) []ast.Inline {
	// Empty-rendering emphasis contributes nothing; drop it so adjacency is
	// literal.
	kept := parts[:0]
	for _, p := range parts {
		if render.InlinesToString([]ast.Inline{p}) != "" {
			kept = append(kept, p)
		}
	}
	parts = kept
	i := 0
	for i+1 < len(parts) {
		endsMarker := parts[i].Kind == ast.IStrong &&
			strings.HasSuffix(oneRender(parts[i]), "**")
		startsMarker := parts[i+1].Kind == ast.IStrong &&
			strings.HasPrefix(oneRender(parts[i+1]), "**")
		if endsMarker && startsMarker {
			b := parts[i+1]
			a := parts[i]
			merged := append(append([]ast.Inline{}, a.Inner...), b.Inner...)
			parts = append(parts[:i+1], parts[i+2:]...)
			parts[i] = ast.Inline{Kind: ast.IStrong, Inner: merged}
			continue // re-check the merged run against its successor
		}
		i++
	}
	return parts
}

func oneRender(in ast.Inline) string {
	return render.InlinesToString([]ast.Inline{in})
}
