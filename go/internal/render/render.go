// Package render: AST → Markdown. One deterministic pass: escape source
// text once, emit structural markers. No regexes, no re-parsing of emitted
// output.
package render

import (
	"strings"

	"github.com/stubbe/html-to-md/internal/ast"
)

// Document renders a whole document: blocks joined by blank lines, blank
// runs collapsed.
func Document(blocks []ast.Block) string {
	parts := make([]string, 0, len(blocks))
	for _, b := range blocks {
		if s := block(b); s != "" {
			parts = append(parts, s)
		}
	}
	return collapseBlankRuns(strings.Join(parts, "\n\n"))
}

func collapseBlankRuns(s string) string {
	for strings.Contains(s, "\n\n\n") {
		s = strings.ReplaceAll(s, "\n\n\n", "\n\n")
	}
	return s
}

// NormalizeWs collapses whitespace between tokens to single spaces (browser
// whitespace collapsing, applied in whitespace-hostile contexts).
func NormalizeWs(s string) string {
	words := strings.Fields(s)
	return strings.Join(words, " ")
}

// InlinesToString renders inlines to their Markdown string.
func InlinesToString(inlines []ast.Inline) string {
	var b strings.Builder
	for _, in := range inlines {
		b.WriteString(inline(in))
	}
	return b.String()
}

// TidyParagraph trims an accumulated paragraph: strip surrounding whitespace
// on every line while preserving blank lines from <br><br>. A paragraph that
// reduces to a single ASCII punctuation char is dropped: it is separator
// residue, e.g. the " . " left between two <img> removed as decorative media.
func TidyParagraph(s string) string {
	var out string
	if !strings.Contains(s, "\n") {
		out = strings.TrimSpace(s)
	} else {
		var lines []string
		for l := range strings.SplitSeq(s, "\n") {
			lines = append(lines, strings.TrimSpace(l))
		}
		out = strings.TrimSpace(strings.Join(lines, "\n"))
	}
	if len(out) == 1 && isAsciiPunct(out[0]) {
		return ""
	}
	return out
}

func isAsciiPunct(c byte) bool {
	asciiPunct := "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"
	return strings.IndexByte(asciiPunct, c) >= 0
}

func block(b ast.Block) string {
	switch b.Kind {
	case ast.KindParagraph:
		return TidyParagraph(InlinesToString(b.Inlines))
	case ast.KindHeading:
		// A heading is a single line; fold any <br>-newline to a space.
		s := NormalizeWs(InlinesToString(b.Inlines))
		if s == "" {
			return ""
		}
		return strings.Repeat("#", b.Level) + " " + s
	case ast.KindRule:
		return "---"
	case ast.KindCode:
		return "```\n" + b.Body + "\n```"
	case ast.KindQuote:
		var inner []string
		for _, q := range b.Blocks {
			if s := block(q); s != "" {
				inner = append(inner, s)
			}
		}
		if len(inner) == 0 {
			return ""
		}
		var lines []string
		for l := range strings.SplitSeq(strings.Join(inner, "\n\n"), "\n") {
			if l == "" {
				lines = append(lines, ">")
			} else {
				lines = append(lines, "> "+l)
			}
		}
		return strings.Join(lines, "\n")
	case ast.KindList:
		return list(b.Items, b.Ordered, 0)
	case ast.KindTable:
		return table(b.Rows)
	}
	return ""
}

func list(items []ast.ListItem, ordered bool, depth int) string {
	indent := strings.Repeat("  ", depth)
	var lines []string
	for _, it := range items {
		text := NormalizeWs(InlinesToString(it.Inlines))
		if text == "" && len(it.SubLists) == 0 {
			continue
		}
		marker := "- "
		if ordered {
			marker = itoa(it.Number) + ". "
		}
		lines = append(lines, indent+marker+text)
		for _, sub := range it.SubLists {
			if sub.Kind == ast.KindList {
				for l := range strings.SplitSeq(list(sub.Items, sub.Ordered, depth+1), "\n") {
					lines = append(lines, l)
				}
			}
		}
	}
	return strings.Join(lines, "\n")
}

func itoa(n int) string {
	if n == 0 {
		return "0"
	}
	neg := n < 0
	if neg {
		n = -n
	}
	var b [12]byte
	i := len(b)
	for n > 0 {
		i--
		b[i] = byte('0' + n%10)
		n /= 10
	}
	if neg {
		return "-" + string(b[i:])
	}
	return string(b[i:])
}

func table(rows ast.Table) string {
	ncols := 0
	for _, r := range rows {
		ncols = max(ncols, len(r))
	}
	if ncols == 0 {
		return ""
	}
	// Minimal framing: single space between the pipes and the cells, no
	// column padding (the terminal concealers don't align raw columns).
	var lines []string
	for i, row := range rows {
		var line strings.Builder
		line.WriteByte('|')
		for c := range ncols {
			cell := ""
			if c < len(row) {
				cell = NormalizeWs(InlinesToString(row[c]))
			}
			line.WriteByte(' ')
			line.WriteString(cell)
			line.WriteString(" |")
		}
		lines = append(lines, line.String())
		if i == 0 {
			var sep strings.Builder
			sep.WriteByte('|')
			for range ncols {
				sep.WriteString(" - |")
			}
			lines = append(lines, sep.String())
		}
	}
	return strings.Join(lines, "\n")
}

func inline(in ast.Inline) string {
	switch in.Kind {
	case ast.IText:
		return escapeMarkdown(in.S)
	case ast.IRaw:
		return in.S
	case ast.ICode:
		return "`" + in.S + "`"
	case ast.IStrong:
		return emphasis(in.Inner, "**")
	case ast.IEmph:
		return emphasis(in.Inner, "*")
	case ast.ILink:
		return "[" + in.LinkText + "](" + in.Href + ")"
	case ast.ILineBreak:
		return "\n"
	}
	return ""
}

// Emphasis wraps inline content in a marker, keeping any leading or trailing
// whitespace *outside* the markers. HTML often splits a phrase across
// adjacent runs where the only separating space lives at a marker boundary.
func emphasis(inner []ast.Inline, marker string) string {
	s := InlinesToString(inner)
	if strings.Contains(s, "\n") {
		// A <br> inside emphasis: wrap each line on its own so markers never
		// span a line break.
		var out []string
		for line := range strings.SplitSeq(s, "\n") {
			t := strings.TrimSpace(line)
			if t == "" {
				out = append(out, "")
			} else {
				out = append(out, marker+t+marker)
			}
		}
		return strings.Join(out, "\n")
	}
	t := strings.TrimSpace(s)
	if t == "" {
		return ""
	}
	lead, trail := "", ""
	if startsWithWs(s) {
		lead = " "
	}
	if endsWithWs(s) {
		trail = " "
	}
	return lead + marker + t + marker + trail
}

func startsWithWs(s string) bool {
	for _, c := range s {
		return isWs(c)
	}
	return false
}

func endsWithWs(s string) bool {
	r := []rune(s)
	if len(r) == 0 {
		return false
	}
	return isWs(r[len(r)-1])
}

func isWs(c rune) bool {
	switch c {
	case ' ', '\t', '\n', '\r', '\v', '\f', 0x85, 0xA0:
		return true
	}
	return false
}

// escapeMarkdown backslash-escapes the structural characters. Whitespace
// collapsing and unicode-escape decoding happened at lowering.
func escapeMarkdown(s string) string {
	var b strings.Builder
	b.Grow(len(s) + 4)
	for _, c := range s {
		switch c {
		case '\\', '`', '*', '_', '[', ']':
			b.WriteByte('\\')
			b.WriteRune(c)
		default:
			b.WriteRune(c)
		}
	}
	return b.String()
}

// Esc escapes the same structural characters (used by the calendar path).
func Esc(s string) string { return escapeMarkdown(s) }
