// SPDX-License-Identifier: AGPL-3.0-only
//! The text of a Rich Text Format document, for the Preview pane (VIEW-070).
//!
//! Windows' Preview pane shows an RTF file's text, not its markup. This
//! reads the text the way an RTF reader does, without its formatting:
//! paragraphs and line breaks become new lines, `\tab` a tab, `\'hh`
//! escapes a character of the document's ANSI code page (`\ansicpgN`,
//! Windows-1252 unless it names another the system converts, one byte
//! per character), `\uN` a Unicode character (two of
//! them a surrogate pair's character beyond the Basic Multilingual Plane,
//! skipping the `\ucN` fallback characters after each), and the groups a reader
//! does not show (the font, color and style tables, document information,
//! pictures, fields' instructions and every `{\*…}` destination) are
//! left out. Bold, italics, fonts and colors are not drawn: the pane shows
//! the text in its usual font.

/// The destinations whose text is never shown.
const HIDDEN_DESTINATIONS: [&str; 16] = [
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "fldinst",
    "header",
    "footer",
    "headerl",
    "headerr",
    "footerl",
    "footerr",
    "listtable",
    "listoverridetable",
    "themedata",
];

/// The characters 0x80 to 0x9F of Windows-1252, the code page of almost
/// every RTF file (`\ansi`, `\ansicpg1252`); the other bytes are Latin-1.
const WINDOWS_1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}',
    '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

/// The deepest group nesting followed; deeper groups are read as text of
/// their parent, so a hostile file cannot grow the stack without end.
const MAX_DEPTH: usize = 256;

/// The state of one group: whether its text is hidden, and how many
/// fallback characters follow a `\uN`.
#[derive(Debug, Clone, Copy)]
struct Group {
    hidden: bool,
    fallback: usize,
}

/// The characters 0x80 to 0xFF of the single-byte ANSI code page
/// `code_page` (`\ansicpgN`), through the system's converters; `None`
/// when they lack it. A byte the code page does not map as one character
/// is read as Windows-1252.
fn code_page_characters(code_page: i64) -> Option<Vec<char>> {
    let codeset = format!("CP{code_page}");
    gtk::glib::IConv::new("UTF-8", codeset.as_str())?;
    let characters = (0x80..=0xff_u8)
        .map(|byte| {
            gtk::glib::convert(&[byte], "UTF-8", codeset.as_str())
                .ok()
                .and_then(|(bytes, _)| {
                    let mut characters = std::str::from_utf8(&bytes).ok()?.chars();
                    characters.next().filter(|_| characters.next().is_none())
                })
                .unwrap_or_else(|| windows_1252(byte))
        })
        .collect();
    Some(characters)
}

/// The ANSI code page an RTF header names (`\ansicpgN`).
#[derive(Debug)]
enum CodePage {
    /// None named yet: raw bytes above 127 may be UTF-8.
    Unnamed,
    /// One the system's converters lack, read as [`CodePage::Unnamed`].
    Unreadable,
    /// Windows-1252.
    Windows1252,
    /// Another single-byte code page: its characters 0x80 to 0xFF.
    Other(Vec<char>),
}

/// The character of the Windows-1252 byte `byte`.
fn windows_1252(byte: u8) -> char {
    match byte {
        0x80..=0x9f => WINDOWS_1252_HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

/// The character a control word stands for, if it is one.
fn word_character(word: &str) -> Option<char> {
    Some(match word {
        "par" | "line" | "sect" | "page" | "row" => '\n',
        "tab" | "cell" => '\t',
        "emdash" => '—',
        "endash" => '–',
        "lquote" => '‘',
        "rquote" => '’',
        "ldblquote" => '“',
        "rdblquote" => '”',
        "bullet" => '•',
        _ => return None,
    })
}

/// Whether `bytes` start as an RTF document does.
pub(super) fn is_rtf(bytes: &[u8]) -> bool {
    bytes.trim_ascii_start().starts_with(b"{\\rtf")
}

/// The text of the RTF document `bytes`, without its markup.
pub(super) fn rtf_text(bytes: &[u8]) -> String {
    let mut reader = Reader {
        bytes,
        index: 0,
        text: String::new(),
        stack: Vec::new(),
        untracked: 0,
        code_page: CodePage::Unnamed,
        group: Group {
            hidden: false,
            fallback: 1,
        },
        skip: 0,
        starred: false,
        group_start: false,
        high_surrogate: None,
    };
    reader.read();
    reader.end_surrogate();
    reader.text
}

/// Reads an RTF document's text.
struct Reader<'a> {
    bytes: &'a [u8],
    index: usize,
    text: String,
    stack: Vec<Group>,
    /// Groups opened past [`MAX_DEPTH`] and not closed yet: their `}`
    /// close no group of the stack.
    untracked: usize,
    /// The ANSI code page the header named.
    code_page: CodePage,
    group: Group,
    /// Fallback characters still to skip after a `\uN`.
    skip: usize,
    /// A `{\*` group started: its destination is hidden.
    starred: bool,
    /// Just after `{`: the first control word may name a destination.
    group_start: bool,
    /// The first half of a UTF-16 surrogate pair a `\uN` gave, waiting
    /// for the second, which the next `\uN` gives: a character beyond the
    /// Basic Multilingual Plane, such as an emoji.
    high_surrogate: Option<u32>,
}

impl Reader<'_> {
    /// The character of the byte `byte` in the document's ANSI code page.
    fn ansi(&self, byte: u8) -> char {
        match &self.code_page {
            CodePage::Other(characters) if byte >= 0x80 => characters[usize::from(byte - 0x80)],
            _ => windows_1252(byte),
        }
    }

    /// Whether the header named a code page this reader can read: raw
    /// bytes above 127 are its characters, never guessed as UTF-8.
    fn code_page_named(&self) -> bool {
        matches!(self.code_page, CodePage::Windows1252 | CodePage::Other(_))
    }

    /// Adds `character` to the text unless it is a fallback to skip or
    /// its group is hidden.
    fn push(&mut self, character: char) {
        if self.skip > 0 {
            self.skip -= 1;
        } else {
            self.push_always(character);
        }
    }

    /// Adds `character`, never a fallback, unless its group is hidden.
    fn push_always(&mut self, character: char) {
        self.end_surrogate();
        if !self.group.hidden {
            self.text.push(character);
        }
    }

    /// Ends a surrogate pair that has only its first half: a replacement
    /// character stands for it.
    fn end_surrogate(&mut self) {
        if self.high_surrogate.take().is_some() && !self.group.hidden {
            self.text.push('\u{fffd}');
        }
    }

    /// Reads the UTF-16 code unit of a `\uN`, `parameter` (negative above
    /// 32767): a character of its own, or half of a surrogate pair, whose
    /// halves make one character beyond the Basic Multilingual Plane. A
    /// half without its other half is a replacement character.
    fn unicode_unit(&mut self, parameter: Option<i64>) {
        let unit = parameter
            .map(|n| if n < 0 { n + 65_536 } else { n })
            .and_then(|n| u32::try_from(n).ok());
        match unit {
            Some(high @ 0xd800..=0xdbff) => {
                self.end_surrogate();
                self.high_surrogate = Some(high);
            }
            Some(low @ 0xdc00..=0xdfff) => {
                let character = self
                    .high_surrogate
                    .take()
                    .and_then(|high| char::from_u32(0x1_0000 + ((high - 0xd800) << 10) + (low - 0xdc00)))
                    .unwrap_or('\u{fffd}');
                self.push_always(character);
            }
            other => {
                let character = other.and_then(char::from_u32).unwrap_or('\u{fffd}');
                self.push_always(character);
            }
        }
    }

    fn read(&mut self) {
        while let Some(&byte) = self.bytes.get(self.index) {
            self.index += 1;
            match byte {
                b'{' => {
                    if self.stack.len() < MAX_DEPTH {
                        self.stack.push(self.group);
                    } else {
                        self.untracked += 1;
                    }
                    self.group_start = true;
                    self.skip = 0;
                    continue;
                }
                b'}' => {
                    if self.untracked > 0 {
                        self.untracked -= 1;
                    } else if let Some(outer) = self.stack.pop() {
                        self.group = outer;
                    }
                    self.skip = 0;
                }
                b'\\' => {
                    if self.control() {
                        continue;
                    }
                }
                // Line ends in the file are not text.
                b'\n' | b'\r' => {}
                _ => self.character(byte),
            }
            self.group_start = false;
            self.starred = false;
        }
    }

    /// Reads a control word or symbol after its backslash; true when the
    /// group's start or star still holds for what follows.
    fn control(&mut self) -> bool {
        let Some(&next) = self.bytes.get(self.index) else {
            return false;
        };
        if next.is_ascii_alphabetic() {
            self.control_word();
            return true;
        }
        self.index += 1;
        match next {
            b'*' => {
                self.starred = true;
                return true;
            }
            b'\'' => {
                let hex = self.bytes.get(self.index..self.index + 2);
                let value = hex
                    .and_then(|hex| std::str::from_utf8(hex).ok())
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                if let Some(value) = value {
                    self.index += 2;
                    self.push(self.ansi(value));
                }
            }
            b'~' => self.push('\u{a0}'),
            b'_' => self.push('\u{2011}'),
            b'\n' | b'\r' => self.push_always('\n'),
            b'\\' | b'{' | b'}' => self.push(char::from(next)),
            _ => {}
        }
        false
    }

    /// Reads a control word, its number and the space that ends it.
    fn control_word(&mut self) {
        let bytes = self.bytes;
        let start = self.index;
        while bytes.get(self.index).is_some_and(u8::is_ascii_alphabetic) {
            self.index += 1;
        }
        let word = std::str::from_utf8(&bytes[start..self.index]).unwrap_or_default();
        let number_start = self.index;
        if bytes.get(self.index) == Some(&b'-') {
            self.index += 1;
        }
        while bytes.get(self.index).is_some_and(u8::is_ascii_digit) {
            self.index += 1;
        }
        let parameter = std::str::from_utf8(&bytes[number_start..self.index])
            .ok()
            .and_then(|number| number.parse::<i64>().ok());
        // One space ends a control word and is part of it.
        if bytes.get(self.index) == Some(&b' ') {
            self.index += 1;
        }
        if (self.group_start && HIDDEN_DESTINATIONS.contains(&word)) || self.starred {
            self.group.hidden = true;
        }
        self.starred = false;
        self.group_start = false;
        match word {
            "par" | "line" | "sect" | "page" | "row" | "tab" | "cell" => {
                self.push_always(word_character(word).unwrap_or('\n'));
            }
            // Only the header's: a file of them cannot keep the converters
            // busy.
            "ansicpg" if matches!(self.code_page, CodePage::Unnamed) => {
                self.code_page = match parameter {
                    Some(1252) => CodePage::Windows1252,
                    Some(code_page) => {
                        code_page_characters(code_page).map_or(CodePage::Unreadable, CodePage::Other)
                    }
                    None => CodePage::Unreadable,
                };
            }
            "uc" => self.group.fallback = parameter.and_then(|n| usize::try_from(n).ok()).unwrap_or(1),
            "u" => {
                self.unicode_unit(parameter);
                self.skip = self.group.fallback;
            }
            "bin" => {
                // Binary data of that many bytes follows.
                let length = parameter.and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
                self.index = self.index.saturating_add(length).min(bytes.len());
            }
            _ => {
                if let Some(character) = word_character(word) {
                    self.push(character);
                }
            }
        }
    }

    /// Reads a character of text: ASCII, a character of the code page the
    /// header named, or, without one, UTF-8 a hand-written file may hold
    /// (bytes above 127 are not RTF), else Windows-1252.
    fn character(&mut self, byte: u8) {
        let start = self.index - 1;
        let character = if byte.is_ascii() {
            char::from(byte)
        } else if self.code_page_named() {
            self.ansi(byte)
        } else {
            let end = (start + 4).min(self.bytes.len());
            let decoded = (start + 2..=end)
                .find_map(|end| std::str::from_utf8(&self.bytes[start..end]).ok())
                .and_then(|text| text.chars().next());
            match decoded {
                Some(character) => {
                    self.index = start + character.len_utf8();
                    character
                }
                None => self.ansi(byte),
            }
        };
        self.push(character);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-070
    #[test]
    fn rtf_markup_becomes_its_text() {
        let letter = b"{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0\\fswiss Calibri;}}\
            {\\colortbl;\\red255\\green0\\blue0;}{\\*\\generator Example;}\
            {\\info{\\title Secret title}}\\f0\\fs22 Dear \\b Sam\\b0 ,\\par\n\
            Caf\\'e9 \\'93costs\\'94 5\\u8364?.\\line Two\\tab cols\\par\
            {\\field{\\*\\fldinst HYPERLINK \"x\"}{\\fldrslt link}}\\par}";
        assert_eq!(
            rtf_text(letter),
            "Dear Sam,\nCafé \u{201c}costs\u{201d} 5€.\nTwo\tcols\nlink\n"
        );
    }

    #[test]
    fn unicode_fallbacks_and_escapes_are_read_as_characters() {
        assert_eq!(
            rtf_text(b"{\\rtf1\\uc2\\u-3913 ab c\\{\\}\\\\}"),
            "\u{f0b7} c{}\\"
        );
        assert_eq!(rtf_text("{\\rtf1 Gr\u{fc}\u{df}e}".as_bytes()), "Grüße");
        assert_eq!(rtf_text(b"{\\rtf1 x\\bin3 {}}y}"), "xy");
    }

    /// A character beyond the Basic Multilingual Plane, such as an emoji
    /// or a supplementary CJK character, is stored as a UTF-16 surrogate
    /// pair in two signed `\\uN` controls and is read as that one
    /// character; a lone half is a replacement character.
    ///
    /// parity: VIEW-070
    #[test]
    fn surrogate_pairs_are_read_as_one_character() {
        assert_eq!(
            rtf_text(b"{\\rtf1\\uc1\\u-10179?\\u-8704? done}"),
            "\u{1f600} done"
        );
        assert_eq!(rtf_text(b"{\\rtf1\\uc0\\u-10176\\u-9191 }"), "\u{20019}");
        assert_eq!(rtf_text(b"{\\rtf1\\uc1\\u-10179? x}"), "\u{fffd} x");
        assert_eq!(rtf_text(b"{\\rtf1\\uc1\\u-8704? y}"), "\u{fffd} y");
    }

    /// `\'hh` escapes and raw bytes above 127 are characters of the ANSI
    /// code page the header names, such as Windows-1251 for Cyrillic.
    ///
    /// parity: VIEW-070
    #[test]
    fn escaped_bytes_are_read_in_the_documents_code_page() {
        // Only where the system can convert Windows-1251.
        if gtk::glib::convert(&[0xcf], "UTF-8", "CP1251").is_err() {
            return;
        }
        assert_eq!(
            rtf_text(b"{\\rtf1\\ansi\\ansicpg1251\\deff0 \\'cf\\'f0\\'e8\\'e2\\'e5\\'f2 \xe4\xe0}"),
            "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{434}\u{430}"
        );
        assert_eq!(rtf_text(b"{\\rtf1\\ansi\\ansicpg1252 Caf\\'e9}"), "Caf\u{e9}");
        // Raw bytes that would also be UTF-8 are still the code page's.
        assert_eq!(rtf_text(b"{\\rtf1\\ansi\\ansicpg1251 \xc2\xa9}"), "\u{412}\u{a9}");
    }

    /// Groups nested past the deepest followed still close their own `}`,
    /// so the text after them is read in the right group.
    #[test]
    fn groups_past_the_deepest_followed_close_their_own_braces() {
        let mut document = b"{\\rtf1{\\*\\hidden ".to_vec();
        document.extend(std::iter::repeat_n(b'{', MAX_DEPTH + 4));
        document.extend(std::iter::repeat_n(b'}', MAX_DEPTH + 4));
        document.extend(b" still hidden} shown}");
        assert_eq!(rtf_text(&document), " shown");
    }

    #[test]
    fn a_new_rich_text_document_has_one_empty_paragraph() {
        let new = ox_core::ops::NewDocument::RichTextDocument.contents();
        assert!(is_rtf(new));
        assert_eq!(rtf_text(new), "\n");
        assert!(!is_rtf(b"plain text"));
    }

    /// Seeded mutations of RTF documents, and deep or endless groups, are
    /// read to text without a panic and in bounded time, as is any text
    /// the Preview pane decodes.
    #[test]
    fn mutated_rtf_and_text_are_read_without_a_panic() {
        let samples: [&[u8]; 4] = [
            br"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0 Arial;}}\f0 Hello \'e9t\'e9 \u8364? \par World}",
            br"{\rtf1{\*\generator x}\uc2\u55357\'3f\'3f\u56832\'3f\'3f text\bin5 abcde}",
            br"{\rtf1\ansicpg932 \'82\'a0\'82 {\field{\*\fldinst HYPERLINK}{\fldrslt link}}}",
            br"{\rtf1 \-\~\_\{\}\\\tab\line\u-10000 \u99999999999999999999 end",
        ];
        let mut seed = 0xC0FF_EE11_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            usize::try_from(seed).unwrap_or(0)
        };
        for sample in samples {
            // `OX_FUZZ_ROUNDS` raises the rounds for a longer local run.
            let rounds = std::env::var("OX_FUZZ_ROUNDS")
                .ok()
                .and_then(|rounds| rounds.parse().ok());
            for _ in 0..rounds.unwrap_or(3000) {
                let mut bytes = sample.to_vec();
                for _ in 0..=(next() % 6) {
                    let at = next() % (bytes.len() + 1);
                    let byte = b"{}\\'u0123456789-? abz\xff\x00"[next() % 22];
                    if next() % 3 == 0 && at < bytes.len() {
                        bytes[at] = byte;
                    } else {
                        bytes.insert(at, byte);
                    }
                }
                let _ = rtf_text(&bytes);
                let _ = crate::window::details_pane::preview_page::decode_text(&bytes, next() % 2 == 0);
            }
        }
        let started = std::time::Instant::now();
        let mut deep = b"{\\rtf1 ".to_vec();
        deep.extend(std::iter::repeat_n(b'{', 1_000_000));
        let _ = rtf_text(&deep);
        let mut words = b"{\\rtf1 ".to_vec();
        words.extend(b"\\u65535".iter().copied().cycle().take(1_000_000));
        let _ = rtf_text(&words);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        for length in 0..512 {
            let bytes: Vec<u8> = (0..length)
                .map(|_| u8::try_from(next() % 256).unwrap_or(0))
                .collect();
            let _ = crate::window::details_pane::preview_page::decode_text(&bytes, length % 2 == 0);
        }
    }
}
