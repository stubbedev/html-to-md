package text

import "testing"

func TestStripsKnownTrackingParams(t *testing.T) {
	cases := [][2]string{
		{"https://x.com/signup?utm_source=news&utm_medium=email&id=7", "https://x.com/signup?id=7"},
		{"https://x.com/a?fbclid=AbCd", "https://x.com/a"},
		{"https://x.com/p?UTM_Source=a&keep=1#section", "https://x.com/p?keep=1#section"},
		{"mailto:me@example.com?utm_source=x", "mailto:me@example.com?utm_source=x"},
		{"https://x.com", "https://x.com"},
		{"https://x.com/a?gclid=1&=2&&ok=3", "https://x.com/a?=2&ok=3"},
	}
	for _, c := range cases {
		if got := StripTrackingParams(c[0]); got != c[1] {
			t.Errorf("StripTrackingParams(%q) = %q, want %q", c[0], got, c[1])
		}
	}
}

func TestDecodesLiteralUnicodeEscapes(t *testing.T) {
	cases := [][2]string{
		{`June 13 – June 20`, `June 13 – June 20`},
		{`\U0001f9e0 Stop`, "🧠 Stop"},
		{`x\u{1F9E0}y`, "x🧠y"},
		{`🧠`, `🧠`},
		{`the \understood plan`, `the \understood plan`},
		{`\uZZZZ`, `\uZZZZ`},
		{"no escapes here", "no escapes here"},
		{`\uD83E!`, `\uD83E!`},
	}
	for _, c := range cases {
		if got := DecodeUnicodeEscapes(c[0]); got != c[1] {
			t.Errorf("DecodeUnicodeEscapes(%q) = %q, want %q", c[0], got, c[1])
		}
	}
}

func TestCleanInvisibles(t *testing.T) {
	if got := CleanInvisibles("A\u200bB\u00a0C\u00adD"); got != "AB CD" {
		t.Errorf("CleanInvisibles = %q", got)
	}
}

func TestWidth(t *testing.T) {
	// CJK counts double; emoji are wide.
	if Width("a") != 1 || Width("漢") != 2 || Width("🧠") != 2 {
		t.Errorf("Width = %d %d %d", Width("a"), Width("漢"), Width("🧠"))
	}
}
