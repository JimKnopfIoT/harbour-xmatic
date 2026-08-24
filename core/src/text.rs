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

#[cfg(test)]
mod tests {
    use super::*;

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
