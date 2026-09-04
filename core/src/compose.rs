//! The way out: a composed message's markers become the `formatted_body` other
//! clients read. The counterpart of `markup`, which walks the way in. Every `<`
//! in the output is written here - the input is text, never markup.

/// Beyond this a message is not one, and the output grows by a constant factor
/// per marker.
const MAX_INPUT: usize = 32 * 1024;
/// How deep markers may sit inside one another.
const MAX_DEPTH: u32 = 8;

struct Marker {
    delim: &'static str,
    open: &'static str,
    close: &'static str,
}

/// Two-character markers first: `*` would otherwise take the first half of `**`.
const MARKERS: [Marker; 4] = [
    Marker { delim: "**", open: "<strong>", close: "</strong>" },
    Marker { delim: "~~", open: "<del>", close: "</del>" },
    Marker { delim: "++", open: "<u>", close: "</u>" },
    Marker { delim: "*", open: "<em>", close: "</em>" },
];

/// Builds `formatted_body` from a composed message, or `None` where the text
/// carries no markers: a plain message must stay plain, or every line would
/// ship a second copy of itself for nothing.
pub fn to_formatted_body(text: &str) -> Option<String> {
    if text.len() > MAX_INPUT {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut marked = false;
    emit(&chars, &mut out, &mut marked, 0);
    if marked {
        Some(out)
    } else {
        None
    }
}

fn emit(chars: &[char], out: &mut String, marked: &mut bool, depth: u32) {
    let mut i = 0;
    while i < chars.len() {
        // A code span wins over everything: what is quoted as code is quoted as
        // it stands, markers included.
        if chars[i] == '`' {
            if let Some(end) = closing_backtick(chars, i + 1) {
                *marked = true;
                out.push_str("<code>");
                escape(&chars[i + 1..end], out);
                out.push_str("</code>");
                i = end + 1;
                continue;
            }
        }

        if depth < MAX_DEPTH {
            if let Some((marker, end)) = opening(chars, i) {
                let width = marker.delim.chars().count();
                *marked = true;
                out.push_str(marker.open);
                emit(&chars[i + width..end], out, marked, depth + 1);
                out.push_str(marker.close);
                i = end + width;
                continue;
            }
        }

        // A line break is not a marker: it needs the tag here, but on its own it
        // is no reason to send a formatted copy at all.
        if chars[i] == '\n' {
            out.push_str("<br>");
        } else {
            escape(&chars[i..i + 1], out);
        }
        i += 1;
    }
}

/// The end of a code span, or `None` where it is never closed.
fn closing_backtick(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\n' {
            return None;
        }
        if chars[i] == '`' {
            return if i > from { Some(i) } else { None };
        }
        i += 1;
    }
    None
}

/// Whether a marker opens at `i`, and where its partner starts. The rules are
/// deliberately narrower than Markdown's: a delimiter has to touch its content
/// on the inside and nothing may be empty.
fn opening(chars: &[char], i: usize) -> Option<(&'static Marker, usize)> {
    for marker in MARKERS.iter() {
        let width = marker.delim.chars().count();
        if !starts_with(chars, i, marker.delim) {
            continue;
        }
        let inner = i + width;
        // The content begins right here, and it is not the delimiter again.
        match chars.get(inner) {
            None => continue,
            Some(next) if next.is_whitespace() => continue,
            Some(next) if width == 1 && *next == '*' => continue,
            _ => {}
        }
        // `2*3*4` is arithmetic, not emphasis. Only the single-character marker
        // can be caught this way; the wide ones are unambiguous.
        if width == 1 && digit_at(chars, i.wrapping_sub(1), i > 0) && digit_at(chars, inner, true) {
            continue;
        }
        if let Some(end) = closing(chars, inner, marker.delim) {
            return Some((marker, end));
        }
    }
    None
}

/// Where the partner delimiter starts, counted from `from`.
fn closing(chars: &[char], from: usize, delim: &str) -> Option<usize> {
    let width = delim.chars().count();
    let mark = match delim.chars().next() {
        Some(c) => c,
        None => return None,
    };
    let mut i = from;
    while i < chars.len() {
        if starts_with(chars, i, delim) {
            // `***` is one run serving two markers. The wide one takes the tail
            // of it and leaves the head to whatever opened inside.
            let mut run = 0;
            while chars.get(i + run) == Some(&mark) {
                run += 1;
            }
            let start = if run > width { i + run - width } else { i };
            // Content on the inside, and nothing empty.
            if start > from
                && !chars[start - 1].is_whitespace()
                && !(width == 1
                    && digit_at(chars, start - 1, true)
                    && digit_at(chars, start + 1, true))
            {
                return Some(start);
            }
            i += run.max(width);
            continue;
        }
        i += 1;
    }
    None
}

fn starts_with(chars: &[char], i: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(offset, expected)| chars.get(i + offset) == Some(&expected))
}

fn digit_at(chars: &[char], index: usize, valid: bool) -> bool {
    valid && chars.get(index).map_or(false, |c| c.is_ascii_digit())
}

fn escape(chars: &[char], out: &mut String) {
    for c in chars {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("<br>"),
            other => out.push(*other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::to_formatted_body;

    #[test]
    fn plain_text_stays_plain() {
        assert_eq!(to_formatted_body("just a message"), None);
        assert_eq!(to_formatted_body("two\nlines"), None);
    }

    #[test]
    fn the_four_wrappers() {
        assert_eq!(
            to_formatted_body("**bold**").as_deref(),
            Some("<strong>bold</strong>")
        );
        assert_eq!(to_formatted_body("*slanted*").as_deref(), Some("<em>slanted</em>"));
        assert_eq!(to_formatted_body("~~gone~~").as_deref(), Some("<del>gone</del>"));
        assert_eq!(to_formatted_body("++under++").as_deref(), Some("<u>under</u>"));
    }

    #[test]
    fn code_is_quoted_as_it_stands() {
        assert_eq!(
            to_formatted_body("`**not bold**`").as_deref(),
            Some("<code>**not bold**</code>")
        );
    }

    #[test]
    fn markup_in_the_text_is_escaped() {
        assert_eq!(
            to_formatted_body("**<script>**").as_deref(),
            Some("<strong>&lt;script&gt;</strong>")
        );
        // Nothing to format, so nothing formatted - the plain body carries it.
        assert_eq!(to_formatted_body("<b>hello</b>"), None);
    }

    #[test]
    fn nesting_works() {
        assert_eq!(
            to_formatted_body("**bold *and slanted***").as_deref(),
            Some("<strong>bold <em>and slanted</em></strong>")
        );
    }

    #[test]
    fn arithmetic_is_not_emphasis() {
        assert_eq!(to_formatted_body("2*3*4"), None);
        assert_eq!(to_formatted_body("5 * 3 * 2"), None);
    }

    #[test]
    fn a_lonely_marker_is_text() {
        assert_eq!(to_formatted_body("*unclosed"), None);
        assert_eq!(to_formatted_body("a ** b"), None);
        assert_eq!(to_formatted_body("**"), None);
    }

    #[test]
    fn inside_a_word_still_counts() {
        assert_eq!(
            to_formatted_body("foo**bar**baz").as_deref(),
            Some("foo<strong>bar</strong>baz")
        );
    }

    #[test]
    fn line_breaks_travel_as_tags() {
        assert_eq!(
            to_formatted_body("**a**\nb").as_deref(),
            Some("<strong>a</strong><br>b")
        );
    }
}
