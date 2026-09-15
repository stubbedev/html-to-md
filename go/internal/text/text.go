// Package text: text-node normalisation, unicode escape decoding, tracking
// parameter stripping, and display-width aware character counting.
package text

import (
	"strings"
	"unicode"

	"github.com/mattn/go-runewidth"
)

// CleanInvisibles drops zero-width / format characters (preview-text padding
// tricks used by vendors) and normalises NBSP-class horizontal whitespace to
// a regular space.
func CleanInvisibles(s string) string {
	var b strings.Builder
	b.Grow(len(s))
	for _, c := range s {
		switch {
		case zeroWidth(c):
			// dropped
		case nbspClass(c):
			b.WriteByte(' ')
		default:
			b.WriteRune(c)
		}
	}
	return b.String()
}

func zeroWidth(c rune) bool {
	switch c {
	case '\u00AD', '\u034F', '\u061C', '\u115F', '\u1160', '\u17B4', '\u17B5',
		'\u180E', '\u200B', '\u200C', '\u200D', '\u200E', '\u200F', '\u2060',
		'\u3164', '\uFEFF', '\uFFA0':
		return true
	}
	return (c >= '\u202A' && c <= '\u202E') ||
		(c >= '\u2061' && c <= '\u2064') ||
		(c >= '\u2066' && c <= '\u2069') ||
		(c >= '\uFE00' && c <= '\uFE0F') ||
		(c >= '\U000E0020' && c <= '\U000E007F')
}

func nbspClass(c rune) bool {
	switch c {
	case '\u00A0', '\u202F', '\u205F', '\u3000':
		return true
	}
	return c >= '\u2000' && c <= '\u200A'
}

// IsDecorativeGlyph reports whether s is a single non-alphanumeric rune
// (a "›"-style icon link).
func IsDecorativeGlyph(s string) bool {
	r := []rune(s)
	if len(r) != 1 {
		return false
	}
	return !unicode.IsLetter(r[0]) && !unicode.IsDigit(r[0])
}

// Width returns the terminal display width of s (CJK and combining marks
// aware), used wherever the pipeline bounds text by visual size.
func Width(s string) int { return runewidth.StringWidth(s) }

// trackingParams carries click-attribution-only query parameters. Stripping
// them from http(s) hrefs never changes the destination; hrefs are otherwise
// never rewritten (see the output contract in README.md). Non-http(s)
// schemes (mailto:, tel:, …) pass through untouched.
var trackingParams = map[string]bool{
	"utm_source": true, "utm_medium": true, "utm_campaign": true,
	"utm_term": true, "utm_content": true, "utm_id": true,
	"gclid": true, "gclsrc": true, "dclid": true, "fbclid": true,
	"msclkid": true, "twclid": true, "yclid": true, "igshid": true,
	"_hsenc": true, "_hsmi": true, "vero_id": true, "vero_conv": true,
	"mc_cid": true, "mc_eid": true, "s_kwcid": true, "elqtrackid": true,
}

func StripTrackingParams(href string) string {
	if !strings.HasPrefix(href, "http://") && !strings.HasPrefix(href, "https://") {
		return href
	}
	base, rest, found := strings.Cut(href, "?")
	if !found {
		return href
	}
	query, frag, hasFrag := strings.Cut(rest, "#")
	kept := make([]string, 0, 8)
	for kv := range strings.SplitSeq(query, "&") {
		if kv == "" {
			continue
		}
		key, _, _ := strings.Cut(kv, "=")
		if trackingParams[strings.ToLower(key)] {
			continue
		}
		kept = append(kept, kv)
	}
	var out string
	if len(kept) == 0 {
		out = base
	} else {
		out = base + "?" + strings.Join(kept, "&")
	}
	if hasFrag {
		out += "#" + frag
	}
	return out
}

// DecodeUnicodeEscapes decodes literal unicode escape sequences that broken
// sender templates emit as visible text: \uXXXX (with UTF-16 surrogate
// pairing), \u{XXXX} and \UXXXXXXXX. Anything not well-formed is left
// verbatim.
func DecodeUnicodeEscapes(s string) string {
	if !strings.Contains(s, "\\u") && !strings.Contains(s, "\\U") {
		return s
	}
	c := []rune(s)
	var b strings.Builder
	b.Grow(len(s))
	isHex := func(sl []rune) (rune, bool) {
		if len(sl) == 0 {
			return 0, false
		}
		v := 0
		for _, h := range sl {
			var d rune
			switch {
			case h >= '0' && h <= '9':
				d = h - '0'
			case h >= 'a' && h <= 'f':
				d = h - 'a' + 10
			case h >= 'A' && h <= 'F':
				d = h - 'A' + 10
			default:
				return 0, false
			}
			v = v<<4 | int(d)
			if v > 0xFFFFFFF {
				return 0, false
			}
		}
		return rune(v), true
	}
	valid := func(cp rune) bool {
		return (cp < 0xD800 || cp > 0xDFFF) && cp <= 0x10FFFF
	}

	i := 0
	for i < len(c) {
		if c[i] == '\\' && i+1 < len(c) && (c[i+1] == 'u' || c[i+1] == 'U') {
			// \u{...} brace form
			if c[i+1] == 'u' && i+2 < len(c) && c[i+2] == '{' {
				if end := runeIndexRune(c[i+3:], '}'); end >= 0 {
					if cp, ok := isHex(c[i+3 : i+3+end]); ok && valid(cp) {
						b.WriteRune(cp)
						i += 3 + end + 1
						continue
					}
				}
			}
			width := 4
			if c[i+1] == 'U' {
				width = 8
			}
			if i+2+width <= len(c) {
				if cp, ok := isHex(c[i+2 : i+2+width]); ok {
					if width == 4 && cp >= 0xD800 && cp <= 0xDBFF {
						// high surrogate followed by a low surrogate: pair
						j := i + 6
						if j+6 <= len(c) && c[j] == '\\' && c[j+1] == 'u' {
							if lo, ok := isHex(c[j+2 : j+6]); ok && lo >= 0xDC00 && lo <= 0xDFFF {
								combined := 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00)
								if combined <= 0x10FFFF {
									b.WriteRune(rune(combined))
									i = j + 6
									continue
								}
							}
						}
					} else if valid(cp) {
						b.WriteRune(cp)
						i += 2 + width
						continue
					}
				}
			}
		}
		b.WriteRune(c[i])
		i++
	}
	return b.String()
}

func runeIndexRune(rs []rune, want rune) int {
	for i, r := range rs {
		if r == want {
			return i
		}
	}
	return -1
}
