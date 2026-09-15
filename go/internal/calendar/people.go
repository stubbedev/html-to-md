package calendar

import (
	"strings"
)

func renderPeople(c *comp) []string {
	if org := c.prop("ORGANIZER"); org != nil {
		cn, email := person(org)
		s := "**Organiser:** "
		if cn != "" {
			s += esc(cn)
			if email != "" {
				s += " <" + email + ">"
			}
		} else if email != "" {
			s += email
		}
		out := []string{s}
		return append(out, renderAttendees(c, email)...)
	}
	return renderAttendees(c, "")
}

func renderAttendees(c *comp, skipEmail string) []string {
	type person3 struct{ cn, email, mark string }
	var people []person3
	for _, p := range c.propsNamed("ATTENDEE") {
		cn, email := person(p)
		mark := ""
		if partstat, ok := p.param("PARTSTAT"); ok {
			switch strings.ToUpper(partstat) {
			case "ACCEPTED":
				mark = " ✓"
			case "DECLINED":
				mark = " ✗"
			case "TENTATIVE":
				mark = " ~"
			}
		}
		if email == "" || email == skipEmail {
			continue
		}
		people = append(people, person3{cn, email, mark})
	}
	if len(people) == 0 {
		return nil
	}
	label := "**Attendees:**"
	if len(people) == 1 {
		label = "**Attendee:**"
	}
	if len(people) <= 3 {
		var entries []string
		for _, p := range people {
			entries = append(entries, personEntry(p.cn, p.email, p.mark))
		}
		return []string{label + " " + joinComma(entries)}
	}
	out := []string{label}
	for _, p := range people {
		out = append(out, "- "+personEntry(p.cn, p.email, p.mark))
	}
	return out
}

func joinComma(parts []string) string {
	var b strings.Builder
	for i, p := range parts {
		if i > 0 {
			b.WriteString(", ")
		}
		b.WriteString(p)
	}
	return b.String()
}

func personEntry(cn, email, mark string) string {
	s := ""
	// Google sets CN to the bare email when no name is known; rendering
	// `email <email>` is duplicate noise.
	if cn != "" && !strings.EqualFold(cn, email) {
		s += esc(cn)
	}
	if email != "" {
		if s != "" {
			s += " "
		}
		s += "<" + email + ">"
	}
	if s == "" {
		s = "(unnamed)"
	}
	return s + mark
}

// person returns (common name, email address).
func person(p *prop) (string, string) {
	cn := ""
	if v, ok := p.param("CN"); ok {
		cn = strings.TrimSpace(unescapeText(v))
	}
	return cn, mailAddress(p.value)
}

// mailAddress strips a "mailto:" scheme; bare values pass through.
func mailAddress(v string) string {
	v = strings.TrimSpace(v)
	return strings.TrimPrefix(v, "mailto:")
}
