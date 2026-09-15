package calendar

import (
	"strconv"
	"strings"
)

// prettyRrule renders an RRULE in prose; unknown FREQs fall back to the raw
// value.
func prettyRrule(v string) string {
	freq := ""
	interval := 1
	count := -1
	until := ""
	var byday []string
	for part := range strings.SplitSeq(v, ";") {
		k, val, found := strings.Cut(part, "=")
		if !found {
			continue
		}
		switch strings.ToUpper(k) {
		case "FREQ":
			switch strings.ToUpper(val) {
			case "DAILY":
				freq = "day"
			case "WEEKLY":
				freq = "week"
			case "MONTHLY":
				freq = "month"
			case "YEARLY":
				freq = "year"
			default:
				freq = ""
			}
		case "INTERVAL":
			if n, err := strconv.Atoi(val); err == nil && n > 0 {
				interval = n
			}
		case "COUNT":
			if n, err := strconv.Atoi(val); err == nil {
				count = n
			}
		case "UNTIL":
			until = strings.TrimRight(val, "Z")
		case "BYDAY":
			for d := range strings.SplitSeq(val, ",") {
				switch strings.TrimSpace(d) {
				case "MO":
					byday = append(byday, "Mon")
				case "TU":
					byday = append(byday, "Tue")
				case "WE":
					byday = append(byday, "Wed")
				case "TH":
					byday = append(byday, "Thu")
				case "FR":
					byday = append(byday, "Fri")
				case "SA":
					byday = append(byday, "Sat")
				case "SU":
					byday = append(byday, "Sun")
				}
			}
		}
	}
	if freq == "" {
		return v
	}
	out := ""
	if interval == 1 {
		out += freq + "s"
	} else {
		out += "every " + strconv.Itoa(interval) + " " + freq + "s"
	}
	// A single repeat day reads better as the bare weekday name.
	if len(byday) == 1 {
		out += " on " + byday[0]
	} else if len(byday) > 1 {
		out += " on " + strings.Join(byday, ", ")
	}
	if count >= 0 {
		out += ", " + strconv.Itoa(count) + " times"
	} else if until != "" {
		if y, m, d, ok := parseDate(until); ok {
			out += ", until " + weekday(y, m, d) + ", " + strconv.Itoa(d) + " " + months[m-1] + " " + strconv.Itoa(y)
		}
	}
	return out
}
