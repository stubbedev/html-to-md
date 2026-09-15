package calendar

import (
	"strings"
)

// unescapeText decodes RFC 5545 text escapes: \n/\N → newline; \, \; \\
// unescaped; undefined escapes (\(, …) are treated as the bare character —
// broken producer templates are common and the backslash is never meaningful.
func unescapeText(v string) string {
	var b strings.Builder
	b.Grow(len(v))
	rs := []rune(v)
	for i := 0; i < len(rs); i++ {
		if rs[i] == '\\' {
			if i+1 < len(rs) {
				switch rs[i+1] {
				case 'n', 'N':
					b.WriteRune('\n')
				case ',':
					b.WriteRune(',')
				case ';':
					b.WriteRune(';')
				case '\\':
					b.WriteRune('\\')
				default:
					b.WriteRune(rs[i+1])
				}
				i++
			}
			continue
		}
		b.WriteRune(rs[i])
	}
	return b.String()
}

func normalizeWs(s string) string {
	return strings.Join(strings.Fields(s), " ")
}

// esc backslash-escapes the structural Markdown characters.
func esc(s string) string {
	var b strings.Builder
	b.Grow(len(s))
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
