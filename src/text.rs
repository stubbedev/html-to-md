//! Text-node normalisation: invisible-character stripping and literal
//! unicode-escape decoding, applied to every text node before serialisation.

/// Replace zero-width / format chars with nothing and NBSP-class spaces with
/// a regular space inside every text node. Done in-place on the live tree so
/// later passes (drop_empty_anchors, table-cell blankness checks) see the
/// cleaned text.
pub fn clean_invisibles(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // Zero-width / format characters that emails use as preview-text
            // padding. Drop entirely.
            '\u{00AD}' // soft hyphen
            | '\u{034F}' // combining grapheme joiner (Klaviyo et al.)
            | '\u{061C}' // arabic letter mark
            | '\u{115F}' // hangul choseong filler
            | '\u{1160}' // hangul jungseong filler
            | '\u{17B4}' // khmer vowel inherent aq
            | '\u{17B5}' // khmer vowel inherent aa
            | '\u{180E}' // mongolian vowel separator
            | '\u{200B}' // zero-width space
            | '\u{200C}' // ZWNJ
            | '\u{200D}' // ZWJ
            | '\u{200E}' // LRM
            | '\u{200F}' // RLM
            | '\u{202A}'..='\u{202E}' // bidi formatting
            | '\u{2060}' // word joiner
            | '\u{2061}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}' // bidi isolates
            | '\u{3164}' // hangul filler
            | '\u{FE00}'..='\u{FE0F}' // variation selectors
            | '\u{FEFF}' // BOM / zero-width nbsp
            | '\u{FFA0}' // halfwidth hangul filler
            | '\u{E0020}'..='\u{E007F}' // tag characters
            => {}
            // NBSP-class horizontal whitespace → plain space so post-processing
            // can collapse runs and trim() works as expected.
            '\u{00A0}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

/// Single non-alphanumeric character links (›, », →, ▸, etc.) are decorative
/// icon links that add noise in a text/terminal reader.
pub fn is_decorative_glyph(s: &str) -> bool {
    let mut chars = s.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_alphanumeric())
}

/// Query parameters that only carry click attribution. Stripping them from
/// http(s) hrefs never changes the destination; hrefs are otherwise never
/// rewritten (see the output contract in README.md). Non-http(s) schemes
/// (mailto:, tel:, …) pass through untouched.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "gclid",
    "gclsrc",
    "dclid",
    "fbclid",
    "msclkid",
    "twclid",
    "yclid",
    "igshid",
    "_hsenc",
    "_hsmi",
    "vero_id",
    "vero_conv",
    "mc_cid",
    "mc_eid",
    "s_kwcid",
    "elqtrackid",
];

pub fn strip_tracking_params(href: &str) -> String {
    if !(href.starts_with("http://") || href.starts_with("https://")) {
        return href.to_string();
    }
    let Some((base, rest)) = href.split_once('?') else {
        return href.to_string();
    };
    let (query, frag) = match rest.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (rest, None),
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            if kv.is_empty() {
                return false;
            }
            let key = kv.split('=').next().unwrap_or("").to_ascii_lowercase();
            !TRACKING_PARAMS.contains(&key.as_str())
        })
        .collect();
    let mut out = if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Decode literal unicode escape sequences that broken sender templates emit as
/// visible text instead of the character itself: `\uXXXX` (with UTF-16 surrogate
/// pairing), `\u{XXXX}` and `\UXXXXXXXX`. No human types these into email body
/// copy, so a stray `–`/`\U0001f9e0` is always a templating bug — turn it
/// back into `–`/`🧠`. Anything that isn't a well-formed escape is left verbatim.
pub fn decode_unicode_escapes(s: &str) -> String {
    if !s.contains("\\u") && !s.contains("\\U") {
        return s.to_string();
    }
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let hex = |slice: &[char]| -> Option<u32> {
        if slice.iter().all(|h| h.is_ascii_hexdigit()) {
            u32::from_str_radix(&slice.iter().collect::<String>(), 16).ok()
        } else {
            None
        }
    };
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\\' && i + 1 < c.len() && (c[i + 1] == 'u' || c[i + 1] == 'U') {
            // `\u{...}` brace form.
            if c[i + 1] == 'u' && i + 2 < c.len() && c[i + 2] == '{' {
                if let Some(end) = c[i + 3..].iter().position(|&ch| ch == '}') {
                    if let Some(cp) = hex(&c[i + 3..i + 3 + end]) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                            i += 3 + end + 1;
                            continue;
                        }
                    }
                }
            }
            let width = if c[i + 1] == 'U' { 8 } else { 4 };
            if i + 2 + width <= c.len() {
                if let Some(cp) = hex(&c[i + 2..i + 2 + width]) {
                    // `\uXXXX` high surrogate followed by a low surrogate → pair.
                    if width == 4 && (0xD800..=0xDBFF).contains(&cp) {
                        let j = i + 6;
                        if j + 6 <= c.len() && c[j] == '\\' && c[j + 1] == 'u' {
                            if let Some(lo) = hex(&c[j + 2..j + 6]) {
                                if (0xDC00..=0xDFFF).contains(&lo) {
                                    let cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    if let Some(ch) = char::from_u32(cp) {
                                        out.push(ch);
                                        i = j + 6;
                                        continue;
                                    }
                                }
                            }
                        }
                    } else if let Some(ch) = char::from_u32(cp) {
                        out.push(ch);
                        i += 2 + width;
                        continue;
                    }
                }
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_known_tracking_params() {
        assert_eq!(
            strip_tracking_params("https://x.com/signup?utm_source=news&utm_medium=email&id=7"),
            "https://x.com/signup?id=7"
        );
        // Bare tracking-only queries leave no dangling '?'.
        assert_eq!(
            strip_tracking_params("https://x.com/a?fbclid=AbCd"),
            "https://x.com/a"
        );
        // Keys are matched case-insensitively; fragments survive.
        assert_eq!(
            strip_tracking_params("https://x.com/p?UTM_Source=a&keep=1#section"),
            "https://x.com/p?keep=1#section"
        );
        // Non-http(s) schemes and non-http URLs are untouched.
        assert_eq!(
            strip_tracking_params("mailto:me@example.com?utm_source=x"),
            "mailto:me@example.com?utm_source=x"
        );
        assert_eq!(strip_tracking_params("https://x.com"), "https://x.com");
        assert_eq!(
            strip_tracking_params("https://x.com/a?gclid=1&=2&&ok=3"),
            "https://x.com/a?=2&ok=3"
        );
    }

    #[test]
    fn decodes_literal_unicode_escapes() {
        // 4-hex, 8-hex, brace form, and a UTF-16 surrogate pair (🧠).
        assert_eq!(
            decode_unicode_escapes(r"June 13 – June 20"),
            "June 13 – June 20"
        );
        assert_eq!(decode_unicode_escapes(r"\U0001f9e0 Stop"), "🧠 Stop");
        assert_eq!(decode_unicode_escapes(r"x\u{1F9E0}y"), "x🧠y");
        assert_eq!(decode_unicode_escapes(r"🧠"), "🧠");
        // Non-escapes and malformed sequences pass through untouched.
        assert_eq!(
            decode_unicode_escapes(r"the \understood plan"),
            r"the \understood plan"
        );
        assert_eq!(decode_unicode_escapes(r"\uZZZZ"), r"\uZZZZ");
        assert_eq!(decode_unicode_escapes("no escapes here"), "no escapes here");
        // Lone high surrogate is invalid → left verbatim.
        assert_eq!(decode_unicode_escapes(r"\uD83E!"), r"\uD83E!");
    }
}
