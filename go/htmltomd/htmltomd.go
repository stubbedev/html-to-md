// Package htmltomd is the importable face of html-to-md: the same
// conversions the binary runs, for programs that want them in-process
// instead of shelling out.
package htmltomd

import "github.com/stubbedev/html-to-md/go/internal/convert"

// Convert renders an HTML email body as Markdown, byte-for-byte what
// `html-to-md --html` prints.
func Convert(html string) string { return convert.Convert(html) }
