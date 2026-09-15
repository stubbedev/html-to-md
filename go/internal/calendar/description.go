package calendar

import (
	conv "github.com/stubbe/html-to-md/internal/convert"
	"strings"
)

// renderDescription renders a DESCRIPTION value as tight lines.
//
// Booking systems (and some CRM tools) embed literal HTML in the
// DESCRIPTION text value; those route through the HTML pipeline so the
// output stays in dialect. Plain descriptions keep the tight-lines path.
func renderDescription(raw string) []string {
	if looksLikeHTML(raw) {
		if converted := conv.Convert(raw); strings.TrimSpace(converted) != "" {
			return []string{converted}
		}
	}
	var lines []string
	for line := range strings.SplitSeq(raw, "\n") {
		t := strings.TrimSpace(line)
		if isGoogleFence(t) || strings.EqualFold(t, "Please do not edit this section.") {
			continue
		}
		if t == "" {
			if len(lines) > 0 && lines[len(lines)-1] != "" {
				lines = append(lines, "")
			}
			continue
		}
		lines = append(lines, esc(normalizeWs(t)))
	}
	// Collapse to single tight paragraphs: consecutive lines stay consecutive.
	var out []string
	var para []string
	for _, l := range lines {
		if l == "" {
			if len(para) > 0 {
				out = append(out, strings.Join(para, "\n"))
				para = nil
			}
		} else {
			para = append(para, l)
		}
	}
	if len(para) > 0 {
		out = append(out, strings.Join(para, "\n"))
	}
	return out
}

// looksLikeHTML checks for real markup in the description (not just a stray
// `<` in prose): require a closing tag or a known block/inline tag.
func looksLikeHTML(s string) bool {
	lower := strings.ToLower(s)
	for _, tag := range []string{"</p>", "<br", "<div", "<p>", "<strong", "<em>", "<ul", "<table>"} {
		if strings.Contains(lower, tag) {
			return true
		}
	}
	return false
}

// isGoogleFence detects the `-::~:~:…::-` lines Google Meet invites fence
// their description with.
func isGoogleFence(t string) bool {
	if len(t) < 3 {
		return false
	}
	hasTilde := false
	for _, c := range t {
		switch c {
		case '-', ':', '~', ' ':
			if c == '~' {
				hasTilde = true
			}
		default:
			return false
		}
	}
	return hasTilde
}
