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

use super::expression::{self, Clauses};

/// The characters that make a word a wildcard pattern.
const WILDCARDS: [char; 3] = ['*', '?', '['];

/// Whether `word` is a wildcard pattern rather than text to find.
pub fn is_wildcard(word: &str) -> bool {
    word.contains(WILDCARDS)
}

/// What a search's words ask of a name: the clauses of
/// [`super::expression`], lower-cased.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamePattern {
    clauses: Clauses,
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
        }
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
                .any(|term| term.holds(lowercase_name, lowercase_context))
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
pub(crate) fn wildcard_in_name(pattern: &str, lowercase_name: &str) -> bool {
    // The pattern followed by `*` matches any text that starts with what
    // the pattern matches.
    let open_end = format!("{pattern}*");
    let extension = extension_start(lowercase_name);
    let mut starts = lowercase_name
        .char_indices()
        .map(|(start, _)| start)
        .chain(std::iter::once(lowercase_name.len()));
    starts.any(|start| {
        let rest = &lowercase_name[start..];
        match extension {
            Some(extension) if start >= extension => start == extension && wildcard_matches(pattern, rest),
            _ => wildcard_matches(&open_end, rest),
        }
    })
}

/// Whether the whole of `name` matches the wildcard `pattern`.
pub(crate) fn wildcard_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut at_pattern, mut at_name) = (0, 0);
    // Where to resume after the last `*`: the pattern after it, and the
    // name position it has consumed up to.
    let mut resume: Option<(usize, usize)> = None;
    while at_name < name.len() {
        match pattern.get(at_pattern) {
            Some('*') => {
                resume = Some((at_pattern + 1, at_name));
                at_pattern += 1;
                continue;
            }
            Some(&token) => {
                if let Some(length) = token_matches(&pattern[at_pattern..], token, name[at_name]) {
                    at_pattern += length;
                    at_name += 1;
                    continue;
                }
            }
            None => {}
        }
        // A mismatch: the last `*` takes one more character, or the name
        // does not match.
        let Some((after_star, consumed)) = resume else {
            return false;
        };
        resume = Some((after_star, consumed + 1));
        at_pattern = after_star;
        at_name = consumed + 1;
    }
    pattern[at_pattern..].iter().all(|token| *token == '*')
}

/// How many pattern characters matched `character`, when the pattern at
/// `rest`, whose first character is `token`, matches it.
fn token_matches(rest: &[char], token: char, character: char) -> Option<usize> {
    match token {
        '?' => Some(1),
        '[' => match character_set(rest) {
            Some((set, length)) => set.contains(character).then_some(length),
            None => (character == '[').then_some(1),
        },
        _ => (token == character).then_some(1),
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

    #[test]
    fn a_word_may_name_the_folder_but_a_wildcard_only_the_name() {
        let pattern = NamePattern::new("work *.txt");
        assert!(pattern.matches_lowercase("plan.txt", "/home/demo/work"));
        assert!(!pattern.matches_lowercase("plan.pdf", "/home/demo/work"));
        assert!(!NamePattern::new("work*").matches_lowercase("plan.txt", "/home/demo/work"));
    }
}
