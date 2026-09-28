// SPDX-License-Identifier: AGPL-3.0-only
//! Finding settings by what they say.
//!
//! Ports `settingsSearch` in `desktop/ui/app.js`: every word typed into
//! "Search settings" must appear, ignoring case, in a setting's title, its
//! description or its keywords (the Python app's `data-search-terms`), so
//! "watch live", "zoom" and "dolphin" find the settings people mean.

/// What a setting row says, and the other words it is found by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowText {
    /// The row's name, such as "Text size".
    pub title: &'static str,
    /// The line under the name.
    pub description: &'static str,
    /// Words people search for it by that it does not show, such as
    /// "zoom" for Text size.
    pub keywords: &'static str,
}

impl RowText {
    /// Whether every word of `query` appears in the row's text.
    pub(crate) fn matches(&self, query: &SearchQuery) -> bool {
        let searchable = [self.title, self.description, self.keywords].join(" ");
        query.is_found_in(&searchable)
    }
}

/// The words typed into "Search settings", in lower case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SearchQuery {
    words: Vec<String>,
}

impl SearchQuery {
    /// The query `typed` asks for: its words, ignoring case and spacing.
    pub(crate) fn parse(typed: &str) -> Self {
        let words = typed.split_whitespace().map(str::to_lowercase).collect();
        Self { words }
    }

    /// True when nothing is typed, so every setting shows.
    pub(crate) fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Whether every word appears in `text`, ignoring case.
    fn is_found_in(&self, text: &str) -> bool {
        let text = text.to_lowercase();
        self.words.iter().all(|word| text.contains(word.as_str()))
    }
}

/// The line under the search box: "1 matching setting", "3 matching
/// settings" or "No matching settings", as `settingsSearch` words it.
pub(crate) fn match_count_text(count: usize) -> String {
    match count {
        0 => "No matching settings".to_owned(),
        1 => "1 matching setting".to_owned(),
        _ => format!("{count} matching settings"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_SIZE: RowText = RowText {
        title: "Text size",
        description: "Ctrl + makes text larger, Ctrl − smaller, and Ctrl 0 resets it.",
        keywords: "zoom font accessibility",
    };

    /// A query and whether it finds [`TEXT_SIZE`].
    struct QueryCase {
        typed: &'static str,
        finds_text_size: bool,
    }

    #[test]
    fn every_typed_word_must_appear_in_the_title_description_or_keywords() {
        let cases = [
            QueryCase {
                typed: "text",
                finds_text_size: true,
            },
            QueryCase {
                typed: "ZOOM",
                finds_text_size: true,
            },
            QueryCase {
                typed: "  larger   size ",
                finds_text_size: true,
            },
            QueryCase {
                typed: "text dark",
                finds_text_size: false,
            },
            QueryCase {
                typed: "brave",
                finds_text_size: false,
            },
        ];
        for case in cases {
            let query = SearchQuery::parse(case.typed);
            assert_eq!(
                TEXT_SIZE.matches(&query),
                case.finds_text_size,
                "{:?}",
                case.typed
            );
        }
    }

    #[test]
    fn an_empty_query_is_no_search() {
        assert!(SearchQuery::parse("  ").is_empty());
        assert!(!SearchQuery::parse("x").is_empty());
    }

    #[test]
    fn the_count_reads_as_in_the_python_app() {
        assert_eq!(match_count_text(0), "No matching settings");
        assert_eq!(match_count_text(1), "1 matching setting");
        assert_eq!(match_count_text(4), "4 matching settings");
    }
}
