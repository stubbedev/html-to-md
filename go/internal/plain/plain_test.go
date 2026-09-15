package plain

import "testing"

func TestJoinsSoftWrappedParagraphs(t *testing.T) {
	input := "The quick brown fox \njumped over \nthe lazy dog.\nSecond paragraph \nhere.\n"
	want := "The quick brown fox jumped over the lazy dog.\n\nSecond paragraph here."
	if got := RenderFlowed(input); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestRendersQuoteLevelsAsBlockquotes(t *testing.T) {
	input := "> quoted line one \n> quoted line two.\nreply body\n>> deeper \n>> note.\n"
	want := "> quoted line one quoted line two.\n\nreply body\n\n>> deeper note."
	if got := RenderFlowed(input); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestHardLinesEndParagraphs(t *testing.T) {
	input := "line one\nline two  \nline three\n"
	want := "line one\n\nline two\n\nline three"
	if got := RenderFlowed(input); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestBlankLinesSeparateParagraphs(t *testing.T) {
	if got, want := RenderFlowed("one \ntwo\n\nthree\n"), "one two\n\nthree"; got != want {
		t.Errorf("got %q want %q", got, want)
	}
}

func TestSignatureStaysVerbatimAndTight(t *testing.T) {
	input := "hello \nworld.\n-- \nAlex\nsent from my terminal \n"
	want := "hello world.\n\n-- \nAlex\nsent from my terminal"
	if got := RenderFlowed(input); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestStripsWhitespaceStuffingKeepsIndents(t *testing.T) {
	input := " leading space kept minus stuffing \n  intentional indent\nFrom protected \n"
	want := "leading space kept minus stuffing intentional indent\n\nFrom protected"
	if got := RenderFlowed(input); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestDepthChangeBreaksParagraph(t *testing.T) {
	input := "para one \n> quote jumps in \n> and stays.\n"
	want := "para one\n\n> quote jumps in and stays."
	if got := RenderFlowed(input); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}

func TestPassthroughWhenNotFlowed(t *testing.T) {
	input := "a \nb\n"
	if got := Render(input, false); got != input {
		t.Errorf("got %q want %q", got, input)
	}
	if got := Render(input, true); got != "a b" {
		t.Errorf("got %q want %q", got, "a b")
	}
}

func TestConditionalCommentNoiseDropped(t *testing.T) {
	input := "<!--[if !mso]><!-->\nreal text \n<!--[if false]><!-->\nmore.\n<!--<![endif]-->\n"
	want := "real text more."
	if got := RenderFlowed(input); got != want {
		t.Errorf("\ngot:\n%s\nwant:\n%s", got, want)
	}
}
