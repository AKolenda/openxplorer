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

use std::sync::atomic::{AtomicBool, Ordering};

use super::expression::{self, Clauses};

/// The characters that make a word a wildcard pattern.
const WILDCARDS: [char; 3] = ['*', '?', '['];

/// Whether `word` is a wildcard pattern rather than text to find.
pub fn is_wildcard(word: &str) -> bool {
    word.contains(WILDCARDS)
}

/// How a search word may reach into a name's extension.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExtensionWords {
    /// Only as the whole extension, with or without its dot: `doc` and
    /// `.doc` do not find `report.docx`. The default.
    #[default]
    WholeOnly,
    /// Also from the extension's start, as Windows 11's search matches:
    /// `doc` and `.doc` find `report.docx`, `ocx` still does not.
    FromStart,
}

/// Whether search words match from the start of an extension
/// ([`ExtensionWords::FromStart`]), for every search the process makes
/// from now on: Settings > Search > "Match words at the start of file
/// extensions (like Windows)".
static WORDS_FROM_EXTENSION_START: AtomicBool = AtomicBool::new(false);

/// Sets how the searches this process makes from now on match words in
/// extensions, as the Settings switch says.
pub fn set_extension_words(mode: ExtensionWords) {
    WORDS_FROM_EXTENSION_START.store(mode == ExtensionWords::FromStart, Ordering::Relaxed);
}

/// The mode [`set_extension_words`] set last.
pub fn extension_words() -> ExtensionWords {
    if WORDS_FROM_EXTENSION_START.load(Ordering::Relaxed) {
        ExtensionWords::FromStart
    } else {
        ExtensionWords::WholeOnly
    }
}

/// What a search's words ask of a name: the clauses of
/// [`super::expression`], lower-cased, and how a word may reach into a
/// name's extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamePattern {
    clauses: Clauses,
    extensions: ExtensionWords,
}

impl NamePattern {
    /// The pattern of the search text `text`: its words, quoted phrases,
    /// operators and properties, lower-cased, each word once.
    pub fn new(text: &str) -> Self {
        Self::normalised(text, &str::to_lowercase)
    }

    /// The pattern of `text` with every value passed through `normalise`,
    /// as the cache folds its words.
    pub(crate) fn normalised(text: &str, normalise: &dyn Fn(&str) -> String) -> Self {
        Self {
            clauses: expression::parse(text, normalise),
            extensions: extension_words(),
        }
    }

    /// The pattern matching words in extensions as `mode` says, whatever
    /// the process-wide setting.
    #[must_use]
    pub fn with_extension_words(mut self, mode: ExtensionWords) -> Self {
        self.extensions = mode;
        self
    }

    /// Whether the search has no words, so it matches everything.
    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }

    /// Keeps the clauses whose terms, counted in order, are at most
    /// `most`, as the cache matches at most 20 words.
    pub(crate) fn keep_terms(&mut self, most: usize) {
        let mut counted = 0;
        self.clauses.retain(|clause| {
            counted += clause.len();
            counted <= most
        });
    }

    /// The clauses that must all hold.
    pub(crate) fn clauses(&self) -> &Clauses {
        &self.clauses
    }

    /// The words and phrases a match holds in its name, for marking
    /// them; not those a match must lack, nor wildcards or extensions.
    pub fn marked_words(&self) -> Vec<String> {
        let mut words: Vec<String> = Vec::new();
        for term in self.clauses.iter().flatten() {
            if let Some(word) = term.marked_text() {
                if !words.iter().any(|known| known == word) {
                    words.push(word.to_owned());
                }
            }
        }
        words
    }

    /// Whether `name` matches, ignoring case.
    pub fn matches(&self, name: &str) -> bool {
        self.matches_lowercase(&name.to_lowercase(), "")
    }

    /// Whether `lowercase_name`, a lower-cased name, matches: in every
    /// clause one term holds. A wildcard word is found within the name
    /// ([`wildcard_in_name`]), and another word occurs in the name or in
    /// `lowercase_context`, such as the folder that holds the item
    /// (SRCH-008) or the file's text, which may be empty.
    pub fn matches_lowercase(&self, lowercase_name: &str, lowercase_context: &str) -> bool {
        self.clauses.iter().all(|clause| {
            clause
                .iter()
                .any(|term| term.holds(lowercase_name, lowercase_context, self.extensions))
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
/// extension's dot, or reaching into the extension as `mode` allows: as
/// the whole extension, with or without the dot, or also from the start
/// of the extension or its dot. A word that starts at the dot reaches
/// into the extension as one without it does, so `.doc` finds
/// `report.docx` exactly when `doc` does.
pub(crate) fn word_in_name(word: &str, lowercase_name: &str, mode: ExtensionWords) -> bool {
    let Some(first) = lowercase_name.find(word) else {
        return false;
    };
    match extension_start(lowercase_name) {
        Some(extension) => {
            let dot = extension - 1;
            match mode {
                ExtensionWords::FromStart => first <= extension,
                ExtensionWords::WholeOnly => {
                    first < dot || lowercase_name[extension..] == *word || lowercase_name[dot..] == *word
                }
            }
        }
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
pub(crate) fn wildcard_in_name(pattern: &str, lowercase_name: &str, mode: ExtensionWords) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let tokens = tokens(&pattern);
    let extension = extension_start(lowercase_name);
    let mut states = States::new(tokens.len());
    // Every position before the extension's dot may start a match, which
    // may run on into the extension, and with `FromStart` the dot and the
    // extension's first character too; reaching the pattern's end at any
    // point means the pattern matches the text from that start.
    let may_start = |at: usize, extension: usize| match mode {
        ExtensionWords::WholeOnly => at + 1 < extension,
        ExtensionWords::FromStart => at <= extension,
    };
    for (at, character) in lowercase_name.char_indices() {
        if extension.is_none_or(|extension| may_start(at, extension)) {
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
    // From the dot or inside the extension, only as the whole extension,
    // with or without its dot.
    extension.is_some_and(|extension| {
        matches_tokens(&tokens, &lowercase_name[extension..])
            || matches_tokens(&tokens, &lowercase_name[extension - 1..])
    })
}

/// Whether the whole of `name` matches the wildcard `pattern`, as
/// `name:~` asks ([`super::expression`]).
pub(crate) fn wildcard_matches(pattern: &str, name: &str) -> bool {
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
        // A word starting at the dot reaches into the extension as one
        // without the dot does: only the whole extension matches.
        for word in ["doc", ".doc", "do?", ".do?", ".d*c"] {
            assert!(!NamePattern::new(word).matches("report.docx"), "{word}");
        }
        for word in ["docx", ".docx", ".doc?", "t.doc", "report.doc"] {
            assert!(NamePattern::new(word).matches("report.docx"), "{word}");
        }
    }

    /// With "Match words at the start of file extensions (like Windows)"
    /// on, a word also matches from the start of an extension or its dot,
    /// as Windows 11 answered in a folder of "WO - M5753.pdf",
    /// "report.docx" and "notes.txt"; a word inside an extension still
    /// does not, and the default stays the owner's rule.
    ///
    /// parity: SRCH-004
    #[test]
    fn words_may_match_from_an_extensions_start_as_windows_does() {
        let names = ["WO - M5753.pdf", "report.docx", "notes.txt"];
        let found = |word: &str, mode: ExtensionWords| -> Vec<&str> {
            let pattern = NamePattern::new(word).with_extension_words(mode);
            names.into_iter().filter(|name| pattern.matches(name)).collect()
        };
        let windows = ExtensionWords::FromStart;
        assert_eq!(found("doc", windows), ["report.docx"]);
        assert_eq!(found(".doc", windows), ["report.docx"]);
        assert_eq!(found("docx", windows), ["report.docx"]);
        assert_eq!(found("pd", windows), ["WO - M5753.pdf"]);
        assert_eq!(found("txt", windows), ["notes.txt"]);
        assert!(found("mp", windows).is_empty());
        assert!(found("ocx", windows).is_empty(), "not from inside the extension");
        assert_eq!(found(".do?", windows), ["report.docx"], "a wildcard too");
        let owner = ExtensionWords::WholeOnly;
        assert!(found("doc", owner).is_empty());
        assert!(
            found(".doc", owner).is_empty(),
            "doc and .doc agree in both modes"
        );
        assert_eq!(found("docx", owner), ["report.docx"]);
        assert_eq!(ExtensionWords::default(), owner, "the owner's rule by default");
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

    /// As Windows Search's Advanced Query Syntax reads them
    /// (<https://learn.microsoft.com/en-us/windows/win32/search/-search-3x-advancedquerysyntax>,
    /// <https://learn.microsoft.com/en-us/windows/win32/lwef/-search-2x-wds-aqsreference>):
    /// a quoted phrase is found as typed, upper-case `AND`, `OR` and `NOT`
    /// combine words, and `name:`, `filename:`, `ext:` and `fileext:` test
    /// the name and its extension.
    ///
    /// parity: SRCH-047
    #[test]
    fn phrases_operators_and_properties_follow_advanced_query_syntax() {
        let names = [
            "Annual report 2026.docx",
            "Report draft.txt",
            "report final.pdf",
            "Invoice 1042.pdf",
            "annual budget.xlsx",
        ];
        let found = |text: &str| -> Vec<&str> {
            let pattern = NamePattern::new(text);
            names.into_iter().filter(|name| pattern.matches(name)).collect()
        };
        assert_eq!(found("\"annual report\""), ["Annual report 2026.docx"]);
        assert_eq!(found("\"report annual\""), Vec::<&str>::new());
        assert_eq!(
            found("report NOT draft"),
            ["Annual report 2026.docx", "report final.pdf"]
        );
        assert_eq!(
            found("budget OR invoice"),
            ["Invoice 1042.pdf", "annual budget.xlsx"]
        );
        assert_eq!(found("report AND annual"), ["Annual report 2026.docx"]);
        assert_eq!(found("ext:pdf"), ["report final.pdf", "Invoice 1042.pdf"]);
        assert_eq!(found("ext:.PDF NOT invoice"), ["report final.pdf"]);
        assert_eq!(
            found("ext:(docx OR xlsx)"),
            ["Annual report 2026.docx", "annual budget.xlsx"]
        );
        assert_eq!(found("name:draft"), ["Report draft.txt"]);
        assert_eq!(
            found("filename:~<report"),
            ["Report draft.txt", "report final.pdf"]
        );
        assert_eq!(
            found("annual (budget OR 2026)"),
            ["Annual report 2026.docx", "annual budget.xlsx"]
        );
        // Lower-case operators are words, as Microsoft documents.
        assert_eq!(found("final or draft"), Vec::<&str>::new());
        assert_eq!(found("report and"), Vec::<&str>::new());
        assert_eq!(
            NamePattern::new("\"annual report\" NOT budget ext:docx").marked_words(),
            ["annual report"]
        );
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
                        Some(extension) if start + 1 >= extension => {
                            (start == extension || start + 1 == extension)
                                && wildcard_matches(pattern, &name[start..])
                        }
                        _ => wildcard_matches(&open_end, &name[start..]),
                    });
                assert_eq!(
                    wildcard_in_name(pattern, name, ExtensionWords::WholeOnly),
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
