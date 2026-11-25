// SPDX-License-Identifier: AGPL-3.0-only
//! The search box's words read as Windows Search's Advanced Query Syntax
//! does: quoted phrases, the operators `AND`, `OR` and `NOT`, parentheses
//! and the name and file-extension properties.
//!
//! Microsoft documents the syntax for Windows Search
//! (<https://learn.microsoft.com/en-us/windows/win32/search/-search-3x-advancedquerysyntax>,
//! <https://learn.microsoft.com/en-us/windows/win32/lwef/-search-2x-wds-aqsreference>):
//!
//! - "AQS is not case sensitive, with the exception of AND, OR, and NOT,
//!   which must be in all uppercase", so `and`, `or` and `not` stay words.
//! - Words apart are both required: `<query> <query>` is "Same as
//!   `<query> AND <query>`".
//! - "If a query has two or more uses of AND or OR, they will bind from
//!   left to right"; parentheses group, as in `author:(theresa OR lee)`.
//! - `"social security"` "Finds items that contain the exact phrase
//!   social security"; quoted text is "not interpreted as part of the
//!   syntax", and a doubled quote stands for one.
//! - "File name | filename or file | filename:MyResume"; the video table
//!   names it `name`. "File extension | ext or fileext | ext:.txt".
//! - A property's value may start with an operator: `=` (equal), `<>` or
//!   `-` (not equal), `~<` (begins with), `~>` (ends with), `~=` or `~~`
//!   (contains), `~!` (does not contain) and `~` ("DOS-style wildcard
//!   characters": `?` one character, `*` any).
//!
//! The expression is kept as clauses that must all hold, each a list of
//! terms of which one must hold (conjunctive normal form), so the folder's
//! filter, the live walk, the search of contents and the cache all apply
//! the same rules ([`super::NamePattern`]).

/// Most clauses an expression keeps; `OR` between groups multiplies them,
/// and a search typed by hand never needs more.
const MAX_CLAUSES: usize = 64;

/// Most terms one clause keeps.
const MAX_TERMS_PER_CLAUSE: usize = 64;

/// The deepest group of parentheses read as a group; groups inside it are
/// read as if their parentheses were not there. A search typed by hand
/// never nests this deep, and each level is read by a call of its own.
const MAX_GROUP_DEPTH: usize = 32;

/// What one term tests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Test {
    /// Text found in the name, as a plain word is, or in the folder that
    /// holds the item; a quoted phrase is one such text, spaces and all.
    Word(String),
    /// A wildcard word found within the name (SRCH-004).
    Wildcard(String),
    /// A property of the name (`name:`, `filename:`, `file:`).
    Name(NameTest),
    /// The file name extension (`ext:`, `fileext:`), without its dot; it
    /// may hold wildcards.
    Extension(String),
}

/// How `name:` compares the name with its value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum NameTest {
    /// The value is in the name, as a plain word is, but not only in the
    /// folder that holds the item.
    Contains(String),
    /// The name is the value (`=`).
    Equals(String),
    /// The name begins with the value (`~<`).
    BeginsWith(String),
    /// The name ends with the value (`~>`).
    EndsWith(String),
    /// The whole name matches the wildcard value (`~`).
    Wildcard(String),
}

/// One term of a clause: a test, or its opposite.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Term {
    /// The term holds when the test fails (`NOT`, `<>`, `-`, `~!`).
    pub negated: bool,
    /// What it tests.
    pub test: Test,
}

impl Term {
    /// Whether the term holds for `lowercase_name`, with
    /// `lowercase_context` the item's folder or text, as
    /// [`super::NamePattern::matches_lowercase`] takes them.
    pub(crate) fn holds(&self, lowercase_name: &str, lowercase_context: &str) -> bool {
        self.test.passes(lowercase_name, lowercase_context) != self.negated
    }

    /// The text a positive term marks in a name; `None` for the others.
    pub(crate) fn marked_text(&self) -> Option<&str> {
        if self.negated {
            return None;
        }
        match &self.test {
            Test::Word(word) | Test::Name(NameTest::Contains(word)) => Some(word),
            Test::Name(NameTest::Equals(word) | NameTest::BeginsWith(word) | NameTest::EndsWith(word)) => {
                Some(word)
            }
            Test::Wildcard(_) | Test::Name(NameTest::Wildcard(_)) | Test::Extension(_) => None,
        }
    }

    /// The text every match holds in its name or its folder, which the
    /// cache can look for in its index; `None` when there is none.
    pub(crate) fn required_text(&self) -> Option<&str> {
        if self.negated {
            return None;
        }
        match &self.test {
            Test::Word(word)
            | Test::Name(
                NameTest::Contains(word)
                | NameTest::Equals(word)
                | NameTest::BeginsWith(word)
                | NameTest::EndsWith(word),
            ) => Some(word),
            Test::Wildcard(_) | Test::Name(NameTest::Wildcard(_)) | Test::Extension(_) => None,
        }
    }
}

impl Test {
    fn passes(&self, lowercase_name: &str, lowercase_context: &str) -> bool {
        use super::pattern::{wildcard_in_name, wildcard_matches, word_in_name};
        match self {
            Test::Word(word) => {
                word_in_name(word, lowercase_name) || lowercase_context.contains(word.as_str())
            }
            Test::Wildcard(pattern) => wildcard_in_name(pattern, lowercase_name),
            Test::Name(NameTest::Contains(word)) => word_in_name(word, lowercase_name),
            Test::Name(NameTest::Equals(word)) => lowercase_name == word,
            Test::Name(NameTest::BeginsWith(word)) => lowercase_name.starts_with(word.as_str()),
            Test::Name(NameTest::EndsWith(word)) => lowercase_name.ends_with(word.as_str()),
            Test::Name(NameTest::Wildcard(pattern)) => wildcard_matches(pattern, lowercase_name),
            Test::Extension(extension) => {
                let found = file_extension(lowercase_name).unwrap_or_default();
                if super::pattern::is_wildcard(extension) {
                    wildcard_matches(extension, found)
                } else {
                    found == extension
                }
            }
        }
    }
}

/// The text after the last dot of a name, as Windows' file extension is;
/// `None` without a dot, or for a name such as `.bashrc` that only starts
/// with one.
fn file_extension(lowercase_name: &str) -> Option<&str> {
    let dot = lowercase_name.rfind('.').filter(|&dot| dot > 0)?;
    Some(&lowercase_name[dot + 1..])
}

/// Clauses that must all hold, each holding when one of its terms does.
pub(crate) type Clauses = Vec<Vec<Term>>;

/// One piece of the typed text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// Text; `quoted_from` is where its first quoted part starts, when it
    /// has one: quoted text is never an operator or a wildcard, and only
    /// a property named before the quote counts.
    Text {
        text: String,
        quoted_from: Option<usize>,
    },
    /// A property name followed by a parenthesis: `ext:(`.
    PropertyGroup(String),
    Open,
    Close,
}

/// Splits `text` into tokens: blanks separate them, quotes hold blanks and
/// parentheses, and `""` inside quotes is one quote.
fn tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut characters = text.chars().peekable();
    while let Some(&character) = characters.peek() {
        if is_blank(character) {
            characters.next();
            continue;
        }
        if character == '(' {
            characters.next();
            tokens.push(Token::Open);
            continue;
        }
        if character == ')' {
            characters.next();
            tokens.push(Token::Close);
            continue;
        }
        let mut word = String::new();
        let mut quoted_from = None;
        while let Some(&character) = characters.peek() {
            if is_blank(character) || character == ')' {
                break;
            }
            if character == '(' {
                if word.ends_with(':') && quoted_from.is_none() && property(&word[..word.len() - 1]).is_some()
                {
                    characters.next();
                    word.pop();
                    tokens.push(Token::PropertyGroup(word));
                    word = String::new();
                }
                break;
            }
            characters.next();
            if character == '"' {
                quoted_from.get_or_insert(word.len());
                read_quoted(&mut characters, &mut word);
            } else {
                word.push(character);
            }
        }
        if !word.is_empty() || quoted_from.is_some() {
            tokens.push(Token::Text {
                text: word,
                quoted_from,
            });
        }
    }
    tokens
}

/// Whether `character` separates words, as Python's `str.split()` does:
/// Unicode `White_Space` and the information separators U+001C to U+001F.
fn is_blank(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

/// Reads up to the closing quote into `word`; `""` is one quote, and an
/// unclosed quote runs to the end, so a phrase being typed already works.
fn read_quoted(characters: &mut std::iter::Peekable<std::str::Chars<'_>>, word: &mut String) {
    while let Some(character) = characters.next() {
        if character == '"' {
            if characters.peek() == Some(&'"') {
                characters.next();
                word.push('"');
            } else {
                return;
            }
        } else {
            word.push(character);
        }
    }
}

/// The properties read from a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Property {
    Name,
    Extension,
}

/// The property a keyword names, ignoring case.
fn property(keyword: &str) -> Option<Property> {
    match keyword.to_lowercase().as_str() {
        "name" | "filename" | "file" => Some(Property::Name),
        "ext" | "fileext" => Some(Property::Extension),
        _ => None,
    }
}

/// Reads `text` into clauses, with `normalise` applied to every value
/// (lower-casing, or the cache's folding). Text that is not syntax is
/// searched for as typed, so a lone `NOT` or an unmatched parenthesis
/// never hides results.
pub(crate) fn parse(text: &str, normalise: &dyn Fn(&str) -> String) -> Clauses {
    let tokens = tokens(text);
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        normalise,
        depth: 0,
        flattened: 0,
    };
    let mut clauses = parser.sequence(None, false).unwrap_or_default();
    dedupe(&mut clauses);
    clauses
}

/// Drops repeated terms in a clause and repeated clauses.
fn dedupe(clauses: &mut Clauses) {
    for clause in clauses.iter_mut() {
        let mut seen: Vec<Term> = Vec::new();
        clause.retain(|term| {
            let fresh = !seen.contains(term);
            if fresh {
                seen.push(term.clone());
            }
            fresh
        });
    }
    let mut seen: Vec<Vec<Term>> = Vec::new();
    clauses.retain(|clause| {
        let fresh = !clause.is_empty() && !seen.contains(clause);
        if fresh {
            seen.push(clause.clone());
        }
        fresh
    });
}

/// How two parts are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Join {
    And,
    Or,
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    normalise: &'a dyn Fn(&str) -> String,
    /// How many groups the token read last is in.
    depth: usize,
    /// How many groups past [`MAX_GROUP_DEPTH`] are open, read as if their
    /// parentheses were not there.
    flattened: usize,
}

impl Parser<'_> {
    /// Reads parts joined left to right until the end or, inside
    /// parentheses, the closing one; `property` applies to bare words in
    /// a group such as `ext:(pdf OR docx)`. `None` when nothing was read.
    fn sequence(&mut self, property: Option<Property>, in_group: bool) -> Option<Clauses> {
        let mut result: Option<Clauses> = None;
        let mut join = Join::And;
        let mut negate = false;
        while let Some(token) = self.tokens.get(self.at) {
            self.at += 1;
            let part = match token {
                // The end of a group read flat ends nothing.
                Token::Close if self.flattened > 0 => {
                    self.flattened -= 1;
                    continue;
                }
                Token::Close if in_group => break,
                // An unmatched `)` is ignored.
                Token::Close => continue,
                Token::Open | Token::PropertyGroup(_) if self.depth >= MAX_GROUP_DEPTH => {
                    self.flattened += 1;
                    continue;
                }
                Token::Text {
                    text,
                    quoted_from: None,
                } if text == "AND" => {
                    join = Join::And;
                    continue;
                }
                Token::Text {
                    text,
                    quoted_from: None,
                } if text == "OR" => {
                    join = Join::Or;
                    continue;
                }
                Token::Text {
                    text,
                    quoted_from: None,
                } if text == "NOT" => {
                    negate = !negate;
                    continue;
                }
                Token::Open => self.group(property),
                Token::PropertyGroup(keyword) => self.group(self::property(keyword)),
                Token::Text { text, quoted_from } => self
                    .term(text, *quoted_from, property)
                    .map(|term| vec![vec![term]]),
            };
            let Some(mut part) = part else {
                continue;
            };
            if negate {
                part = negated(part);
                negate = false;
            }
            result = Some(match result {
                None => part,
                Some(earlier) if join == Join::Or => or(&earlier, &part),
                Some(mut earlier) => {
                    earlier.extend(part);
                    earlier.truncate(MAX_CLAUSES);
                    earlier
                }
            });
            join = Join::And;
        }
        result
    }

    /// The group that starts here, up to its closing parenthesis.
    fn group(&mut self, property: Option<Property>) -> Option<Clauses> {
        self.depth += 1;
        let group = self.sequence(property, true);
        self.depth -= 1;
        group
    }

    /// The term `text` stands for: a property with its value, a wildcard
    /// or a word. `property` is the group's property, if any.
    fn term(&self, text: &str, quoted_from: Option<usize>, property: Option<Property>) -> Option<Term> {
        let normalise = self.normalise;
        let quoted = quoted_from.is_some();
        let named = text
            .find(':')
            .filter(|&colon| quoted_from.is_none_or(|quote| colon < quote))
            .and_then(|colon| Some((self::property(&text[..colon])?, &text[colon + 1..])));
        let (property, value) = match (named, property) {
            (Some((keyword, value)), _) => (Some(keyword), value),
            (None, group) => (group, text),
        };
        if let Some(property) = property {
            return property_term(property, &normalise(value), quoted);
        }
        let value = normalise(text);
        if value.is_empty() {
            return None;
        }
        let test = if !quoted && super::pattern::is_wildcard(&value) {
            Test::Wildcard(value)
        } else {
            Test::Word(value)
        };
        Some(Term { negated: false, test })
    }
}

/// The term of `property` with `value`, which may start with an operator.
fn property_term(property: Property, value: &str, quoted: bool) -> Option<Term> {
    let (negated, operator, rest) = operator(value);
    if rest.is_empty() {
        return None;
    }
    let rest = rest.to_owned();
    let test = match property {
        Property::Extension => {
            let extension = rest.trim_start_matches('*').trim_start_matches('.').to_owned();
            if extension.is_empty() {
                return None;
            }
            Test::Extension(extension)
        }
        Property::Name => Test::Name(match operator {
            Operator::Equals => NameTest::Equals(rest),
            Operator::BeginsWith => NameTest::BeginsWith(rest),
            Operator::EndsWith => NameTest::EndsWith(rest),
            Operator::Wildcard => NameTest::Wildcard(rest),
            Operator::Contains if !quoted && super::pattern::is_wildcard(&rest) => {
                return Some(Term {
                    negated,
                    test: Test::Wildcard(rest),
                })
            }
            Operator::Contains => NameTest::Contains(rest),
        }),
    };
    Some(Term { negated, test })
}

/// How a property's value is compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Contains,
    Equals,
    BeginsWith,
    EndsWith,
    Wildcard,
}

/// The operator `value` starts with, whether it negates, and the rest.
fn operator(value: &str) -> (bool, Operator, &str) {
    const OPERATORS: [(&str, bool, Operator); 10] = [
        ("<>", true, Operator::Equals),
        ("\u{2260}", true, Operator::Equals),
        ("~<", false, Operator::BeginsWith),
        ("~>", false, Operator::EndsWith),
        ("~=", false, Operator::Contains),
        ("~~", false, Operator::Contains),
        ("~!", true, Operator::Contains),
        ("~", false, Operator::Wildcard),
        ("=", false, Operator::Equals),
        ("-", true, Operator::Equals),
    ];
    for (prefix, negated, operator) in OPERATORS {
        if let Some(rest) = value.strip_prefix(prefix) {
            return (negated, operator, rest);
        }
    }
    (false, Operator::Contains, value)
}

/// `one OR two`: every pair of a clause from each, joined.
fn or(one: &Clauses, two: &Clauses) -> Clauses {
    let mut clauses = Vec::new();
    'pairs: for first in one {
        for second in two {
            if clauses.len() == MAX_CLAUSES {
                break 'pairs;
            }
            // Only the terms kept are copied.
            let clause = first
                .iter()
                .chain(second)
                .take(MAX_TERMS_PER_CLAUSE)
                .cloned()
                .collect();
            clauses.push(clause);
        }
    }
    clauses
}

/// `NOT clauses`: one of the clauses fails, so for each clause all of its
/// terms fail; written back as clauses: every choice of one negated term
/// from each clause, in the order `OR` would join them, the first
/// [`MAX_CLAUSES`] only. Each choice is made once, so a negated group costs
/// no more than the clauses it gives.
fn negated(clauses: Clauses) -> Clauses {
    let choices: Vec<Vec<Term>> = clauses
        .into_iter()
        .map(|clause| {
            clause
                .into_iter()
                .map(|term| Term {
                    negated: !term.negated,
                    ..term
                })
                .collect()
        })
        .collect();
    if choices.is_empty() || choices.iter().any(Vec::is_empty) {
        return Vec::new();
    }
    let mut picked = vec![0_usize; choices.len()];
    let mut negated = Vec::new();
    while negated.len() < MAX_CLAUSES {
        let clause = picked
            .iter()
            .zip(&choices)
            .take(MAX_TERMS_PER_CLAUSE)
            .map(|(&index, terms)| terms[index].clone())
            .collect();
        negated.push(clause);
        // The last clause's choice moves fastest, as `or` pairs them.
        let Some(position) = (0..picked.len())
            .rev()
            .find(|&position| picked[position] + 1 < choices[position].len())
        else {
            break;
        };
        picked[position] += 1;
        picked[position + 1..].fill(0);
    }
    negated
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parentheses nested far deeper than anyone types are read without
    /// running out of stack, and in a moment: the groups past the deepest
    /// one followed are read as if their parentheses were not there
    /// (found by fuzzing).
    #[test]
    fn groups_nested_past_the_deepest_followed_are_read_flat() {
        let opened = "(".repeat(100_000);
        assert_eq!(parse(&opened, &lower), Clauses::new());
        let closed = format!("{}a{}", "(".repeat(100_000), ")".repeat(100_000));
        assert_eq!(parse(&closed, &lower), vec![vec![word("a")]]);
        let started = std::time::Instant::now();
        let negated = parse(&"NOT (a ".repeat(3000), &lower);
        assert!(negated.len() <= MAX_CLAUSES);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
        // Every word of a deep group is still required, and a closing
        // parenthesis past the deepest group does not end the groups
        // around it early.
        let deep: String = (0..MAX_GROUP_DEPTH + 8).fold(String::new(), |mut text, level| {
            let _ = std::fmt::Write::write_fmt(&mut text, format_args!("w{level} ("));
            text
        });
        let deep = format!("{deep}last{} after", ")".repeat(MAX_GROUP_DEPTH + 8));
        let clauses = parse(&deep, &lower);
        assert_eq!(clauses.len(), MAX_GROUP_DEPTH + 8 + 2);
        assert!(clauses.contains(&vec![word("last")]));
        assert!(clauses.contains(&vec![word("after")]));
        assert_eq!(
            parse("a OR (b c)", &lower),
            vec![vec![word("a"), word("b")], vec![word("a"), word("c")]]
        );
    }

    /// `NOT` gives what joining each clause's negated terms with `OR` gives.
    #[test]
    fn a_negation_is_its_clauses_negated_terms_joined_with_or() {
        let reference = |clauses: &Clauses| -> Clauses {
            clauses
                .iter()
                .map(|clause| {
                    clause
                        .iter()
                        .map(|term| vec![not(term.clone())])
                        .collect::<Clauses>()
                })
                .reduce(|one, two| or(&one, &two))
                .unwrap_or_default()
        };
        let mut seed = 0x5EED_u32;
        for _ in 0..500 {
            let mut next = |below: u32| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed % below
            };
            let clauses: Clauses = (0..=next(5))
                .map(|clause| {
                    (0..=next(4))
                        .map(|term| word(&format!("w{clause}{term}")))
                        .collect()
                })
                .collect();
            assert_eq!(negated(clauses.clone()), reference(&clauses), "{clauses:?}");
        }
        let wide: Clauses = (0..9)
            .map(|clause| (0..3).map(|term| word(&format!("{clause}{term}"))).collect())
            .collect();
        assert_eq!(negated(wide.clone()), reference(&wide));
        assert_eq!(negated(wide).len(), MAX_CLAUSES);
    }

    /// A query that negates groups inside groups is read in a moment: the
    /// clauses it would multiply into are capped as they are made, not
    /// after (found by fuzzing).
    #[test]
    fn negated_groups_inside_groups_are_read_quickly() {
        let found = "re\"htory\"\u{d}01 \u{0}\u{c}\u{c}\u{0}\u{c}\u{fffd}t\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}NOT\u{c}\u{c}\u{c}(L\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}NOT\u{c}\u{c}\u{c}(L\u{c}\u{fffd}\u{c}\u{c}\u{c}\u{3}\u{c}\u{2}\u{c}\u{c}$\u{c}OR\u{c}\u{c}\u{c}\u{c}(\u{c}\u{c}!\u{c}\u{c}\u{c}\u{c}\u{c}NOT\u{c}W\u{fffd}\u{c}(L\u{c}\u{c}\u{c}\u{0}\u{0}\u{0}\u{c}\u{c}\u{c}\u{c}\u{0}\u{0}\u{0}\u{0}G*****\u{c}\u{c}\u{c}\u{c}D\u{1}P\u{1f}\u{c}!\u{c}\u{c}\u{fffd}\u{0}\u{0}\u{0}\u{c}\u{c}\u{c}NOT%%%%%t \u{0}\u{c}\u{c}\u{0}\u{c}\u{0}E\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}NOT\u{c}\u{c}\u{c}(L\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}NOT\u{c}\u{c}\u{c}(L\u{c}\u{c}\u{c}\u{c}\u{c}\u{3}\u{c}\u{2}\u{c}\u{c}\u{c}\u{c}OR\u{c}\u{c}\u{c}\u{c}(\u{c}\u{c}!\u{c}:\u{fffd}/\u{fffd}t d\u{c}\u{c}\u{c}(L\u{c}\u{c}\u{c}\u{c}\u{c}%%%%%%%%%%%%%%%%%%%%%%%%% fileext:\u{fffd}/\u{fffd}t d\u{fffd}\u{fffd}\u{fffd}%%%%%%%%%%%%%%t \u{0}\u{c}\u{c}\u{0}\u{c}\u{0}E\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}NOT\u{c}\u{c}\u{c}(L\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}\u{c}NOT\u{c}\u{c}\u{c}(L\u{c}\u{c}\u{c}\u{c}\u{c}\u{3}\u{c}\u{2}\u{c}\u{c}\u{c}\u{c}OR\u{c}\u{c}\u{c}\u{c}(\u{c}\u{c}!\u{c}:\u{fffd}/\u{fffd}t d\u{fffd}\u{fffd}\u{fffd}ed";
        let nested = (0..12).fold(String::new(), |mut text, level| {
            let _ = std::fmt::Write::write_fmt(
                &mut text,
                format_args!("NOT (a{level} b{level} OR c{level} d{level} "),
            );
            text
        });
        for query in [nested, found.to_owned()] {
            let started = std::time::Instant::now();
            let clauses = parse(&query, &lower);
            assert!(clauses.len() <= MAX_CLAUSES);
            assert!(clauses.iter().all(|clause| clause.len() <= MAX_TERMS_PER_CLAUSE));
            assert!(
                started.elapsed() < std::time::Duration::from_millis(200),
                "{:?}",
                started.elapsed()
            );
        }
    }

    fn lower(text: &str) -> String {
        text.to_lowercase()
    }

    fn word(text: &str) -> Term {
        Term {
            negated: false,
            test: Test::Word(text.to_owned()),
        }
    }

    fn not(term: Term) -> Term {
        Term {
            negated: true,
            ..term
        }
    }

    #[test]
    fn words_apart_are_all_required_and_a_quoted_phrase_is_one_word() {
        assert_eq!(
            parse("Annual  Report", &lower),
            [vec![word("annual")], vec![word("report")]]
        );
        assert_eq!(
            parse("\"Annual  Report\" 2026", &lower),
            [vec![word("annual  report")], vec![word("2026")]]
        );
        assert_eq!(
            parse("\"the \"\"blue\"\" team\"", &lower),
            [vec![word("the \"blue\" team")]]
        );
        assert_eq!(
            parse("\"unclosed phrase", &lower),
            [vec![word("unclosed phrase")]]
        );
        assert_eq!(
            parse("\"report*\"", &lower),
            [vec![word("report*")]],
            "quoted text is not syntax"
        );
        assert_eq!(
            parse("\"OR\" \"NOT\"", &lower),
            [vec![word("or")], vec![word("not")]]
        );
    }

    #[test]
    fn upper_case_operators_bind_left_to_right_and_lower_case_ones_are_words() {
        assert_eq!(parse("a AND b", &lower), [vec![word("a")], vec![word("b")]]);
        assert_eq!(parse("a OR b", &lower), [vec![word("a"), word("b")]]);
        assert_eq!(parse("a NOT b", &lower), [vec![word("a")], vec![not(word("b"))]]);
        assert_eq!(
            parse("a or b", &lower),
            [vec![word("a")], vec![word("or")], vec![word("b")]]
        );
        // Left to right: (a OR b) AND c, then (a AND b) OR c.
        assert_eq!(
            parse("a OR b c", &lower),
            [vec![word("a"), word("b")], vec![word("c")]]
        );
        assert_eq!(
            parse("a b OR c", &lower),
            [vec![word("a"), word("c")], vec![word("b"), word("c")]]
        );
        assert_eq!(
            parse("a (b OR c)", &lower),
            [vec![word("a")], vec![word("b"), word("c")]]
        );
        assert_eq!(
            parse("NOT (a OR b)", &lower),
            [vec![not(word("a"))], vec![not(word("b"))]]
        );
        assert_eq!(parse("NOT (a b)", &lower), [vec![not(word("a")), not(word("b"))]]);
        assert_eq!(parse("NOT NOT a", &lower), [vec![word("a")]]);
        assert!(parse("NOT", &lower).is_empty());
        assert!(parse("OR AND", &lower).is_empty());
        assert_eq!(parse("a) (b", &lower), [vec![word("a")], vec![word("b")]]);
    }

    #[test]
    fn name_and_extension_properties_take_operators() {
        let name = |test: NameTest| Term {
            negated: false,
            test: Test::Name(test),
        };
        let extension = |text: &str| Term {
            negated: false,
            test: Test::Extension(text.to_owned()),
        };
        assert_eq!(
            parse("name:Report", &lower),
            [vec![name(NameTest::Contains("report".into()))]]
        );
        assert_eq!(
            parse("FileName:report", &lower),
            [vec![name(NameTest::Contains("report".into()))]]
        );
        assert_eq!(
            parse("name:\"annual report\"", &lower),
            [vec![name(NameTest::Contains("annual report".into()))]]
        );
        assert_eq!(
            parse("name:=plan.txt", &lower),
            [vec![name(NameTest::Equals("plan.txt".into()))]]
        );
        assert_eq!(
            parse("name:~<rep", &lower),
            [vec![name(NameTest::BeginsWith("rep".into()))]]
        );
        assert_eq!(
            parse("name:~>.md", &lower),
            [vec![name(NameTest::EndsWith(".md".into()))]]
        );
        assert_eq!(
            parse("name:~\"r?p*\"", &lower),
            [vec![name(NameTest::Wildcard("r?p*".into()))]]
        );
        assert_eq!(
            parse("name:~!draft", &lower),
            [vec![not(name(NameTest::Contains("draft".into())))]]
        );
        assert_eq!(parse("ext:.PDF", &lower), [vec![extension("pdf")]]);
        assert_eq!(parse("fileext:pdf", &lower), [vec![extension("pdf")]]);
        assert_eq!(parse("ext:*.pdf", &lower), [vec![extension("pdf")]]);
        assert_eq!(parse("ext:<>pdf", &lower), [vec![not(extension("pdf"))]]);
        assert_eq!(
            parse("ext:(pdf OR docx)", &lower),
            [vec![extension("pdf"), extension("docx")]]
        );
        assert_eq!(parse("NOT ext:pdf", &lower), [vec![not(extension("pdf"))]]);
        // Unknown properties, and an empty value, stay words.
        assert_eq!(parse("author:lee", &lower), [vec![word("author:lee")]]);
        assert!(parse("ext:", &lower).is_empty());
    }

    #[test]
    fn the_tests_follow_the_folder_filter_rules() {
        let holds = |text: &str, name: &str| {
            parse(text, &lower).iter().all(|clause| {
                clause
                    .iter()
                    .any(|term| term.holds(&name.to_lowercase(), "/home/demo/work"))
            })
        };
        assert!(holds("\"annual report\"", "Annual report 2026.docx"));
        assert!(!holds("\"annual report\"", "Report annual.docx"));
        assert!(holds("report NOT draft", "Report final.pdf"));
        assert!(!holds("report NOT draft", "Report draft.txt"));
        assert!(holds("budget OR invoice", "Invoice 1042.pdf"));
        assert!(holds("ext:pdf", "Invoice 1042.pdf"));
        assert!(!holds("ext:pdf", "Invoice 1042.pdf.txt"));
        assert!(holds("ext:t?t", "notes.txt"));
        assert!(!holds("ext:pdf", "pdf"));
        assert!(holds("work", "plan.txt"), "a word may name the folder");
        assert!(!holds("name:work", "plan.txt"), "name: looks at the name alone");
        assert!(holds("name:=plan.txt", "Plan.txt"));
        assert!(!holds("name:=plan", "Plan.txt"));
        assert!(holds("name:~<pl", "Plan.txt"));
        assert!(holds("name:~>.TXT", "Plan.txt"));
        assert!(holds("name:~p?an.*", "Plan.txt"));
        assert!(!holds("name:~p?an", "Plan.txt"), "~ matches the whole name");
    }
}
