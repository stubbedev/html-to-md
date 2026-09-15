// Package calendar: iCalendar (RFC 5545) → Markdown for text/calendar parts.
//
// Hand-rolled parser, no dependencies: line unfolding, a component tree
// (BEGIN/END), and property lines split on the first unquoted colon that is
// not inside a `KEY=…` parameter — the quirk that lets unquoted
// `TZID=Europe/Berlin:…` values survive. Renders events and todos as the
// same Markdown dialect the HTML path emits: one heading, bold-labelled
// fact lines, attendees as a list, description paragraphs.
package calendar

import (
	"strconv"
	"strings"
)

// Convert converts an ICS payload; unparsable input passes through.
func Convert(input string) string {
	roots := parseICS(input)
	if len(roots) == 0 {
		return strings.TrimRight(input, " \t\n")
	}
	return renderRoots(roots)
}

type prop struct {
	name   string
	params [][2]string
	value  string
}

// param looks a parameter up case-insensitively, first-wins like producers
// that repeat keys.
// param looks a parameter up case-insensitively, first-wins like producers
// that repeat keys.
func (p *prop) param[T ~string](name T) (string, bool) {
	for _, kv := range p.params {
		if kv[0] == strings.ToUpper(string(name)) {
			return kv[1], true
		}
	}
	return "", false
}

type comp struct {
	name  string
	props []*prop
	comps []*comp
}

func (c *comp) prop(name string) *prop {
	for _, p := range c.props {
		if p.name == name {
			return p
		}
	}
	return nil
}

func (c *comp) propsNamed(name string) []*prop { // name must be uppercase
	var out []*prop
	for _, p := range c.props {
		if p.name == name {
			out = append(out, p)
		}
	}
	return out
}

// parseICS unfolds continuation lines and builds the component tree.
func parseICS(input string) []*comp {
	var lines []string
	for raw := range strings.SplitSeq(strings.ReplaceAll(input, "\r\n", "\n"), "\n") {
		if strings.HasPrefix(raw, " ") || strings.HasPrefix(raw, "\t") {
			if len(lines) > 0 {
				lines[len(lines)-1] += raw[1:]
				continue
			}
		}
		lines = append(lines, strings.TrimRight(raw, "\r"))
	}

	var roots []*comp
	var stack []*comp
	for _, line := range lines {
		if name, ok := strings.CutPrefix(line, "BEGIN:"); ok {
			stack = append(stack, &comp{name: strings.ToUpper(strings.TrimSpace(name))})
		} else if _, ok := strings.CutPrefix(line, "END:"); ok {
			if len(stack) > 0 {
				done := stack[len(stack)-1]
				stack = stack[:len(stack)-1]
				if len(stack) == 0 {
					roots = append(roots, done)
				} else {
					parent := stack[len(stack)-1]
					parent.comps = append(parent.comps, done)
				}
			}
		} else if p := parseProp(line); p != nil {
			if len(stack) > 0 {
				top := stack[len(stack)-1]
				top.props = append(top.props, p)
			}
		}
	}
	return roots
}

// parseProp splits a property line into name, parameters and value. The
// value starts at the first unquoted colon whose preceding segment (since
// the last `;`) does not contain `=` — that keeps unquoted
// `TZID=Europe/Berlin:…` intact while still splitting `SUMMARY:Hello: world`
// on the first colon.
func parseProp(line string) *prop {
	inQuotes := false
	escaped := false
	segStart := 0
	sep, firstColon := -1, -1
	for i := 0; i < len(line); i++ {
		c := line[i]
		if escaped {
			escaped = false
			continue
		}
		switch c {
		case '\\':
			if inQuotes {
				escaped = true
			}
		case '"':
			inQuotes = !inQuotes
		case ';':
			if !inQuotes {
				segStart = i + 1
			}
		case ':':
			if !inQuotes {
				if firstColon < 0 {
					firstColon = i
				}
				seg := line[segStart:i]
				if !strings.Contains(seg, "=") {
					sep = i
					i = len(line)
				}
			}
		}
	}
	if sep < 0 {
		sep = firstColon
	}
	if sep < 0 {
		return nil
	}

	head := line[:sep]
	value := line[sep+1:]
	segs := splitUnquotedSemicolons(head)
	name := strings.ToUpper(strings.TrimSpace(segs[0]))
	if name == "" {
		return nil
	}
	var params [][2]string
	for _, seg := range segs[1:] {
		k, v, found := strings.Cut(seg, "=")
		if !found {
			v = ""
		}
		if len(v) >= 2 && strings.HasPrefix(v, `"`) && strings.HasSuffix(v, `"`) {
			v = v[1 : len(v)-1]
		}
		params = append(params, [2]string{strings.ToUpper(strings.TrimSpace(k)), v})
	}
	return &prop{name: name, params: params, value: value}
}

// splitUnquotedSemicolons splits on `;` outside quoted parameter values
// (honouring backslash escapes inside quotes).
func splitUnquotedSemicolons(s string) []string {
	var parts []string
	start := 0
	inQuotes := false
	escaped := false
	for i, c := range s {
		if escaped {
			escaped = false
			continue
		}
		switch c {
		case '\\':
			if inQuotes {
				escaped = true
			}
		case '"':
			inQuotes = !inQuotes
		case ';':
			if !inQuotes {
				parts = append(parts, s[start:i])
				start = i + len(string(c))
			}
		}
	}
	return append(parts, s[start:])
}

// ─── Rendering ───────────────────────────────────────────────────────────────

func renderRoots(roots []*comp) string {
	var blocks []string
	for _, rootp := range roots {
		for _, c := range rootp.comps {
			switch c.name {
			case "VEVENT":
				blocks = append(blocks, renderComponent(c, false)...)
			case "VTODO":
				blocks = append(blocks, renderComponent(c, true)...)
			}
		}
	}
	joined := strings.Join(blocks, "\n\n")
	for strings.Contains(joined, "\n\n\n") {
		joined = strings.ReplaceAll(joined, "\n\n\n", "\n\n")
	}
	return joined
}

func renderComponent(c *comp, todo bool) []string {
	var lines []string
	if p := c.prop("SUMMARY"); p != nil {
		if s := strings.TrimSpace(unescapeText(p.value)); s != "" {
			lines = append(lines, "# "+esc(normalizeWs(s)))
		}
	}

	status := ""
	if p := c.prop("STATUS"); p != nil {
		status = strings.ToUpper(strings.TrimSpace(p.value))
	}
	switch status {
	case "CANCELLED":
		lines = append(lines, "**Cancelled**")
	case "TENTATIVE":
		lines = append(lines, "**Tentative**")
	case "COMPLETED":
		lines = append(lines, "**Completed**")
	}

	if todo {
		if p := c.prop("DUE"); p != nil {
			if dt := parseDatetime(p); dt != nil {
				lines = append(lines, "**Due:** "+formatDatetime(dt))
			}
		}
		if p := c.prop("PRIORITY"); p != nil {
			n, err := strconv.Atoi(strings.TrimSpace(p.value))
			if err == nil && (n >= 1 && n <= 4 || n >= 6 && n <= 9) {
				prio := "high"
				if n >= 6 {
					prio = "low"
				}
				lines = append(lines, "**Priority:** "+prio)
			}
		}
	} else if when := formatEventTime(c); when != "" {
		lines = append(lines, when)
	}

	if p := c.prop("LOCATION"); p != nil {
		if s := strings.TrimSpace(unescapeText(p.value)); s != "" {
			lines = append(lines, "**Location:** "+esc(tidyLocation(s)))
		}
	}
	if p := c.prop("X-GOOGLE-CONFERENCE"); p != nil {
		if url := strings.TrimSpace(p.value); url != "" {
			lines = append(lines, "[Join call]("+url+")")
		}
	}
	if p := c.prop("URL"); p != nil {
		if u := strings.TrimSpace(p.value); u != "" {
			lines = append(lines, "[Link]("+u+")")
		}
	}

	lines = append(lines, renderPeople(c)...)

	if p := c.prop("RRULE"); p != nil {
		if s := prettyRrule(strings.TrimSpace(p.value)); s != "" {
			lines = append(lines, "**Repeats:** "+s)
		}
	}
	if p := c.prop("DESCRIPTION"); p != nil {
		lines = append(lines, renderDescription(unescapeText(p.value))...)
	}
	return lines
}

// tidyLocation fixes escape-decoding residue like "Beta Boulders , Sydhavn"
// where a backslash-escaped comma decoded with a trailing space.
func tidyLocation(s string) string {
	return strings.TrimSpace(strings.ReplaceAll(s, " ,", ","))
}
