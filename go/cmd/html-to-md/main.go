// Command html-to-md is the aerc filter binary: read a mail part from
// stdin, write Markdown to stdout. The conversion mode comes from a flag;
// with no flag the input is sniffed (see convert.Detect).
package main

import (
	"fmt"
	"io"
	"os"
	"strings"

	"github.com/stubbe/html-to-md/internal/calendar"
	"github.com/stubbe/html-to-md/internal/convert"
	"github.com/stubbe/html-to-md/internal/plain"
)

const usage = `html-to-md — render email parts as Markdown

usage: html-to-md [--html | --plain | --calendar]

  --html      force the HTML pipeline (vendor-noisy HTML → Markdown)
  --plain     dewrap RFC 3676 format=flowed (AERC_FORMAT=flowed); passthrough otherwise
  --calendar  render an iCalendar part (invites, todos)
  (no flag)   auto-detect: BEGIN:VCALENDAR → calendar, HTML markers → html, else plain
`

func main() {
	var mode convert.Format
	haveMode := false
	for _, arg := range os.Args[1:] {
		switch arg {
		case "--html":
			mode, haveMode = convert.Html, true
		case "--plain":
			mode, haveMode = convert.Plain, true
		case "--calendar":
			mode, haveMode = convert.Calendar, true
		case "-h", "--help":
			fmt.Print(usage)
			return
		default:
			fmt.Fprintf(os.Stderr, "html-to-md: unknown argument `%s`\n\n%s", arg, usage)
			os.Exit(2)
		}
	}

	// Filters are handed UTF-8 by aerc, but decode lossily anyway so a
	// stray non-UTF-8 byte degrades to U+FFFD instead of killing the render.
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		fmt.Fprintf(os.Stderr, "html-to-md: reading stdin: %v\n", err)
		os.Exit(1)
	}
	input := string(raw)

	out := ""
	switch {
	case !haveMode:
		mode = convert.Detect(input)
	}
	switch mode {
	case convert.Html:
		out = convert.Convert(input)
	case convert.Plain:
		// aerc passes the part's content-type format= parameter here.
		flowed := strings.EqualFold(os.Getenv("AERC_FORMAT"), "flowed")
		out = plain.Render(input, flowed)
	case convert.Calendar:
		out = calendar.Convert(input)
	}
	if _, err := io.WriteString(os.Stdout, out); err != nil {
		fmt.Fprintf(os.Stderr, "html-to-md: writing stdout: %v\n", err)
		os.Exit(1)
	}
}
