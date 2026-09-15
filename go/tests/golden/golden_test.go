// Golden tests for the full pipeline, ported from tests/pipeline.rs. Each
// case pairs a vendor-shaped HTML snippet with the exact expected Markdown;
// the expected strings are the contract the pager (and any future reader)
// relies on.
package golden

import (
	"strings"
	"testing"

	"github.com/stubbe/html-to-md/internal/convert"
)

var golden = []struct {
	name, input, want string
}{
	{"ie_conditional", "<p><![if !supportLists]><span>·</span><![endif]>Bullet one</p>", "Bullet one"},
	{"mso_comment", "<!--[if mso]><style>x{}</style><![endif]--><p>Hello</p>", "Hello"},
	{"namespaced", "<p>Hi<o:p>drop</o:p>Bye</p>", "HiBye"},
	{"hidden", "<p>visible</p><div style=\"display:none\">one</div><div style=\"visibility: hidden\">two</div>", "visible"},
	{"media_drop", "<style>a{}</style><script>b()</script><img src=\"x\"><p>text</p>", "text"},
	{"invisibles", "<p>A\u200bB&nbsp;C\u00adD</p>", "AB CD"},
	{"empty_anchor", "<a href=\"https://x.com/\"><img src=\"logo.png\"></a><p>body</p>", "body"},
	{"decorative_anchor", "<p><a href=\"/next\">›</a> <a href=\"/i\">Item</a></p>", "[Item](/i)"},
	{"punct_emphasis", "<p><em>=</em> and <strong>:</strong> done</p>", "= and : done"},
	{"ws_emphasis", "<p>alpha<em> </em>beta</p>", "alpha beta"},
	{"stat_heading", "<h1>471k</h1><p>details</p>", "**471k**\n\ndetails"},
	{"heading_shift", "<h3>Small title</h3><p>body</p>", "# Small title\n\nbody"},
	{"heading_compress", "<h1>Promo</h1><h4>Real section</h4><p>body</p>", "# Promo\n\n## Real section\n\nbody"},
	{"empty_section", "<h2>REVIEWERS</h2><h2>NEW ACTIVITY</h2><p>stuff</p>", "# NEW ACTIVITY\n\nstuff"},
	{"flex_row", "<div style=\"display: flex; gap: 4px\"><div>12</div><div><a href=\"/i/42\">Issue 42</a></div><div>resolved</div></div>", "12 [Issue 42](/i/42) resolved"},
	{"layout_table", "<table><tr><td><p>Hello</p></td></tr><tr><td><p>World</p></td></tr></table>", "Hello\nWorld"},
	{"data_table", "<table border=\"1\"><thead><tr><th>Name</th><th>Qty</th></tr></thead><tbody><tr><td>Apples</td><td>3</td></tr><tr><td>Pears</td><td>12</td></tr></tbody></table>", "| Name | Qty |\n| - | - |\n| Apples | 3 |\n| Pears | 12 |"},
	{"col_prune", "<table border=\"1\"><thead><tr><th>Name</th><th></th></tr></thead><tbody><tr><td>A</td><td></td></tr><tr><td>B</td><td></td></tr></tbody></table>", "| Name |\n| - |\n| A |\n| B |"},
	{"ragged_table", "<table border=\"1\"><thead><tr><th>Name</th></tr></thead><tbody><tr><td>A</td><td>Extra</td></tr></tbody></table>", "| Name |  |\n| - | - |\n| A | Extra |"},
	{"link_identity", "<p><a href=\"https://example.com/\">https://example.com/</a></p>", "https://example.com/"},
	{"link_mailto_identity", "<p><a href=\"mailto:me@example.com\">me@example.com</a></p>", "me@example.com"},
	{"link_tracking_stripped", "<p><a href=\"https://x.com/signup?utm_source=news&utm_medium=email&id=7\">Sign up</a></p>", "[Sign up](https://x.com/signup?id=7)"},
	{"link_identity_after_tracking_strip", "<p><a href=\"https://x.com/?utm_source=a\">https://x.com/</a></p>", "https://x.com/"},
	{"link_garbage_href", "<p><a href=\"Legaldesk.dk Njalsgade 21\">Legaldesk</a></p>", "Legaldesk"},
	{"link_multiline", "<p><a href=\"https://x.com/a\">multi\nline</a></p>", "[multi line](https://x.com/a)"},
	{"link_run_join", "<p><a href=\"/a\">About</a></p><p><a href=\"/b\">Blog</a></p><p><a href=\"/c\">Shop</a></p>", "[About](/a) · [Blog](/b) · [Shop](/c)"},
	{"link_run_join_dedup", "<p><a href=\"/\">Home</a></p><p><a href=\"/\">Home</a></p><p><a href=\"/b\">Blog</a></p>", "[Home](/) · [Blog](/b)"},
	{"strong_adjacency", "<p><b>So, w</b><b>atch this</b></p>", "**So, watch this**"},
	{"emph_space_boundaries", "<p><b>Twenty</b><b>&nbsp;minutes</b></p>", "**Twenty** **minutes**"},
	{"br_semantics", "<p>line1<br>line2<br><br>para2</p>", "line1\nline2\n\npara2"},
	{"pre", "<pre>code  \n  indented  \n</pre>", "```\ncode\n  indented\n```"},
	{"blockquote", "<blockquote><p>quoted text</p><p>more</p></blockquote>", "> quoted text\n>\n> more"},
	{"nested_lists", "<ul><li>a</li><li>b<ul><li>b1</li><li>b2</li></ul></li></ul>", "- a\n- b\n  - b1\n  - b2"},
	{"ordered_gap", "<ol><li>first</li><li></li><li>third</li></ol>", "1. first\n3. third"},
	{"escapes", "<p>a * b _ c [ d ] e \\ f ` g # h</p>", "a \\* b \\_ c \\[ d \\] e \\\\ f \\` g # h"},
	{"punct_block", "<p> . </p><p>real</p>", "real"},
	{"unicode_body", "<p>June 13 \\u2013 June 20</p>", "June 13 – June 20"},
	{"inline_block_grouping", "<div>for the account <strong>x</strong>.<ul><li>item</li></ul></div>", "for the account **x**.\n\n- item"},
	{"wrapper_cell", "<table><tr><td><p>Hello</p></td><td>World</td></tr><tr><td>a</td><td>b</td></tr></table>", "Hello World\na b"},
	{"blocks_cell", "<table><tr><td><p>Title</p><p>Desc</p></td></tr><tr><td>x</td></tr></table>", "Title\nDesc\nx"},
	{"emph_multiline", "<p><b>line1<br>line2</b></p>", "**line1**\n**line2**"},
	{"code_inline", "<p>run <code>let x = 1;</code> now</p>", "run `let x = 1;` now"},
	{"hr", "<p>a</p><hr><p>b</p>", "a\n\n---\n\nb"},
	{"heading_normalised", "<h2><strong>Normal</strong>  Sauna  <em>Hours</em></h2><p>body</p>", "# Normal Sauna Hours\n\nbody"},
}

func TestGolden(t *testing.T) {
	for _, tt := range golden {
		t.Run(tt.name, func(t *testing.T) {
			got := convert.Convert(tt.input)
			if got != tt.want {
				t.Errorf("\nexpected:\n%s\nactual:\n%s", tt.want, got)
			}
		})
	}
}

func TestDetectsFormats(t *testing.T) {
	cases := []struct {
		in   string
		want convert.Format
	}{
		{"<!DOCTYPE html><html><body>", convert.Html},
		{"<div>fragment</div>", convert.Html},
		{"\n   \ufeffBEGIN:VCALENDAR\r\nVERSION:2.0", convert.Calendar},
		{"plain prose\nover lines\n", convert.Plain},
		{strings.Repeat("x", 3000) + "<div>", convert.Plain},
		{"a < b and <b>bold</b> dreams", convert.Plain},
	}
	for _, c := range cases {
		if got := convert.Detect(c.in); got != c.want {
			t.Errorf("Detect(%q) = %v, want %v", c.in, got, c.want)
		}
	}
}
