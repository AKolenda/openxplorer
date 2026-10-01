// SPDX-License-Identifier: AGPL-3.0-only
//! Finding settings by what they say and show.
//!
//! Ports `settingsSearch` in `v2.0.0:desktop/ui/app.js` (SET-004): every word
//! typed into "Search settings" must appear, ignoring case, in what a
//! setting says or shows. For a row that is its title, its description,
//! its keywords (the Python app's `data-search-terms`), the labels of its
//! controls and the heading of its group; for a status card, its title,
//! its text and the labels of its buttons. The Python search read an
//! element's `textContent` the same way, so "watch live", "zoom",
//! "dolphin", "make default" and "refresh all" find the settings people
//! mean.

use gtk::prelude::*;

use crate::window::children;

/// The class of a setting that matches the settings search.
const SEARCH_MATCH_CLASS: &str = "search-match";
/// The class of the setting the search jumped to.
const JUMP_TARGET_CLASS: &str = "jump-target";

/// What a setting row says, and the other words it is found by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowText {
    /// The row's name, such as "Text size".
    pub title: &'static str,
    /// The line under the name.
    pub description: &'static str,
    /// Words people search for it by that it does not show, such as
    /// "zoom" for Text size, and the Python app's longer wording.
    pub keywords: &'static str,
}

impl RowText {
    /// The row's own words: its title, description and keywords.
    pub(crate) fn words(&self) -> String {
        [self.title, self.description, self.keywords].join(" ")
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

    /// How a setting that says and shows `texts` stands against the query:
    /// each word may appear in any of them.
    pub(crate) fn find_in(&self, texts: &[&str]) -> Finding {
        if self.is_empty() {
            return Finding::NoSearch;
        }
        let text = texts.join(" ").to_lowercase();
        let has_every_word = self.words.iter().all(|word| text.contains(word.as_str()));
        if has_every_word {
            Finding::Match
        } else {
            Finding::Miss
        }
    }
}

/// How a setting stands against the search typed now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Finding {
    /// Nothing is typed: every setting shows, none is marked.
    NoSearch,
    /// Every typed word is in the setting's text.
    Match,
    /// A typed word is missing, so the setting hides.
    Miss,
}

impl Finding {
    /// Whether the setting shows.
    pub(crate) fn is_shown(self) -> bool {
        self != Finding::Miss
    }

    /// Shows `setting` or hides it as the finding says, marks it while it
    /// matches a search, and forgets an earlier jump to it.
    pub(crate) fn show_on(self, setting: &impl IsA<gtk::Widget>) {
        setting.set_visible(self.is_shown());
        setting.remove_css_class(JUMP_TARGET_CLASS);
        if self == Finding::Match {
            setting.add_css_class(SEARCH_MATCH_CLASS);
        } else {
            setting.remove_css_class(SEARCH_MATCH_CLASS);
        }
    }
}

/// Outlines `setting` as the one the search jumped to, and gives the first
/// of its `controls` that works keyboard focus, as Enter in the Python
/// app's search clicked the first result. False when none works, as on a
/// disabled row.
pub(crate) fn jump_to(setting: &impl IsA<gtk::Widget>, controls: &[gtk::Widget]) -> bool {
    setting.add_css_class(JUMP_TARGET_CLASS);
    for control in controls {
        if control.is_sensitive() && control.grab_focus() {
            return true;
        }
    }
    false
}

/// The text of every label in `widget`, in tree order: what the Python
/// search read as an element's `textContent`. A drop-down's options are in
/// its list, which is a child of its button, so they are found too.
pub(crate) fn shown_text(widget: &impl IsA<gtk::Widget>) -> String {
    let mut texts = Vec::new();
    collect_label_texts(widget.upcast_ref(), &mut texts);
    texts.join(" ")
}

fn collect_label_texts(widget: &gtk::Widget, texts: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        texts.push(label.text().into());
    }
    for child in children(widget) {
        collect_label_texts(&child, texts);
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
            let finding = SearchQuery::parse(case.typed).find_in(&[&TEXT_SIZE.words()]);
            assert_eq!(
                finding == Finding::Match,
                case.finds_text_size,
                "{:?}",
                case.typed
            );
        }
    }

    /// A row's words and the heading of its group are one text, so a query
    /// may take some words from each.
    #[test]
    fn a_query_may_take_its_words_from_the_row_and_its_heading() {
        let query = SearchQuery::parse("refresh folders");
        let found = query.find_in(&[
            "Folders Double-clicking a folder",
            "What opens where Refresh status",
        ]);
        assert_eq!(found, Finding::Match);
        let missing = query.find_in(&["Folders Double-clicking a folder"]);
        assert_eq!(missing, Finding::Miss);
    }

    #[test]
    fn an_empty_query_is_no_search() {
        assert!(SearchQuery::parse("  ").is_empty());
        assert!(!SearchQuery::parse("x").is_empty());
        let finding = SearchQuery::parse("").find_in(&["anything"]);
        assert_eq!(finding, Finding::NoSearch);
        assert!(finding.is_shown());
    }

    #[test]
    fn the_count_reads_as_in_the_python_app() {
        assert_eq!(match_count_text(0), "No matching settings");
        assert_eq!(match_count_text(1), "1 matching setting");
        assert_eq!(match_count_text(4), "4 matching settings");
    }

    /// Ported from `settingsSearch`, which searched an element's
    /// `textContent`: the labels of a button, however deep, are found.
    #[gtk::test]
    fn the_shown_text_is_every_label_in_tree_order() {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.append(&gtk::Label::new(Some("Refresh")));
        content.append(&gtk::Label::new(Some("all")));
        let button = gtk::Button::builder().child(&content).build();
        assert_eq!(shown_text(&button), "Refresh all");
    }
}
