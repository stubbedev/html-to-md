// Package plain: RFC 3676 format=flowed dewrapping for text/plain parts.
//
// Flowed mail soft-wraps paragraphs at ~78 columns: a line ending in a
// single space continues on the next line, a line without one is a hard
// break. Quoting is > markers (each optionally followed by one space), a
// "-- " line starts the signature. This module reflows soft-wrapped
// paragraphs into one line per paragraph — the same dialect the HTML path
// emits — and renders quote levels as Markdown blockquotes.
//
// Whitespace-stuffing (the single leading space senders add to protect
// "From " lines) is deleted; deeper indents survive.
package plain

import (
	"strings"
)

// Render renders a text/plain part: passthrough unless flowed.
func Render(input string, flowed bool) string {
	if flowed {
		return RenderFlowed(input)
	}
	return input
}

// RenderFlowed reflows a format=flowed body.
func RenderFlowed(input string) string {
	var out []string
	var cur strings.Builder
	curDepth := -1 // Option<usize>::None
	inSig := false
	flush := func() {
		if strings.TrimSpace(cur.String()) != "" {
			depth := max(curDepth, 0)
			line := strings.TrimRight(cur.String(), " ")
			if depth == 0 {
				out = append(out, line)
			} else {
				out = append(out, strings.Repeat(">", depth)+" "+line)
			}
			out = append(out, "")
		}
		cur.Reset()
		curDepth = -1
	}

	for raw := range strings.SplitSeq(input, "\n") {
		raw = strings.TrimSuffix(raw, "\r")
		if isConditionalNoise(raw) {
			continue
		}
		if !inSig && strings.TrimRight(raw, " ") == "--" {
			flush()
			inSig = true
			out = append(out, "-- ")
			continue
		}
		if inSig {
			out = append(out, raw)
			continue
		}

		depth, content := splitQuoteDepth(raw)
		if depth != curDepth && (curDepth >= 0 || depth != 0) {
			// Different quote depth (or first line) ends the paragraph.
			flush()
			curDepth = depth
		}
		if raw == "" {
			// A blank line ends the paragraph and is the separator.
			flush()
			curDepth = 0
			continue
		}
		trailing := trailingSpaces(content)
		if trailing == 1 {
			// The trailing space is the join separator; a wrapped
			// continuation's own leading spaces must not pile onto it.
			if strings.HasSuffix(cur.String(), " ") {
				content = strings.TrimLeft(content, " ")
			}
			cur.WriteString(content)
		} else {
			cur.WriteString(strings.TrimRight(content, " "))
			flush()
			curDepth = depth
		}
	}
	flush()

	joined := strings.Join(out, "\n")
	for strings.Contains(joined, "\n\n\n") {
		joined = strings.ReplaceAll(joined, "\n\n\n", "\n\n")
	}
	return strings.TrimSpace(joined)
}

func trailingSpaces(s string) int {
	n := 0
	for i := len(s) - 1; i >= 0 && s[i] == ' '; i-- {
		n++
	}
	return n
}

// isConditionalNoise drops Outlook conditional-comment residue vendors leak
// into the plain part; never meaningful content.
func isConditionalNoise(raw string) bool {
	t := strings.TrimSpace(raw)
	return strings.HasPrefix(t, "<!--[if") ||
		strings.HasPrefix(t, "<![endif") ||
		strings.HasPrefix(t, "<!--<![endif") ||
		strings.HasPrefix(t, "<!--[endif") ||
		t == "<!-->"
}

// splitQuoteDepth counts quote markers: each > consumes one level; an
// optional single space after a > only counts as the inter-marker separator
// when another > follows. One optional space after the final marker is the
// content separator; deeper indentation survives. A sender-prepended single
// space protecting "From " lines is deleted; multi-space indents survive.
func splitQuoteDepth(line string) (int, string) {
	depth := 0
	rest := line
	for strings.HasPrefix(rest, ">") {
		rest = rest[1:]
		if strings.HasPrefix(rest, " ") && strings.HasPrefix(rest[1:], ">") {
			rest = rest[1:]
		}
		depth++
	}
	rest = strings.TrimPrefix(rest, " ")
	if strings.HasPrefix(rest, " ") && !strings.HasPrefix(rest[1:], " ") && rest[1:] != "" {
		rest = rest[1:]
	}
	return depth, rest
}
