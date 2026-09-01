# html-to-md

aerc email→Markdown filters in a single binary. One conversion mode per flag,
auto-detected from the part when no flag is given (`BEGIN:VCALENDAR` →
calendar, HTML markers in the first 2 KB → html, else plain):

| Flag | MIME part | Purpose |
| --- | --- | --- |
| `--html` | `text/html` | vendor-noisy HTML → Markdown (typed-AST pipeline) |
| `--plain` | `text/plain` | dewrap RFC 3676 `format=flowed`; passthrough otherwise |
| `--calendar` | `text/calendar` | iCalendar invites/todos → Markdown |

## HOW IT WORKS

```
stdin HTML ─▶ pre-process ─▶ html5ever ─▶ clean ─▶ lower ─▶ transform ─▶ render ─▶ stdout
```

1. **Pre-process** — strip non-comment IE conditionals (`<![if …]>…<![endif]>`,
   an Outlook/Word-ism) before parsing.
2. **Parse** — html5ever via kuchikiki (spec-compliant HTML5 parsing).
3. **Clean** (`src/clean.rs`, `src/table.rs`) — DOM surgery passes: strip
   comments (catches `<!--[if mso]>` Outlook blocks), drop namespaced/hidden/
   non-text elements, normalise invisible characters, flatten CSS-flex rows
   and layout tables, demote stat-only headings, drop decorative/empty
   anchors. The pass order in `clean_doc` is load-bearing; each pass is
   documented at its definition.
4. **Lower** (`src/lower.rs`) — the cleaned DOM becomes a typed Markdown AST
   (`src/ast.rs`: `Block`/`Inline`). Every heuristic the old string pipeline
   re-parsed with regexes (heading levels, link-only blocks, visible widths)
   is expressed on typed nodes instead.
5. **Transform** (`src/ast.rs`) — heading remap and rank-compression,
   empty-section dropping, short-link-row joining, table column pruning.
6. **Render** (`src/render.rs`) — one deterministic pass: escape source text
   once, emit structural markers, pad pipe-table columns from typed widths.

Exactly one regex survives: the pre-parse IE-conditional strip — the only
step that must run before a parser exists.

## OUTPUT CONTRACT

The emitted Markdown is a small, predictable dialect that a pager (aerc's
built-in, nvim with treesitter conceal, or a future custom reader) can rely
on:

- **One line per paragraph.** No hard-wrapping; the pager owns soft-wrapping
  at the real terminal width (aerc passes no width to filters, so the old
  converter-side 80-column wrap and `AERC_FILTER_WIDTH` are gone).
- **Links are always single-line `[text](href)`** — `<br>`/newlines inside
  link text are folded to spaces, so concealers and `:open-link` always see a
  well-formed token. `[url](url)` and `[email](mailto:email)` collapse to the
  bare string; repeated `(label, href)` pairs inside a joined link row are
  dropped.
- **Tracking parameters are stripped from hrefs** (`utm_*`, `gclid`, `fbclid`,
  `dclid`, `msclkid`, `twclid`, `yclid`, `igshid`, `_hsenc`, `_hsmi`, `mc_cid`,
  `mc_eid`, `vero_*`, `s_kwcid`, `elqtrackid`, `gclsrc` — click-attribution
  only, never routing parameters). Nothing else in a URL is rewritten.
- **Headings are rank-compressed**: the shallowest heading in a document
  becomes `#` and the distinct levels present remap to a contiguous `1..n`
  range, so jumping source levels render as `#` → `##` rather than `#` →
  `####`. Heading text is whitespace-normalised to single spaces and emphasis
  markers inside headings are dropped (`#` already carries the weight).
- **`<br>` is a hard newline** inside paragraphs (signatures, address blocks
  and log dumps stay tight); `<br><br>` is a blank line. Emphasis markers
  never span a line break.
- **Layout tables become paragraphs**; data tables (with `<thead>`/`<caption>`
  or uniform bordered rows) become **minimally framed pipe tables** — single
  space between pipes and cells, no column padding, `| - |` separators — so
  they stay narrow and ragged rows never stretch the frame.
- **Structural characters** (`\`, `` ` ``, `*`, `_`, `[`, `]`) in source text
  are backslash-escaped at render time; unicode-escape sequences a broken
  template emitted as literal text (`\u2013`, `\U0001f9e0`) are decoded.

## USAGE (AERC)

```ini
[filters]
text/html = html-to-md
text/plain = html-to-md --plain
text/calendar = html-to-md --calendar
```

Or via the flake (home-manager):

```nix
inputs.html-to-md.packages.${pkgs.stdenv.hostPlatform.system}.default
```

(The flags are explicit in aerc because it already knows the MIME type; the
no-flag auto-detect serves manual pipes and `just try`.)

## TEXT/PLAIN

`html-to-md --plain` reads `AERC_FORMAT`: `flowed` triggers RFC 3676
dewrapping — soft-wrapped lines (trailing single space) join into one line
per paragraph, hard breaks stay, quote levels (`>`, `>>`) become blockquotes,
the `-- ` signature stays verbatim and tight, whitespace-stuffing is deleted,
and Outlook conditional-comment lines vendors leak into the plain part are
dropped. Any other value passes the part through untouched.

## TEXT/CALENDAR

`html-to-md --calendar` parses iCalendar by hand (no dependencies): line
unfolding, a component tree, and property lines split on the first unquoted
colon that is not inside a `KEY=…` parameter — which is what keeps unquoted
`DTSTART;TZID=Europe/Berlin:…` intact. It renders `VEVENT` and `VTODO`
components in the same dialect as the HTML path: one `#` heading, the time
range (`Tue, 1 Sep 2026, 17:00 – 18:30 (Europe/Berlin)` — weekdays via
Zeller's congruence, `Z` → `(UTC)`, all-day and `DURATION` forms included),
bold-labelled `**Location:**` / `**Organiser:**` facts, attendees with
participation marks (`✓` accepted, `✗` declined, `~` tentative), a
prettified `**Repeats:**` line for `RRULE`, and the description — routed
through the full HTML pipeline when a booking system embeds literal HTML in
it, tight lines otherwise (Google's decorative fence is stripped).

## DEVELOPMENT

```sh
just test        # cargo-nextest
just test-watch  # re-run on every change
just lint-check  # fmt --check + clippy -D warnings + nextest (what CI runs)
just try f.html  # render an HTML sample through the debug build
just try-mail 'tag:inbox' 5   # render the text/html part of recent mail
just try-ics f.ics            # render an iCalendar sample
just try-plain f.txt          # render a text sample as format=flowed
just nix-check   # nix flake check (the sandbox check phase runs nextest too)
just dev         # enter the flake dev shell
```

Behaviour is locked by golden tests (`tests/pipeline.rs`): each case pairs a
vendor-shaped HTML snippet (Outlook conditionals, Bitbucket PR notices,
Mailchimp newsletters, Sentry digests, …) with its exact expected Markdown.
New heuristics land together with a case.

## LICENSE

MIT — see [LICENSE](LICENSE).
