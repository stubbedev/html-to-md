package calendar

import (
	"strconv"
	"strings"
)

type tzKind uint8

const (
	tzUTC tzKind = iota
	tzFloating
)

type dateTime struct {
	y      int
	m      int
	d      int
	h      int
	min    int
	hasH   bool
	hasMin bool
	tz     tzKind
	tzid   string
	allDay bool
}

func parseDatetime(p *prop) *dateTime {
	v := strings.TrimSpace(p.value)
	tz := tzFloating
	tzid := ""
	if strings.HasSuffix(v, "Z") || strings.HasSuffix(v, "z") {
		tz = tzUTC
	} else if tzv, ok := p.param("TZID"); ok {
		tzid = tzv
	}
	core := strings.TrimRight(v, "Zz")
	if len(core) >= 15 && core[8] == 'T' {
		y, m, d, ok := parseDate(core[:8])
		if !ok {
			return nil
		}
		h, err1 := strconv.Atoi(core[9:11])
		mn, err2 := strconv.Atoi(core[11:13])
		if err1 != nil || err2 != nil {
			return nil
		}
		return &dateTime{y: y, m: m, d: d, h: h, min: mn, hasH: true, hasMin: true, tz: tz, tzid: tzid}
	}
	if len(core) == 8 {
		y, m, d, ok := parseDate(core)
		if !ok {
			return nil
		}
		return &dateTime{y: y, m: m, d: d, tz: tz, tzid: tzid, allDay: true}
	}
	return nil
}

func parseDate(s string) (int, int, int, bool) {
	if len(s) != 8 {
		return 0, 0, 0, false
	}
	for i := range 8 {
		if s[i] < '0' || s[i] > '9' {
			return 0, 0, 0, false
		}
	}
	y, _ := strconv.Atoi(s[0:4])
	m, _ := strconv.Atoi(s[4:6])
	d, _ := strconv.Atoi(s[6:8])
	if m < 1 || m > 12 || d < 1 || d > 31 {
		return 0, 0, 0, false
	}
	return y, m, d, true
}

// Zeller's congruence; 0 → Saturday.
var weekdays = [...]string{"Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"}

var months = [...]string{"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"}

func weekday(y, m, d int) string {
	yy, mm := y, int64(m)
	if m < 3 {
		yy--
		mm += 12
	}
	k := ((int64(yy) % 100) + 100) % 100
	j := divFloor(int64(yy), 100)
	h := modFloor(int64(d)+(13*(mm+1))/5+int64(k)+k/4+j/4+5*j, 7)
	return weekdays[h]
}

func modFloor(a, b int64) int64 {
	r := a % b
	if r < 0 {
		r += b
	}
	return r
}

func divFloor(a, b int64) int64 {
	q := a / b
	if a%b != 0 && (a < 0) != (b < 0) {
		q--
	}
	return q
}

func formatDate(dt *dateTime) string {
	return weekday(dt.y, dt.m, dt.d) + ", " + strconv.Itoa(dt.d) + " " + months[dt.m-1] + " " + strconv.Itoa(dt.y)
}

func formatTime(dt *dateTime) string {
	return pad2(dt.h) + ":" + pad2(dt.min)
}

func pad2(n int) string {
	return string([]byte{byte('0' + (n/10)%10), byte('0' + n%10)})
}

func formatTz(dt *dateTime) string {
	if dt.tzid != "" {
		return " (" + dt.tzid + ")"
	}
	if dt.tz == tzUTC {
		return " (UTC)"
	}
	return ""
}

func formatDatetime(dt *dateTime) string {
	if dt.allDay {
		return formatDate(dt) + " (all day)"
	}
	return formatDate(dt) + formatTz(dt) + ", " + formatTime(dt)
}

func formatEventTime(c *comp) string {
	start := (*dateTime)(nil)
	if p := c.prop("DTSTART"); p != nil {
		start = parseDatetime(p)
	}
	if start == nil {
		return ""
	}
	if p := c.prop("DTEND"); p != nil {
		if end := parseDatetime(p); end != nil {
			return formatRange(start, end)
		}
	}
	if p := c.prop("DURATION"); p != nil {
		if w, dd, h, m, s, ok := parseDuration(p.value); ok {
			return formatDatetime(start) + " (" + formatDurationCompact(w, dd, h, m, s) + ")"
		}
	}
	return formatDatetime(start)
}

func formatRange(start, end *dateTime) string {
	if start.allDay && end.allDay {
		if start.y == end.y && start.m == end.m && start.d == end.d {
			return formatDate(start) + " (all day)"
		}
		return formatDate(start) + " – " + formatDate(end) + " (all day)"
	}
	if start.y == end.y && start.m == end.m && start.d == end.d {
		tz := formatTz(start)
		return formatDate(start) + ", " + formatTime(start) + " – " + formatTime(end) + tz
	}
	return formatDatetime(start) + " – " + formatDatetime(end) + formatTz(start)
}

// parseDuration parses an RFC 5545 DURATION.

func parseDuration(v string) (weeks, days, hours, mins, secs int, ok bool) {
	v = strings.TrimSpace(v)
	if len(v) == 0 || v[0] != 'P' && v[0] != 'p' {
		return 0, 0, 0, 0, 0, false
	}
	inTime := false
	num := ""
	flushNum := func(unit rune) bool {
		if num == "" {
			return unit == 'T' || unit == 't'
		}
		n, err := strconv.Atoi(num)
		if err != nil {
			return false
		}
		switch unit {
		case 'W':
			weeks = n
		case 'D':
			if !inTime {
				days = n
			}
		case 'H':
			hours = n
		case 'M':
			if inTime {
				mins = n
			}
		case 'S':
			secs = n
		default:
			return false
		}
		num = ""
		return true
	}
	for _, c := range v[1:] {
		if c >= '0' && c <= '9' {
			num += string(c)
			continue
		}
		if c == 'T' || c == 't' {
			inTime = true
			continue
		}
		if !flushNum(c) {
			return 0, 0, 0, 0, 0, false
		}
	}
	return weeks, days, hours, mins, secs, true
}

func formatDurationCompact(w, dd, h, m, s int) string {
	var parts []string
	if w > 0 {
		parts = append(parts, unit(w, "week"))
	}
	if dd > 0 {
		parts = append(parts, unit(dd, "day"))
	}
	if h > 0 {
		parts = append(parts, strconv.Itoa(h)+" h")
	}
	if m > 0 {
		parts = append(parts, strconv.Itoa(m)+" m")
	}
	if s > 0 {
		parts = append(parts, strconv.Itoa(s)+" s")
	}
	if len(parts) == 0 {
		return "0 m"
	}
	return strings.Join(parts, " ")
}

func unit(n int, name string) string {
	if n == 1 {
		return "1 " + name
	}
	return strconv.Itoa(n) + " " + name + "s"
}
