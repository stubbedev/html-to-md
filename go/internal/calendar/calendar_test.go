package calendar

import (
	"strings"
	"testing"
)

// INVITE keeps its ICS continuation line's leading space (a Go raw string
// would eat it, so CRLF lines are concatenated literal pieces).
const invite = "" +
	"BEGIN:VCALENDAR\r\n" +
	"PRODID:-//Google Inc//Google Calendar 70.9054//EN\r\n" +
	"VERSION:2.0\r\n" +
	"METHOD:REQUEST\r\n" +
	"BEGIN:VEVENT\r\n" +
	"DTSTART;TZID=Europe/Berlin:20260128T173000\r\n" +
	"DTEND;TZID=Europe/Berlin:20260128T190000\r\n" +
	"ORGANIZER;CN=CPH - Bookings:mailto:bookings@example.com\r\n" +
	"ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;CN=Maria \r\n" +
	" Stage;X-NUM-GUESTS=0:mailto:maria@example.com\r\n" +
	"ATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Second Guest:mailto:second@example.com\r\n" +
	"LOCATION:Beta Boulders \\, Sydhavn\r\n" +
	"X-GOOGLE-CONFERENCE:https://meet.google.com/abc-def-ghi\r\n" +
	"DESCRIPTION:-::~:~::~:~::-\\nJoin with Google Meet: https://meet.google.com/ab\r\n" +
	" c-def-ghi\\n\\nBring chalk.\\nPlease do not edit this section.\\n-::~:~::~:~::-\r\n" +
	"STATUS:CONFIRMED\r\n" +
	"SUMMARY:#127550 Beginner Class \\(Booking\\)\r\n" +
	"END:VEVENT\r\n" +
	"END:VCALENDAR\r\n"

func TestRendersRealisticGoogleInvite(t *testing.T) {
	want := "# #127550 Beginner Class (Booking)\n\n" +
		"Wed, 28 Jan 2026, 17:30 – 19:00 (Europe/Berlin)\n\n" +
		"**Location:** Beta Boulders, Sydhavn\n\n" +
		"[Join call](https://meet.google.com/abc-def-ghi)\n\n" +
		"**Organiser:** CPH - Bookings <bookings@example.com>\n\n" +
		"**Attendees:** Maria Stage <maria@example.com> ✓, Second Guest <second@example.com>\n\n" +
		"Join with Google Meet: https://meet.google.com/abc-def-ghi\n\n" +
		"Bring chalk."
	if got := Convert(invite); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestSplitsUnquotedTzidColonCorrectly(t *testing.T) {
	p := parseProp("DTSTART;TZID=Europe/Berlin:20260128T173000")
	if p.name != "DTSTART" || p.value != "20260128T173000" {
		t.Errorf("parsed %+v", p)
	}
	if v, ok := p.param("TZID"); !ok || v != "Europe/Berlin" {
		t.Errorf("TZID param = %q", v)
	}
}

func TestSplitsQuotedParamsAndFirstColonValues(t *testing.T) {
	p := parseProp(`ATTENDEE;CN="Doe; John":mailto:a@b`)
	if v, _ := p.param("CN"); v != "Doe; John" {
		t.Errorf("CN = %q", v)
	}
	if p.value != "mailto:a@b" {
		t.Errorf("value = %q", p.value)
	}
	p2 := parseProp("SUMMARY:a=b:c")
	if p2.name != "SUMMARY" || p2.value != "a=b:c" {
		t.Errorf("parsed %+v", p2)
	}
}

func TestAllDayAndUtcAndDuration(t *testing.T) {
	ics := "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260901\r\n" +
		"SUMMARY:All-day off\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20260902T100000Z\r\n" +
		"DURATION:PT1H30M\r\nSUMMARY:Sync\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
	want := "# All-day off\n\nTue, 1 Sep 2026 (all day)\n\n# Sync\n\n" +
		"Wed, 2 Sep 2026 (UTC), 10:00 (1 h 30 m)"
	if got := Convert(ics); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestCancelledAndRrule(t *testing.T) {
	ics := "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260901T090000Z\r\n" +
		"DTEND:20260901T100000Z\r\nSTATUS:CANCELLED\r\n" +
		"RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10\r\n" +
		"SUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
	want := "# Standup\n\n**Cancelled**\n\n" +
		"Tue, 1 Sep 2026, 09:00 – 10:00 (UTC)\n\n" +
		"**Repeats:** every 2 weeks on Mon, Wed, 10 times"
	if got := Convert(ics); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestNonCalendarPassthrough(t *testing.T) {
	if got := Convert("just some text"); got != "just some text" {
		t.Errorf("got %q", got)
	}
}

func TestHTMLDescriptionRoutedThroughHTMLPipeline(t *testing.T) {
	ics := "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260901T090000Z\r\n" +
		"DTEND:20260901T100000Z\r\n" +
		"DESCRIPTION:<p><strong>Status</strong>: Paid</p><p><strong>Location</strong>: SYDHAVN</p>\r\n" +
		"SUMMARY:Booking\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
	out := Convert(ics)
	if !strings.Contains(out, "**Status**: Paid") || !strings.Contains(out, "**Location**: SYDHAVN") || strings.Contains(out, "<p>") {
		t.Errorf("out:\n%s", out)
	}
}

func TestAttendeeWithEmailAsCnIsNotDuplicated(t *testing.T) {
	ics := "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260901T090000Z\r\n" +
		"ORGANIZER;CN=Org:mailto:org@example.com\r\n" +
		"ATTENDEE;PARTSTAT=NEEDS-ACTION;CN=guest@example.com:mailto:guest@example.com\r\n" +
		"SUMMARY:Meet\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
	out := Convert(ics)
	if !strings.Contains(out, "**Attendee:** <guest@example.com>") ||
		strings.Contains(out, "guest@example.com <guest@example.com>") {
		t.Errorf("out:\n%s", out)
	}
}

func TestUnescapesTextValues(t *testing.T) {
	want := "a,b;c\nd\\e"
	if got, want := unescapeText(`a\,b\;c\nd\\e`), want; got != want {
		t.Errorf("got %q want %q", got, want)
	}
}
