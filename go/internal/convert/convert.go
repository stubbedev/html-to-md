// Package convert ties the pipeline together: mode sniffing and the
// HTML→Markdown conversion entry point.
package convert

import (
	"strings"

	"github.com/stubbe/html-to-md/internal/ast"
	"github.com/stubbe/html-to-md/internal/clean"
	"github.com/stubbe/html-to-md/internal/lower"
	"github.com/stubbe/html-to-md/internal/render"
)

// Format is a conversion mode: selected by the binary's flags, or sniffed
// by Detect.
type Format uint8

const (
	Html Format = iota
	Plain
	Calendar
)

// Detect sniffs a mail part. ICS declares itself (BEGIN:VCALENDAR, after
// any BOM and whitespace); HTML is detected by common tags in the first
// 2 KB; everything else is plain text.
func Detect(input string) Format {
	trimmed := strings.TrimSpace(input)
	trimmed = strings.TrimSpace(strings.TrimPrefix(trimmed, "\ufeff"))
	if strings.HasPrefix(trimmed, "BEGIN:VCALENDAR") {
		return Calendar
	}
	runes := []rune(input)
	if len(runes) > 2048 {
		runes = runes[:2048]
	}
	head := strings.ToLower(string(runes))
	for _, tag := range []string{
		"<!doctype html", "<html", "<body", "<div", "<p>", "<p ", "<br",
		"<table", "<span", "<strong", "<em>",
	} {
		if strings.Contains(head, tag) {
			return Html
		}
	}
	return Plain
}

// Convert renders an HTML email body as Markdown. Paragraphs are emitted
// unwrapped; the pager owns soft-wrapping.
func Convert(html string) string {
	docp := clean.Doc(html)
	shift := max(lower.MinHeadingLevel(docp)-1, 0)
	blocks := lower.Document(docp, shift)
	blocks = astDropEmpty(blocks)
	blocks = astCompress(blocks)
	blocks = astJoinLinks(blocks)
	return strings.TrimLeft(render.Document(blocks), "\n")
}

func astDropEmpty(bs []ast.Block) []ast.Block { return ast.DropEmptySections(bs) }
func astCompress(bs []ast.Block) []ast.Block  { return ast.CompressHeadingLevels(bs) }
func astJoinLinks(bs []ast.Block) []ast.Block { return ast.JoinLinkRows(bs) }
