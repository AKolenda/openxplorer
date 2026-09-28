// SPDX-License-Identifier: AGPL-3.0-only
//! Member names as Python's `zipfile` decodes them.
//!
//! A name flagged as UTF-8 (general purpose bit 11) must be valid UTF-8;
//! any other name is IBM code page 437, the historical ZIP encoding. The
//! name is then cut at its first NUL character (`_sanitize_filename`),
//! while the uncut name is kept to detect exactly that trick.

use super::error::ZipFormatError;

/// General purpose flag bit 11: the name and comment are UTF-8.
pub(super) const UTF8_NAME_FLAG: u16 = 1 << 11;

/// Code page 437 characters for the bytes 0x80 to 0xFF. The bytes 0x00 to
/// 0x7F are ASCII, control characters included, as in Python's `cp437`
/// codec.
const CP437_HIGH_HALF: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', //
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', //
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', //
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', //
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', //
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', //
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', //
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

/// Decodes a raw member name: UTF-8 when `flags` say so, code page 437
/// otherwise.
///
/// # Errors
///
/// [`ZipFormatError::NameNotUtf8`] for a name flagged as UTF-8 that is not.
pub(super) fn decode_name(raw: &[u8], flags: u16) -> Result<String, ZipFormatError> {
    if flags & UTF8_NAME_FLAG == 0 {
        return Ok(raw.iter().copied().map(cp437_char).collect());
    }
    match std::str::from_utf8(raw) {
        Ok(name) => Ok(name.to_owned()),
        Err(_) => Err(ZipFormatError::NameNotUtf8),
    }
}

/// The name up to its first NUL character. Python's `zipfile` does this
/// for every member (`ZipInfo.filename`); a name that changes here was
/// built to show one name and extract another.
pub(super) fn cut_at_nul(name: &str) -> String {
    match name.split_once('\0') {
        Some((visible, _hidden)) => visible.to_owned(),
        None => name.to_owned(),
    }
}

/// The character code page 437 assigns to `byte`.
fn cp437_char(byte: u8) -> char {
    if byte.is_ascii() {
        char::from(byte)
    } else {
        CP437_HIGH_HALF[usize::from(byte - 0x80)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_without_the_utf8_flag_are_code_page_437() {
        let decoded = decode_name(b"Caf\x82 \x9c\xff/r\xe9sum\x82.txt", 0).expect("cp437 decodes any byte");

        assert_eq!(decoded, "Café £\u{a0}/rΘsumé.txt");
    }

    #[test]
    fn flagged_names_must_be_valid_utf8() {
        assert_eq!(
            decode_name("Café.txt".as_bytes(), UTF8_NAME_FLAG),
            Ok("Café.txt".to_owned())
        );
        assert_eq!(
            decode_name(b"Caf\xe9.txt", UTF8_NAME_FLAG),
            Err(ZipFormatError::NameNotUtf8)
        );
    }

    #[test]
    fn names_are_cut_at_their_first_nul() {
        assert_eq!(cut_at_nul("Docs/\0suffix\0more"), "Docs/");
        assert_eq!(cut_at_nul("plain.txt"), "plain.txt");
    }
}
