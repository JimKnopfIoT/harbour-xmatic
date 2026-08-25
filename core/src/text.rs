//! Text that came from someone else, made safe to look at.
//!
//! Not safe to *parse* - nothing here is a sanitiser for markup; that is
//! `markup.rs`, which writes its own output. This is about what the eye can be
//! made to believe.

/// Removes the Unicode characters that change the direction text is drawn in.
///
/// They are invisible and they reorder what follows them, so a display name, a
/// room name or a file name can be made to read as something else entirely -
/// the classic being a file that shows as `holiday.jpg` while it ends in
/// `.exe`. Every one of them is a formatting instruction, never content: a
/// message that means to be right-to-left is written in Arabic or Hebrew
/// letters, and those carry their direction themselves.
///
/// Removed rather than replaced: a marker in the middle of a name would be
/// noise in every legitimate Arabic or Hebrew name, and there are many.
///
/// * U+061C  arabic letter mark
/// * U+200E, U+200F  left-to-right / right-to-left mark
/// * U+202A…U+202E  embeddings and overrides
/// * U+2066…U+2069  isolates
pub fn strip_bidi(text: &str) -> String {
    if !text.chars().any(is_bidi_control) {
        // The overwhelming case: hand back what came in, without allocating a
        // second copy of every message body in the room.
        return text.to_owned();
    }
    text.chars().filter(|character| !is_bidi_control(*character)).collect()
}

pub fn is_bidi_control(character: char) -> bool {
    matches!(character,
        '\u{061C}' | '\u{200E}' | '\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2066}'..='\u{2069}')
}

/// A file name that is safe to write into a folder, from a name that came from
/// somebody else.
///
/// The name of an attachment is a **field in the event**, not a name any
/// filesystem ever accepted: the sending client writes it, so it can hold
/// slashes, `..`, newlines or nothing at all. Everything up to the last
/// separator goes, the leftovers that are not a name go with it, and the rest
/// is trimmed of what should never appear in a directory listing.
///
/// The Qt side strips the directory part a second time before writing
/// (`QFileInfo::fileName`). Two lines of defence on purpose: this one can be
/// tested, that one is the last word.
pub fn safe_file_name(name: &str) -> String {
    let name = strip_bidi(name);
    // Both separators: a name is not necessarily written on this system.
    let base = name
        .rsplit(|character| character == '/' || character == '\\')
        .next()
        .unwrap_or_default();
    let base: String = base
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let base = base.trim();
    if base.is_empty() || base == "." || base == ".." {
        return String::new();
    }
    // Long enough for any real name, short enough not to fight the filesystem.
    const MAX: usize = 120;
    if base.chars().count() <= MAX {
        return base.to_owned();
    }
    base.chars().take(MAX).collect()
}

/// Blanks anything in a message that could identify somebody or something.
///
/// Applied at the sinks - `reply_error` and the events that carry an error -
/// not at the places that build the text: an SDK error carries the request URL
/// (`reqwest` appends "for url (…)"), and a Matrix URL carries the room and
/// the user in its path.
pub fn scrub_ids(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            // Punctuation the sentence brought, including the colon a DNS
            // error ends its host with. What is left is the candidate.
            let trimmed = word.trim_matches(|c: char| "(),;:.\"'[]{}<>".contains(c));
            if trimmed.is_empty() {
                return word.to_owned();
            }

            let url = trimmed.contains("://");
            // A sigil anywhere in the word. The server part is not required:
            // `!room` and `@alice` name somebody just as well.
            let matrix_id = trimmed
                .chars()
                .any(|c| matches!(c, '@' | '!' | '#' | '$' | '+'))
                && trimmed.len() >= 3;
            let path = trimmed.matches('/').count() >= 2;

            // `host`, `host:port`, or an address. Split the port off first -
            // a homeserver is addressed with one more often than without.
            let host_part = trimmed.split(':').next().unwrap_or(trimmed);
            let labels: Vec<&str> = host_part.split('.').collect();
            let dotted = labels.len() >= 2
                && labels.iter().all(|label| {
                    !label.is_empty()
                        && label
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                });
            // A name (last label is letters) or an address (all digits).
            let hostname = dotted
                && labels
                    .last()
                    .map(|last| {
                        (last.len() >= 2 && last.chars().all(|c| c.is_ascii_alphabetic()))
                            || last.chars().all(|c| c.is_ascii_digit())
                    })
                    .unwrap_or(false);

            // `M_LIMIT_EXCEEDED` and its relatives: upper case, underscores,
            // no identifier in them, and the one token worth keeping when a
            // user reports a failure.
            let error_code = trimmed.len() >= 3
                && trimmed
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                && trimmed.contains('_');

            let token = !error_code
                && trimmed.len() > 12
                && trimmed
                    .split_once('_')
                    .map(|(prefix, rest)| {
                        prefix.len() <= 4
                            && !prefix.is_empty()
                            && prefix.chars().all(|c| c.is_ascii_alphabetic())
                            && rest.len() >= 8
                            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    })
                    .unwrap_or(false);
            let blob = !error_code
                && trimmed.len() >= 32
                && trimmed.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_' | '.')
                });

            if url || matrix_id || path || hostname || token || blob {
                "<id>".to_owned()
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scrubber_blanks_every_class_of_identifier() {
        assert_eq!(scrub_ids("failed for url (https://server/_matrix/x)"),
                   "failed for url <id>");
        assert_eq!(scrub_ids("no room !abcdefgh:server.tld here"), "no room <id> here");
        assert_eq!(scrub_ids("user @name:server.tld left"), "user <id> left");
        assert_eq!(scrub_ids("event $abcdef:server.tld gone"), "event <id> gone");
        assert_eq!(scrub_ids("token syt_YWxpY2U_abcdefgh_1234"), "token <id>");
        assert_eq!(scrub_ids("at /home/defaultuser/.local/share/x"), "at <id>");
        assert_eq!(scrub_ids("key AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"), "key <id>");
    }

    #[test]
    fn the_scrubber_catches_what_the_system_really_says() {
        // Verbatim shapes from std::io, hyper and reqwest - the previous test
        // asserted on a string no operating system emits, so it passed while
        // the homeserver went into the journal.
        for probe in [
            "dns error: failed to lookup address information for matrix.example.org: Name or service not known",
            "error trying to connect: tcp connect error: chat.example.org:8448: Connection refused",
            "failed to connect to 203.0.113.42:8448",
            "no route to 198.51.100.5",
            "could not reach the homeserver matrix.example.org.",
            "the room !AbCdEfGhIjKlMnOpQr was not found",
            "user @alice was not found",
        ] {
            let scrubbed = scrub_ids(probe);
            assert!(scrubbed.contains("<id>"), "not scrubbed: {probe} -> {scrubbed}");
            assert!(!scrubbed.contains("example.org"), "host survived: {scrubbed}");
            assert!(!scrubbed.contains("203.0.113"), "address survived: {scrubbed}");
            assert!(!scrubbed.contains("alice"), "user survived: {scrubbed}");
        }
    }

    #[test]
    fn the_scrubber_catches_what_the_second_hearing_smuggled_past_it() {
        for probe in [
            "dns error: failed to lookup address information for matrix.example.org",
            "the homeserver chat.company.internal refused the connection",
            "server returned {\"room_id\":\"!SecretRoom:example.org\"}",
            "invited by [@alice:example.org] to the room",
            "sender=@bob:example.org could not be reached",
            "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N",
            "at home/defaultuser/.local/share/org.xmatic/xmatic/session.json",
        ] {
            let scrubbed = scrub_ids(probe);
            assert!(scrubbed.contains("<id>"), "not scrubbed: {probe} -> {scrubbed}");
        }
    }

    #[test]
    fn the_scrubber_keeps_what_makes_a_report_useful() {
        // Two failures of one class must not read differently because one
        // word happened to be longer than the other.
        for probe in ["M_LIMIT_EXCEEDED", "M_UNKNOWN_TOKEN", "M_FORBIDDEN", "M_NOT_FOUND"] {
            assert_eq!(scrub_ids(probe), probe);
        }
        assert_eq!(scrub_ids("M_LIMIT_EXCEEDED, retrying"), "M_LIMIT_EXCEEDED, retrying");
    }

    #[test]
    fn the_scrubber_leaves_ordinary_words_alone() {
        assert_eq!(scrub_ids("the server said 429, try again"),
                   "the server said 429, try again");
        assert_eq!(scrub_ids("could not read the devices"), "could not read the devices");
    }

    #[test]
    fn a_file_name_cannot_pretend_to_end_differently() {
        // "photo_gpj.exe" written with an override reads as "photo_exe.jpg".
        let disguised = "photo_\u{202E}gpj.exe";
        assert_eq!(strip_bidi(disguised), "photo_gpj.exe");
    }

    #[test]
    fn ordinary_text_is_returned_unchanged() {
        for text in ["hello", "Grüße", "مرحبا", "שלום", "こんにちは", ""] {
            assert_eq!(strip_bidi(text), text);
        }
    }

    #[test]
    fn a_sender_cannot_name_a_file_out_of_its_folder() {
        assert_eq!(safe_file_name("../../.ssh/authorized_keys"), "authorized_keys");
        assert_eq!(safe_file_name("/etc/passwd"), "passwd");
        assert_eq!(safe_file_name("..\\..\\windows\\system32\\evil.dll"), "evil.dll");
        assert_eq!(safe_file_name("holiday.jpg"), "holiday.jpg");
    }

    #[test]
    fn a_name_that_is_no_name_becomes_none() {
        for name in ["", "..", ".", "   ", "some/path/", "\n\t"] {
            assert_eq!(safe_file_name(name), "", "{name:?}");
        }
    }

    #[test]
    fn a_name_cannot_carry_control_characters_or_a_reversal() {
        assert_eq!(safe_file_name("re\u{202E}port.pdf"), "report.pdf");
        assert_eq!(safe_file_name("two\nlines.txt"), "twolines.txt");
        assert_eq!(safe_file_name(&"a".repeat(400)).chars().count(), 120);
    }

    #[test]
    fn every_control_goes_including_the_isolates() {
        let noisy = "a\u{061C}b\u{200E}c\u{200F}d\u{202A}e\u{202B}f\u{202C}g\u{202D}h\u{202E}i\u{2066}j\u{2067}k\u{2068}l\u{2069}m";
        assert_eq!(strip_bidi(noisy), "abcdefghijklm");
    }
}
