// SPDX-License-Identifier: AGPL-3.0-only
//! What a name must hold to match a search: every word, ignoring case,
//! where a word with wildcards is a pattern found within the name.
//!
//! Ports the filter of `filtered()` in `v2.0.0:desktop/ui/app.js` (every
//! whitespace-separated word occurs in the name) and adds the wildcards of
//! Dolphin's filter bar (`KFileItemModelFilter::setPattern`, SRCH-004): a
//! word holding `*`, `?` or `[` is a pattern. `*` stands for any text,
//! `?` for one character, and `[a-c]` or `[!a-c]` for one character in
//! or outside a set; a `[` without its `]` is a plain character.
//!
//! As in Windows 11 File Explorer (checked on Windows 11 on 2026-11-14), a
//! pattern is found within the name, as a plain word is, rather than
//! having to match the whole name: `a?b` finds `a1b.txt` and `acb.txt`,
//! `*.pdf` finds PDF files and `report-202?` the reports of a decade.
//!
//! A plain word or a pattern must be found starting before the name's
//! extension, or be the whole extension: `pd - m5753` finds `PD -
//! M5753RA.pdf` but not `WO - M5753.pdf`, whose only `pd` is in `.pdf`,
//! while `pdf`, `.pdf`, `p?f` and `66873.pdf` still find PDF files.

/// The characters that make a word a wildcard pattern.
const WILDCARDS: [char; 3] = ['*', '?', '['];

/// Whether `word` is a wildcard pattern rather than text to find.
pub fn is_wildcard(word: &str) -> bool {
    word.contains(WILDCARDS)
}

/// The words of a search, lower-cased.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamePattern {
    words: Vec<String>,
}

impl NamePattern {
    /// The pattern of the search text `text`: its whitespace-separated
    /// words, lower-cased, each once.
    pub fn new(text: &str) -> Self {
        let mut words: Vec<String> = Vec::new();
        for word in text.to_lowercase().split_whitespace() {
            if !words.iter().any(|known| known == word) {
                words.push(word.to_owned());
            }
        }
        Self { words }
    }

    /// Whether the search has no words, so it matches everything.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Whether `name` matches, ignoring case.
    pub fn matches(&self, name: &str) -> bool {
        self.matches_lowercase(&name.to_lowercase(), "")
    }

    /// Whether `lowercase_name`, a lower-cased name, matches: each
    /// wildcard word is found within the name ([`wildcard_in_name`]), and
    /// each other word occurs in the name or in `lowercase_context`, such
    /// as the folder that holds the item (SRCH-008), which may be empty.
    pub fn matches_lowercase(&self, lowercase_name: &str, lowercase_context: &str) -> bool {
        self.words.iter().all(|word| {
            if is_wildcard(word) {
                wildcard_in_name(word, lowercase_name)
            } else {
                word_in_name(word, lowercase_name) || lowercase_context.contains(word.as_str())
            }
        })
    }
}

/// The longest extension: `.docx`, `.jpeg` and `.tar` count, while the
/// end of `Notes v2.final draft` does not.
const MAX_EXTENSION_CHARS: usize = 8;

/// Where the extension of `lowercase_name` starts, after its last dot:
/// up to [`MAX_EXTENSION_CHARS`] characters holding a letter and no
/// space. `None` when the name has none, as `.bashrc` and `v1.2` do not.
fn extension_start(lowercase_name: &str) -> Option<usize> {
    let dot = lowercase_name.rfind('.').filter(|&dot| dot > 0)?;
    let extension = &lowercase_name[dot + 1..];
    let is_extension = (1..=MAX_EXTENSION_CHARS).contains(&extension.chars().count())
        && extension.chars().any(char::is_alphabetic)
        && !extension.chars().any(char::is_whitespace);
    is_extension.then_some(dot + 1)
}

/// Whether the plain `word` is in `lowercase_name`, starting before its
/// extension, or is the whole extension.
pub(crate) fn word_in_name(word: &str, lowercase_name: &str) -> bool {
    let Some(first) = lowercase_name.find(word) else {
        return false;
    };
    match extension_start(lowercase_name) {
        Some(extension) => first < extension || lowercase_name[extension..] == *word,
        None => true,
    }
}

/// Whether the wildcard `pattern` is found within `lowercase_name`, as a
/// plain word is ([`word_in_name`]): the text it matches starts before
/// the name's extension, or is the whole extension.
///
/// The search cache leaves wildcard words out of its trigram prefilter, so
/// a search may test every cached name here: one pass over the name, in
/// time proportional to the name's length times the pattern's, with a
/// fixed number of allocations.
pub(crate) fn wildcard_in_name(pattern: &str, lowercase_name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let tokens = tokens(&pattern);
    let extension = extension_start(lowercase_name);
    let mut states = States::new(tokens.len());
    // Every position before the extension may start a match, which may
    // run on into the extension; reaching the pattern's end at any point
    // means the pattern matches the text from that start.
    for (at, character) in lowercase_name.char_indices() {
        if extension.is_none_or(|extension| at < extension) {
            states.start(&tokens);
        }
        if states.is_complete() {
            return true;
        }
        states.step(&tokens, character);
    }
    if extension.is_none() {
        // The empty text at the end, which `*` matches.
        states.start(&tokens);
    }
    if states.is_complete() {
        return true;
    }
    // Inside the extension, only as the whole extension.
    extension.is_some_and(|extension| matches_tokens(&tokens, &lowercase_name[extension..]))
}

/// Whether the whole of `name` matches the wildcard `pattern`.
#[cfg(test)]
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    matches_tokens(&tokens(&pattern), name)
}

/// Whether the whole of `name` matches the pattern `tokens`.
fn matches_tokens(tokens: &[Token<'_>], name: &str) -> bool {
    let mut states = States::new(tokens.len());
    states.start(tokens);
    for character in name.chars() {
        if states.is_empty() {
            return false;
        }
        states.step(tokens, character);
    }
    states.is_complete()
}

/// One element of a wildcard pattern.
enum Token<'a> {
    /// `*`: any text, also none.
    Any,
    /// `?`: one character.
    One,
    /// `[...]`: one character in or outside a set.
    Set(CharacterSet<'a>),
    /// A character standing for itself, also a `[` without its `]`.
    Literal(char),
}

/// The elements of `pattern`, read once.
fn tokens(pattern: &[char]) -> Vec<Token<'_>> {
    let mut tokens = Vec::with_capacity(pattern.len());
    let mut at = 0;
    while let Some(&character) = pattern.get(at) {
        let (token, length) = match character {
            '*' => (Token::Any, 1),
            '?' => (Token::One, 1),
            '[' => match character_set(&pattern[at..]) {
                Some((set, length)) => (Token::Set(set), length),
                None => (Token::Literal('['), 1),
            },
            _ => (Token::Literal(character), 1),
        };
        tokens.push(token);
        at += length;
    }
    tokens
}

impl Token<'_> {
    /// Whether this element, other than `*`, matches `character`.
    fn matches(&self, character: char) -> bool {
        match self {
            Token::Any | Token::One => true,
            Token::Set(set) => set.contains(character),
            Token::Literal(literal) => *literal == character,
        }
    }
}

/// The places in a pattern that the text read so far can have reached:
/// `active[i]` when the elements before `i` matched, and `active[len]`
/// when the whole pattern did.
struct States {
    active: Vec<bool>,
    next: Vec<bool>,
}

impl States {
    fn new(token_count: usize) -> Self {
        Self {
            active: vec![false; token_count + 1],
            next: vec![false; token_count + 1],
        }
    }

    /// Starts a match at the current place in the text.
    fn start(&mut self, tokens: &[Token<'_>]) {
        self.active[0] = true;
        close(&mut self.active, tokens);
    }

    /// Whether the whole pattern matched up to the current place.
    fn is_complete(&self) -> bool {
        self.active.last().copied().unwrap_or(false)
    }

    /// Whether no match is under way.
    fn is_empty(&self) -> bool {
        !self.active.contains(&true)
    }

    /// Reads `character`: `*` takes it and stays, any other element that
    /// matches it moves on.
    fn step(&mut self, tokens: &[Token<'_>], character: char) {
        self.next.fill(false);
        for (index, token) in tokens.iter().enumerate() {
            if !self.active[index] {
                continue;
            }
            match token {
                Token::Any => self.next[index] = true,
                other if other.matches(character) => self.next[index + 1] = true,
                _ => {}
            }
        }
        close(&mut self.next, tokens);
        std::mem::swap(&mut self.active, &mut self.next);
    }
}

/// Adds to `active` the places after each reached `*`, which may match no
/// text.
fn close(active: &mut [bool], tokens: &[Token<'_>]) {
    for (index, token) in tokens.iter().enumerate() {
        if active[index] && matches!(token, Token::Any) {
            active[index + 1] = true;
        }
    }
}

/// A `[...]` set of characters.
struct CharacterSet<'a> {
    /// `[!...]` or `[^...]`: any character outside the set.
    is_negated: bool,
    /// The characters and ranges between the brackets.
    members: &'a [char],
}

impl CharacterSet<'_> {
    fn contains(&self, character: char) -> bool {
        let mut members = self.members;
        let mut found = false;
        while let Some(&first) = members.first() {
            if let [start, '-', end, ..] = members {
                found |= (*start..=*end).contains(&character);
                members = &members[3..];
            } else {
                found |= first == character;
                members = &members[1..];
            }
        }
        found != self.is_negated
    }
}

/// The set at the start of `rest`, which starts with `[`, and how many
/// characters it takes; `None` without a closing `]`. A `]` right after
/// the opening (or after `!`) belongs to the set.
fn character_set(rest: &[char]) -> Option<(CharacterSet<'_>, usize)> {
    let is_negated = matches!(rest.get(1), Some('!' | '^'));
    let first = if is_negated { 2 } else { 1 };
    let closing = rest
        .iter()
        .enumerate()
        .skip(first + 1)
        .find(|(_, character)| **character == ']')
        .map(|(index, _)| index)?;
    let set = CharacterSet {
        is_negated,
        members: &rest[first..closing],
    };
    Some((set, closing + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SRCH-004
    #[test]
    fn wildcard_words_and_other_words_match_within_the_name() {
        let pdfs = NamePattern::new("*.PDF");
        assert!(pdfs.matches("Bank statement.pdf"));
        assert!(
            pdfs.matches("statement.pdf.txt"),
            "found within the name, as the plain word .pdf is"
        );
        assert!(!pdfs.matches("statement.txt"));
        let decade = NamePattern::new("report-202?.pdf");
        assert!(decade.matches("Report-2024.pdf"));
        assert!(!decade.matches("report-2030.pdf"));
        let quarters = NamePattern::new("q[1-3]* summary");
        assert!(quarters.matches("Q2 summary.pdf"));
        assert!(!quarters.matches("Q4 summary.pdf"));
        assert!(!quarters.matches("Q2 notes.pdf"));
        let words = NamePattern::new("  Report  2026 ");
        assert!(words.matches("quarterly report 2026.docx"));
        assert!(!words.matches("quarterly report 2025.docx"));
    }

    /// A word found only inside the extension does not match, so the
    /// `pd` of `pd - m5753` does not find every PDF.
    ///
    /// parity: SRCH-004
    #[test]
    fn a_word_inside_the_extension_alone_does_not_match() {
        let pattern = NamePattern::new("pd - m5753");
        assert!(pattern.matches("PD - M5753RA - 66873.pdf"));
        assert!(!pattern.matches("WO - TM - M5753 - 66873.pdf"));
        assert!(!pattern.matches("DOC - M5753RA - M6069RA - 66873.pdf"));
        assert!(!pattern.matches("MTR - M5753 - 66873 - CC09-24-26.pdf"));
        for word in ["pdf", ".pdf", "66873.pdf", "873.p"] {
            assert!(NamePattern::new(word).matches("WO - M5753 - 66873.pdf"), "{word}");
        }
        assert!(!NamePattern::new("df").matches("WO - M5753 - 66873.pdf"));
        // Not extensions: the words are found anywhere.
        assert!(NamePattern::new("draft").matches("Notes v2.final draft"));
        assert!(NamePattern::new("2").matches("v1.2"));
        assert!(NamePattern::new("bash").matches(".bashrc"));
    }

    /// As in Windows 11 File Explorer, `?` stands for exactly one
    /// character: `extra_??.txt` finds `extra_30.txt` and `extra_31.txt`
    /// but neither `extra_2.txt` nor `extra_3.txt`, as `.txt` must follow
    /// the two characters. A Linux name may hold
    /// a literal `?`; it is still a wildcard, which also matches that `?`.
    ///
    /// parity: SRCH-004
    #[test]
    fn a_question_mark_stands_for_exactly_one_character() {
        let names = [
            "extra_2.txt",
            "extra_3.txt",
            "extra_30.txt",
            "extra_31.txt",
            "what?.txt",
            "ab",
        ];
        let found = |text: &str| -> Vec<&str> {
            let pattern = NamePattern::new(text);
            names.into_iter().filter(|name| pattern.matches(name)).collect()
        };
        assert_eq!(found("extra_??.txt"), ["extra_30.txt", "extra_31.txt"]);
        assert_eq!(found("extra_?.txt"), ["extra_2.txt", "extra_3.txt"]);
        assert_eq!(
            found("?xtra*"),
            ["extra_2.txt", "extra_3.txt", "extra_30.txt", "extra_31.txt"]
        );
        assert_eq!(found("*_3?.txt"), ["extra_30.txt", "extra_31.txt"]);
        assert_eq!(found("??"), names, "every name holds two characters");
        assert_eq!(found("extra_???.txt"), Vec::<&str>::new());
        assert_eq!(found("what?.txt"), ["what?.txt"]);
        assert_eq!(found("wha??.txt"), ["what?.txt"]);
        assert!(
            NamePattern::new("what?.txt").matches("whatX.txt"),
            "? is a wildcard"
        );
    }

    /// As in Windows 11 File Explorer, a wildcard word matches within the
    /// name, as a plain word does: `a?b` finds `a1b.txt` and `acb.txt`,
    /// where it used to have to match the whole name. It starts before
    /// the extension or is the whole extension, as a plain word must.
    ///
    /// parity: SRCH-004
    #[test]
    fn a_wildcard_word_matches_within_the_name_as_a_plain_word_does() {
        let names = [
            "a1b.txt",
            "acb.txt",
            "notes.txt",
            "Xa2b report.md",
            "plan.pdf",
            "ab",
        ];
        let found = |text: &str| -> Vec<&str> {
            let pattern = NamePattern::new(text);
            names.into_iter().filter(|name| pattern.matches(name)).collect()
        };
        assert_eq!(found("a?b"), ["a1b.txt", "acb.txt", "Xa2b report.md"]);
        assert_eq!(found("?cb"), ["acb.txt"]);
        assert_eq!(found("a*b"), ["a1b.txt", "acb.txt", "Xa2b report.md", "ab"]);
        assert_eq!(found("a?b.txt"), ["a1b.txt", "acb.txt"]);
        assert_eq!(found("*.txt"), ["a1b.txt", "acb.txt", "notes.txt"]);
        assert_eq!(found("*"), names);
        assert_eq!(found("a?b report"), ["Xa2b report.md"]);
        // Inside the extension only as the whole extension, as `pdf` is.
        assert_eq!(found("p?f"), ["plan.pdf"]);
        assert_eq!(found("?df"), ["plan.pdf"]);
        assert_eq!(found("d?"), Vec::<&str>::new());
    }

    #[test]
    fn sets_negation_and_plain_brackets() {
        assert!(wildcard_matches("[!a-c]*", "draft"));
        assert!(!wildcard_matches("[!a-c]*", "budget"));
        assert!(wildcard_matches("[]x]", "]"));
        assert!(wildcard_matches("draft [v2*", "draft [v2 final"));
        assert!(wildcard_matches("*", ""));
        assert!(!wildcard_matches("?", ""));
    }

    /// The one-pass search finds a pattern exactly where trying it from
    /// every start of the name, as `pattern*` before the extension or as
    /// the whole extension, finds it.
    ///
    /// parity: SRCH-004
    #[test]
    fn the_one_pass_search_agrees_with_trying_every_start() {
        let patterns = [
            "a?b",
            "*",
            "?",
            "??",
            "*.txt",
            "a*b",
            "[!a-c]*",
            "[]x]",
            "draft [v2*",
            "p?f",
            "?df",
            "d?",
            "*_3?.txt",
            "x*y*z",
            "[a-",
            "**",
            "a**b?",
            ".pdf",
            "[^p]df",
        ];
        let names = [
            "",
            "a",
            "ab",
            "a1b.txt",
            "acb.txt",
            "plan.pdf",
            "draft [v2 final",
            "]",
            "extra_31.txt",
            "xayaz.md",
            "[a-b",
            "aab.pdf.txt",
            "über.pdf",
            "v1.2",
            ".bashrc",
            "notes v2.final draft",
        ];
        for pattern in patterns {
            for name in names {
                let extension = extension_start(name);
                let open_end = format!("{pattern}*");
                let expected = name
                    .char_indices()
                    .map(|(start, _)| start)
                    .chain(std::iter::once(name.len()))
                    .any(|start| match extension {
                        Some(extension) if start >= extension => {
                            start == extension && wildcard_matches(pattern, &name[start..])
                        }
                        _ => wildcard_matches(&open_end, &name[start..]),
                    });
                assert_eq!(
                    wildcard_in_name(pattern, name),
                    expected,
                    "{pattern:?} in {name:?}"
                );
            }
        }
    }

    #[test]
    fn a_word_may_name_the_folder_but_a_wildcard_only_the_name() {
        let pattern = NamePattern::new("work *.txt");
        assert!(pattern.matches_lowercase("plan.txt", "/home/demo/work"));
        assert!(!pattern.matches_lowercase("plan.pdf", "/home/demo/work"));
        assert!(!NamePattern::new("work*").matches_lowercase("plan.txt", "/home/demo/work"));
    }
}
