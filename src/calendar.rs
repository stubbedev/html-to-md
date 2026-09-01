//! iCalendar (RFC 5545) → Markdown for `text/calendar` parts.
//!
//! Hand-rolled parser, no dependencies: line unfolding, component tree
//! (BEGIN/END), and property lines split on the first unquoted colon that is
//! not inside a `KEY=…` parameter — the quirk that lets unquoted
//! `TZID=Europe/Berlin:…` values survive. Renders events and todos as the
//! same Markdown dialect the HTML path emits: one heading, bold-labelled
//! fact lines, attendees as a list, description paragraphs.

pub fn convert(input: &str) -> String {
    let roots = parse(input);
    if roots.is_empty() {
        // Not a parseable calendar — pass the part through rather than
        // swallowing it.
        return input.trim_end().to_string();
    }
    render_roots(&roots)
}

#[derive(Debug, Clone)]
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

#[derive(Debug, Clone, Default)]
struct Comp {
    name: String,
    props: Vec<Prop>,
    comps: Vec<Comp>,
}

impl Comp {
    fn prop(&self, name: &str) -> Option<&Prop> {
        self.props
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    fn prop_values(&self, name: &str) -> Vec<&Prop> {
        self.props
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(name))
            .collect()
    }
}

fn parse(input: &str) -> Vec<Comp> {
    // RFC 5545 folding: a line starting with a space or tab continues the
    // previous one; the leading whitespace char itself is dropped.
    let mut lines: Vec<String> = Vec::new();
    for raw in input.lines() {
        if raw.starts_with(' ') || raw.starts_with('\t') {
            if let Some(last) = lines.last_mut() {
                last.push_str(&raw[1..]);
                continue;
            }
        }
        lines.push(raw.to_string());
    }

    let mut roots: Vec<Comp> = Vec::new();
    let mut stack: Vec<Comp> = Vec::new();
    for line in &lines {
        let line = line.trim_end_matches('\r');
        if let Some(name) = line.strip_prefix("BEGIN:") {
            stack.push(Comp {
                name: name.trim().to_ascii_uppercase(),
                ..Comp::default()
            });
        } else if line.strip_prefix("END:").is_some() {
            if let Some(done) = stack.pop() {
                if stack.is_empty() {
                    roots.push(done);
                } else if let Some(parent) = stack.last_mut() {
                    parent.comps.push(done);
                }
            }
        } else if let Some(p) = parse_prop(line) {
            if let Some(top) = stack.last_mut() {
                top.props.push(p);
            }
        }
    }
    roots
}

/// Split a property line into name, parameters and value. The value starts at
/// the first unquoted colon whose preceding segment (since the last `;`) does
/// not contain `=` — that keeps unquoted `TZID=Europe/Berlin:…` intact while
/// still splitting `SUMMARY:Hello: world` on the first colon.
fn parse_prop(line: &str) -> Option<Prop> {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut seg_start = 0;
    let mut sep: Option<usize> = None;
    let mut first_colon: Option<usize> = None;
    for (i, &c) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            b'\\' if in_quotes => escaped = true,
            b'"' => in_quotes = !in_quotes,
            b';' if !in_quotes => seg_start = i + 1,
            b':' if !in_quotes => {
                if first_colon.is_none() {
                    first_colon = Some(i);
                }
                let seg = &line[seg_start..i];
                if !seg.contains('=') {
                    sep = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let sep = sep.or(first_colon)?;

    let head = &line[..sep];
    let value = line[sep + 1..].to_string();
    let mut segments = split_unquoted_semicolons(head).into_iter();
    let name = segments.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let mut params = Vec::new();
    for seg in segments {
        let (k, v) = match seg.split_once('=') {
            Some((k, v)) => (k, v),
            None => (seg, ""),
        };
        let v = v.trim();
        let v = if v.starts_with('"') && v.len() >= 2 && v.ends_with('"') {
            v[1..v.len() - 1].to_string()
        } else {
            v.to_string()
        };
        params.push((k.trim().to_ascii_uppercase(), v));
    }
    Some(Prop {
        name,
        params,
        value,
    })
}

/// Split on `;` outside of quoted parameter values (backslash escapes inside
/// quotes are honoured).
fn split_unquoted_semicolons(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render_roots(roots: &[Comp]) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for root in roots {
        for comp in &root.comps {
            match comp.name.as_str() {
                "VEVENT" => blocks.extend(render_component(comp, false)),
                "VTODO" => blocks.extend(render_component(comp, true)),
                _ => {}
            }
        }
    }
    let mut joined = blocks.join("\n\n");
    while joined.contains("\n\n\n") {
        joined = joined.replace("\n\n\n", "\n\n");
    }
    joined
}

fn render_component(comp: &Comp, todo: bool) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();

    if let Some(p) = comp.prop("SUMMARY") {
        let s = unescape_text(&p.value).trim().to_string();
        if !s.is_empty() {
            lines.push(format!("# {}", esc(&normalize_ws(&s))));
        }
    }

    let status = comp
        .prop("STATUS")
        .map(|p| p.value.trim().to_ascii_uppercase())
        .unwrap_or_default();
    match status.as_str() {
        "CANCELLED" => lines.push("**Cancelled**".to_string()),
        "TENTATIVE" => lines.push("**Tentative**".to_string()),
        "COMPLETED" => lines.push("**Completed**".to_string()),
        _ => {}
    }

    if todo {
        if let Some(due) = comp.prop("DUE") {
            if let Some(dt) = parse_datetime(due) {
                lines.push(format!("**Due:** {}", format_datetime(&dt)));
            }
        }
        if let Some(p) = comp.prop("PRIORITY") {
            match p.value.trim().parse::<u32>() {
                Ok(1..=4) => lines.push("**Priority:** high".to_string()),
                Ok(6..=9) => lines.push("**Priority:** low".to_string()),
                _ => {}
            }
        }
    } else {
        let when = format_event_time(comp);
        if !when.is_empty() {
            lines.push(when);
        }
    }

    if let Some(loc) = comp.prop("LOCATION") {
        let s = unescape_text(&loc.value).trim().to_string();
        if !s.is_empty() {
            lines.push(format!("**Location:** {}", esc(&s)));
        }
    }

    if let Some(conf) = comp.prop("X-GOOGLE-CONFERENCE") {
        let url = conf.value.trim();
        if !url.is_empty() {
            lines.push(format!("[Join call]({url})"));
        }
    }
    if let Some(url) = comp.prop("URL") {
        let u = url.value.trim();
        if !u.is_empty() {
            lines.push(format!("[Link]({u})"));
        }
    }

    lines.extend(render_people(comp));

    if let Some(rr) = comp.prop("RRULE") {
        let s = pretty_rrule(rr.value.trim());
        if !s.is_empty() {
            lines.push(format!("**Repeats:** {s}"));
        }
    }

    if let Some(desc) = comp.prop("DESCRIPTION") {
        lines.extend(render_description(&unescape_text(&desc.value)));
    }

    lines
}

fn format_event_time(comp: &Comp) -> String {
    let start = comp.prop("DTSTART").and_then(parse_datetime);
    let Some(start) = start else {
        return String::new();
    };
    match comp.prop("DTEND").and_then(parse_datetime) {
        Some(end) => format_range(&start, &end),
        None => match comp.prop("DURATION") {
            Some(d) => match parse_duration(&d.value) {
                Some(len) => format!(
                    "{} ({})",
                    format_datetime(&start),
                    format_duration_compact(&len)
                ),
                None => format_datetime(&start),
            },
            None => format_datetime(&start),
        },
    }
}

fn format_range(start: &DateTime, end: &DateTime) -> String {
    if start.all_day && end.all_day {
        if (start.y, start.m, start.d) == (end.y, end.m, end.d) {
            return format!("{} (all day)", format_date(start));
        }
        return format!("{} – {} (all day)", format_date(start), format_date(end));
    }
    if (start.y, start.m, start.d) == (end.y, end.m, end.d) {
        let tz = format_tz(start);
        return format!(
            "{}, {} – {}{tz}",
            format_date(start),
            format_time(start),
            format_time(end)
        );
    }
    format!(
        "{} – {}{}",
        format_datetime(start),
        format_datetime(end),
        format_tz(start)
    )
}

fn render_people(comp: &Comp) -> Vec<String> {
    match comp.prop("ORGANIZER") {
        Some(org) => {
            let (cn, email) = person(org);
            let mut s = String::from("**Organiser:** ");
            if !cn.is_empty() {
                s.push_str(&esc(&cn));
                if !email.is_empty() {
                    s.push_str(&format!(" <{email}>"));
                }
            } else if !email.is_empty() {
                s.push_str(&email);
            }
            let mut out = vec![s];
            out.extend(render_attendees(comp, &email));
            out
        }
        None => render_attendees(comp, ""),
    }
}

fn render_attendees(comp: &Comp, skip_email: &str) -> Vec<String> {
    let people: Vec<(String, String, String)> = comp
        .prop_values("ATTENDEE")
        .iter()
        .map(|p| {
            let (cn, email) = person(p);
            let partstat = p
                .params
                .iter()
                .find(|(k, _)| k == "PARTSTAT")
                .map(|(_, v)| v.to_ascii_uppercase())
                .unwrap_or_default();
            let mark = match partstat.as_str() {
                "ACCEPTED" => " ✓",
                "DECLINED" => " ✗",
                "TENTATIVE" => " ~",
                _ => "",
            };
            (cn, email, mark.to_string())
        })
        .filter(|(_, email, _)| !email.is_empty() && email != skip_email)
        .collect();
    if people.is_empty() {
        return vec![];
    }
    let label = if people.len() == 1 {
        "**Attendee:**"
    } else {
        "**Attendees:**"
    };
    if people.len() <= 3 {
        let list = people
            .iter()
            .map(|(cn, email, mark)| person_entry(cn, email, mark))
            .collect::<Vec<_>>()
            .join(", ");
        vec![format!("{label} {list}")]
    } else {
        let mut out = vec![label.to_string()];
        for (cn, email, mark) in &people {
            out.push(format!("- {}", person_entry(cn, email, mark)));
        }
        out
    }
}

fn person_entry(cn: &str, email: &str, mark: &str) -> String {
    let mut s = String::new();
    // Google sets CN to the bare email when no name is known; rendering
    // `email <email>` is duplicate noise.
    if !cn.is_empty() && !cn.eq_ignore_ascii_case(email) {
        s.push_str(&esc(cn));
    }
    if !email.is_empty() {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&format!("<{email}>"));
    }
    if s.is_empty() {
        s.push_str("(unnamed)");
    }
    s.push_str(mark);
    s
}

fn person(p: &Prop) -> (String, String) {
    let cn = p
        .params
        .iter()
        .find(|(k, _)| k == "CN")
        .map(|(_, v)| unescape_text(v).trim().to_string())
        .unwrap_or_default();
    (cn, mail_address(&p.value))
}

/// `mailto:user@host` → `user@host`; bare values pass through.
fn mail_address(v: &str) -> String {
    v.trim()
        .strip_prefix("mailto:")
        .unwrap_or(v.trim())
        .to_string()
}

// ─── Description ─────────────────────────────────────────────────────────────

fn render_description(raw: &str) -> Vec<String> {
    // Booking systems (and some CRM tools) embed literal HTML in the
    // DESCRIPTION text value. Route those through the HTML pipeline so the
    // output stays in dialect; plain descriptions keep the tight-lines path.
    if looks_like_html(raw) {
        let converted = crate::convert(raw);
        if !converted.trim().is_empty() {
            return vec![converted];
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        let t = line.trim();
        if is_google_fence(t) || t.eq_ignore_ascii_case("Please do not edit this section.") {
            continue;
        }
        if t.is_empty() {
            if let Some(last) = lines.last() {
                if !last.is_empty() {
                    lines.push(String::new());
                }
            }
            continue;
        }
        lines.push(esc(&normalize_ws(t)));
    }
    // Collapse to a single tight block: consecutive lines stay consecutive.
    let mut out: Vec<String> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    for l in lines {
        if l.is_empty() {
            if !para.is_empty() {
                out.push(para.join("\n"));
                para.clear();
            }
        } else {
            para.push(l);
        }
    }
    if !para.is_empty() {
        out.push(para.join("\n"));
    }
    out
}

/// Heuristic: the description carries real HTML markup (not just a stray `<`
/// in prose). Require a closing tag or a known block/inline tag to trigger.
fn looks_like_html(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    [
        "</p>", "<br", "<div", "<p>", "<strong", "<em>", "<ul", "<table>",
    ]
    .iter()
    .any(|tag| lower.contains(tag))
}

/// Google Meet invites fence their description with `-::~:~:…::-` lines.
fn is_google_fence(t: &str) -> bool {
    t.len() >= 3 && t.contains('~') && t.chars().all(|c| matches!(c, '-' | ':' | '~' | ' '))
}

// ─── Datetime ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tz {
    Utc,
    Floating,
}

#[derive(Debug, Clone)]
struct DateTime {
    y: i32,
    m: u32,
    d: u32,
    h: Option<u32>,
    min: Option<u32>,
    tz: Tz,
    tzid: Option<String>,
    all_day: bool,
}

fn parse_datetime(p: &Prop) -> Option<DateTime> {
    let v = p.value.trim();
    let (tz, tzid) = if v.ends_with('Z') {
        (Tz::Utc, None)
    } else {
        let tzid = p
            .params
            .iter()
            .find(|(k, _)| k == "TZID")
            .map(|(_, v)| v.clone());
        (Tz::Floating, tzid)
    };
    let core = v.trim_end_matches(['Z', 'z']);
    if core.len() >= 15 && core.as_bytes()[8] == b'T' {
        let date = parse_date(&core[..8])?;
        let h = core[9..11].parse::<u32>().ok()?;
        let min = core[11..13].parse::<u32>().ok()?;
        Some(DateTime {
            y: date.0,
            m: date.1,
            d: date.2,
            h: Some(h),
            min: Some(min),
            tz,
            tzid,
            all_day: false,
        })
    } else if core.len() == 8 {
        let (y, m, d) = parse_date(core)?;
        Some(DateTime {
            y,
            m,
            d,
            h: None,
            min: None,
            tz,
            tzid,
            all_day: true,
        })
    } else {
        None
    }
}

fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let b = s.as_bytes();
    if b.len() != 8 || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let m: u32 = s[4..6].parse().ok()?;
    let d: u32 = s[6..8].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

const WEEKDAYS: [&str; 7] = ["Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Zeller's congruence; h = 0 → Saturday.
fn weekday(y: i32, m: u32, d: u32) -> &'static str {
    let (yy, mm) = if m < 3 {
        (y as i64 - 1, m as i64 + 12)
    } else {
        (y as i64, m as i64)
    };
    let k = yy.rem_euclid(100);
    let j = yy.div_euclid(100);
    let h = (d as i64 + (13 * (mm + 1)) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    WEEKDAYS[h as usize]
}

fn format_date(dt: &DateTime) -> String {
    format!(
        "{}, {} {} {}",
        weekday(dt.y, dt.m, dt.d),
        dt.d,
        MONTHS[(dt.m - 1) as usize],
        dt.y
    )
}

fn format_time(dt: &DateTime) -> String {
    format!("{:02}:{:02}", dt.h.unwrap_or(0), dt.min.unwrap_or(0))
}

fn format_tz(dt: &DateTime) -> String {
    if let Some(tzid) = &dt.tzid {
        format!(" ({tzid})")
    } else if dt.tz == Tz::Utc {
        " (UTC)".to_string()
    } else {
        String::new()
    }
}

fn format_datetime(dt: &DateTime) -> String {
    if dt.all_day {
        return format!("{} (all day)", format_date(dt));
    }
    format!("{}{}, {}", format_date(dt), format_tz(dt), format_time(dt))
}

// ─── Duration ────────────────────────────────────────────────────────────────

fn parse_duration(v: &str) -> Option<(u32, u32, u32, u32, u32)> {
    // (weeks, days, hours, minutes, seconds)
    let mut weeks = 0;
    let mut days = 0;
    let mut hours = 0;
    let mut mins = 0;
    let mut secs = 0;
    let mut in_time = false;
    let mut num = String::new();
    for c in v.trim().strip_prefix(['P', 'p'])?.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        match c {
            'T' | 't' => in_time = true,
            'W' | 'w' => weeks = num.parse().ok()?,
            'D' | 'd' if !in_time => days = num.parse().ok()?,
            'H' | 'h' => hours = num.parse().ok()?,
            'M' | 'm' if in_time => mins = num.parse().ok()?,
            'S' | 's' => secs = num.parse().ok()?,
            _ => return None,
        }
        num.clear();
    }
    Some((weeks, days, hours, mins, secs))
}

fn format_duration_compact(d: &(u32, u32, u32, u32, u32)) -> String {
    let (w, dd, h, m, s) = *d;
    let mut parts: Vec<String> = Vec::new();
    if w > 0 {
        parts.push(unit(w, "week"));
    }
    if dd > 0 {
        parts.push(unit(dd, "day"));
    }
    if h > 0 {
        parts.push(format!("{h} h"));
    }
    if m > 0 {
        parts.push(format!("{m} m"));
    }
    if s > 0 {
        parts.push(format!("{s} s"));
    }
    if parts.is_empty() {
        return "0 m".to_string();
    }
    parts.join(" ")
}

fn unit(n: u32, name: &str) -> String {
    if n == 1 {
        format!("1 {name}")
    } else {
        format!("{n} {name}s")
    }
}

// ─── RRULE ───────────────────────────────────────────────────────────────────

fn pretty_rrule(v: &str) -> String {
    let mut freq = "";
    let mut interval = 1usize;
    let mut count: Option<usize> = None;
    let mut until = String::new();
    let mut byday: Vec<&str> = Vec::new();
    for part in v.split(';') {
        let Some((k, val)) = part.split_once('=') else {
            continue;
        };
        match k.to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = match val.to_ascii_uppercase().as_str() {
                    "DAILY" => "day",
                    "WEEKLY" => "week",
                    "MONTHLY" => "month",
                    "YEARLY" => "year",
                    _ => "",
                }
            }
            "INTERVAL" => interval = val.parse().unwrap_or(1),
            "COUNT" => count = val.parse().ok(),
            "UNTIL" => until = val.trim_end_matches('Z').to_string(),
            "BYDAY" => {
                byday = val
                    .split(',')
                    .filter_map(|d| match d.trim() {
                        "MO" => Some("Mon"),
                        "TU" => Some("Tue"),
                        "WE" => Some("Wed"),
                        "TH" => Some("Thu"),
                        "FR" => Some("Fri"),
                        "SA" => Some("Sat"),
                        "SU" => Some("Sun"),
                        _ => None,
                    })
                    .collect();
            }
            _ => {}
        }
    }
    if freq.is_empty() {
        return v.to_string();
    }
    let mut out = String::new();
    match interval {
        1 => out.push_str(&format!("{freq}s")),
        n => out.push_str(&format!("every {n} {freq}s")),
    }
    if !byday.is_empty() {
        out.push_str(&format!(" on {}", byday.join(", ")));
    }
    if let Some(c) = count {
        out.push_str(&format!(", {c} times"));
    } else if !until.is_empty() {
        if let Some((y, m, d)) = parse_date(&until) {
            out.push_str(&format!(
                ", until {}, {} {} {}",
                weekday(y, m, d),
                d,
                MONTHS[(m - 1) as usize],
                y
            ));
        }
    }
    out
}

// ─── Text helpers ────────────────────────────────────────────────────────────

/// RFC 5545 text escapes: `\n`/`\N` → newline; `\,` `\;` `\\` unescaped;
/// undefined escapes (`\(`, …) are treated as the bare character — broken
/// producer templates are common and the backslash is never meaningful.
fn unescape_text(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built with concat! so the ICS continuation line keeps its leading
    // space — a Rust `\`-string continuation would eat it and break folding.
    const INVITE: &str = concat!(
        "BEGIN:VCALENDAR\r\n",
        "PRODID:-//Google Inc//Google Calendar 70.9054//EN\r\n",
        "VERSION:2.0\r\n",
        "METHOD:REQUEST\r\n",
        "BEGIN:VEVENT\r\n",
        "DTSTART;TZID=Europe/Berlin:20260128T173000\r\n",
        "DTEND;TZID=Europe/Berlin:20260128T190000\r\n",
        "ORGANIZER;CN=CPH - Bookings:mailto:bookings@example.com\r\n",
        "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;CN=Maria \r\n",
        " Stage;X-NUM-GUESTS=0:mailto:maria@example.com\r\n",
        "ATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Second Guest:mailto:second@example.com\r\n",
        "LOCATION:Beta Boulders \\, Sydhavn\r\n",
        "X-GOOGLE-CONFERENCE:https://meet.google.com/abc-def-ghi\r\n",
        "DESCRIPTION:-::~:~::~:~::-\\nJoin with Google Meet: https://meet.google.com/ab\r\n",
        " c-def-ghi\\n\\nBring chalk.\\nPlease do not edit this section.\\n-::~:~::~:~::-\r\n",
        "STATUS:CONFIRMED\r\n",
        "SUMMARY:#127550 Beginner Class \\(Booking\\)\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n",
    );

    #[test]
    fn renders_realistic_google_invite() {
        let out = convert(INVITE);
        assert_eq!(
            out,
            "# #127550 Beginner Class (Booking)\n\n\
Wed, 28 Jan 2026, 17:30 – 19:00 (Europe/Berlin)\n\n\
**Location:** Beta Boulders , Sydhavn\n\n\
[Join call](https://meet.google.com/abc-def-ghi)\n\n\
**Organiser:** CPH - Bookings <bookings@example.com>\n\n\
**Attendees:** Maria Stage <maria@example.com> ✓, Second Guest <second@example.com>\n\n\
Join with Google Meet: https://meet.google.com/abc-def-ghi\n\n\
Bring chalk."
        );
    }

    #[test]
    fn splits_unquoted_tzid_colon_correctly() {
        let p = parse_prop("DTSTART;TZID=Europe/Berlin:20260128T173000").unwrap();
        assert_eq!(p.name, "DTSTART");
        assert_eq!(
            p.params,
            vec![("TZID".to_string(), "Europe/Berlin".to_string())]
        );
        assert_eq!(p.value, "20260128T173000");
    }

    #[test]
    fn splits_quoted_params_and_first_colon_values() {
        let p = parse_prop(r#"ATTENDEE;CN="Doe; John":mailto:a@b"#).unwrap();
        assert_eq!(p.params, vec![("CN".to_string(), "Doe; John".to_string())]);
        assert_eq!(p.value, "mailto:a@b");
        let p2 = parse_prop("SUMMARY:a=b:c").unwrap();
        assert_eq!(p2.name, "SUMMARY");
        assert_eq!(p2.value, "a=b:c");
    }

    #[test]
    fn all_day_and_utc_and_duration() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260901\r\n\
SUMMARY:All-day off\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20260902T100000Z\r\n\
DURATION:PT1H30M\r\nSUMMARY:Sync\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let out = convert(ics);
        assert_eq!(
            out,
            "# All-day off\n\nTue, 1 Sep 2026 (all day)\n\n# Sync\n\n\
Wed, 2 Sep 2026 (UTC), 10:00 (1 h 30 m)"
        );
    }

    #[test]
    fn cancelled_and_rrule() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260901T090000Z\r\n\
DTEND:20260901T100000Z\r\nSTATUS:CANCELLED\r\n\
RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10\r\n\
SUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let out = convert(ics);
        assert_eq!(
            out,
            "# Standup\n\n**Cancelled**\n\n\
Tue, 1 Sep 2026, 09:00 – 10:00 (UTC)\n\n\
**Repeats:** every 2 weeks on Mon, Wed, 10 times"
        );
    }

    #[test]
    fn non_calendar_passthrough() {
        assert_eq!(convert("just some text"), "just some text");
    }

    #[test]
    fn html_description_routed_through_html_pipeline() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260901T090000Z\r\n\
DTEND:20260901T100000Z\r\n\
DESCRIPTION:<p><strong>Status</strong>: Paid</p><p><strong>Location</strong>: SYDHAVN</p>\r\n\
SUMMARY:Booking\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let out = convert(ics);
        assert!(out.contains("**Status**: Paid"));
        assert!(out.contains("**Location**: SYDHAVN"));
        assert!(!out.contains("<p>"));
    }

    #[test]
    fn attendee_with_email_as_cn_is_not_duplicated() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260901T090000Z\r\n\
ORGANIZER;CN=Org:mailto:org@example.com\r\n\
ATTENDEE;PARTSTAT=NEEDS-ACTION;CN=guest@example.com:mailto:guest@example.com\r\n\
SUMMARY:Meet\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let out = convert(ics);
        assert!(out.contains("**Attendee:** <guest@example.com>"));
        assert!(!out.contains("guest@example.com <guest@example.com>"));
    }

    #[test]
    fn unescapes_text_values() {
        assert_eq!(unescape_text(r"a\,b\;c\nd\\e"), "a,b;c\nd\\e");
    }
}
