//! aerc filter binary: read a mail part from stdin, write Markdown to stdout.
//! The conversion mode comes from a flag; with no flag the input is sniffed
//! (see [`html_to_md::detect`]). All the work lives in the library crate.

use std::error::Error;
use std::io::{self, Read, Write};

use html_to_md::Format;

const USAGE: &str = "\
html-to-md — render email parts as Markdown

usage: html-to-md [--html | --plain | --calendar]

  --html      force the HTML pipeline (vendor-noisy HTML → Markdown)
  --plain     dewrap RFC 3676 format=flowed (AERC_FORMAT=flowed); passthrough otherwise
  --calendar  render an iCalendar part (invites, todos)
  (no flag)   auto-detect: BEGIN:VCALENDAR → calendar, HTML markers → html, else plain
";

fn main() -> Result<(), Box<dyn Error>> {
    let mut mode: Option<Format> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--html" => mode = Some(Format::Html),
            "--plain" => mode = Some(Format::Plain),
            "--calendar" => mode = Some(Format::Calendar),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => {
                eprintln!("html-to-md: unknown argument `{other}`\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let input = {
        // Filters are handed UTF-8 by aerc, but decode lossily anyway so a
        // stray non-UTF-8 byte degrades to U+FFFD instead of killing the render.
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        String::from_utf8_lossy(&bytes).into_owned()
    };

    let out = match mode.unwrap_or_else(|| html_to_md::detect(&input)) {
        Format::Html => html_to_md::convert(&input),
        Format::Plain => {
            // aerc passes the part's content-type format= parameter here.
            let flowed = std::env::var("AERC_FORMAT")
                .map(|v| v.eq_ignore_ascii_case("flowed"))
                .unwrap_or(false);
            html_to_md::plain::render(&input, flowed)
        }
        Format::Calendar => html_to_md::calendar::convert(&input),
    };
    io::stdout().write_all(out.as_bytes())?;
    Ok(())
}
