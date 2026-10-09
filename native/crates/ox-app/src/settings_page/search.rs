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
//!
//! The words people type are often not the ones a setting uses, so a typed
//! word also matches the words of its group in [`SYNONYMS`] ("chevron"
//! finds the expand arrows, "dark mode" finds Theme), and a word matches
//! the same word with another ending ("arrow" finds "arrows", "hiding"
//! finds "hide"). A synonym is looked for only in what names a setting
//! (its title, keywords, control labels and heading), not in its longer
//! details, where common words would find settings that only mention
//! them. This finds more, never less: a setting the literal words found
//! is still found.

use gtk::prelude::*;
use ox_core::i18n::{gettext, ngettext};

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
    #[cfg(test)]
    pub(crate) fn words(&self) -> String {
        [self.title, self.description, self.keywords]
            .map(gettext)
            .join(" ")
    }

    /// The words that name the row: its title and keywords, where a
    /// synonym of a typed word may be found.
    pub(crate) fn name_words(&self) -> String {
        [self.title, self.keywords].map(gettext).join(" ")
    }

    /// What the row says about itself: its description, where only the
    /// words typed are looked for.
    pub(crate) fn detail_words(&self) -> String {
        gettext(self.description)
    }
}

/// Words people use for the same thing, each group's words standing for
/// one another in a search. A typed word in a group also matches the
/// group's other words; a word of two parts ("dark mode") is typed as one.
/// English only, as the settings' own words are: a translation finds its
/// own words. Kept to words that mean one thing here, so a search does
/// not fill with settings that only share a word.
const SYNONYMS: &[&[&str]] = &[
    // Theme.
    &["dark", "dark mode", "dark theme", "night mode", "night theme"],
    &["light mode", "light theme", "day mode"],
    &["theme", "colour scheme", "color scheme"],
    &["colour", "color"],
    // Text size, the desktop font and compact view.
    &["zoom", "magnify", "magnification", "enlarge"],
    &["font", "typeface"],
    &["compact", "dense", "density", "condensed"],
    // The sidebar's and Details' expand arrows.
    &[
        "arrow",
        "chevron",
        "expander",
        "triangle",
        "disclosure",
        "caret",
        "twisty",
    ],
    &["tree", "hierarchy", "subfolder"],
    &[
        "sidebar",
        "navigation pane",
        "nav pane",
        "side panel",
        "left pane",
    ],
    // Previews.
    &["preview", "thumbnail"],
    &["picture", "image", "photo"],
    &["video", "movie", "film"],
    // Confirmations.
    &["confirm", "confirmation", "prompt", "warning", "popup", "pop-up"],
    &["recycle bin", "trash", "wastebasket", "rubbish bin"],
    &["delete", "erase"],
    // Running programs.
    &["run", "execute", "launch"],
    &["program", "executable", "binary"],
    // Tabs, windows and panes.
    &["split", "dual pane", "two panes", "side by side", "commander"],
    &["breadcrumb", "crumb", "path bar"],
    &["drag", "drag and drop", "dnd"],
    &["reopen", "restore tabs", "remember tabs"],
    &["startup", "start up", "login", "logon", "boot"],
    // Archives and default apps.
    &["zip", "archive", "compressed", "rar", "7z"],
    &[
        "default app",
        "default file manager",
        "file association",
        "open with",
    ],
    &[
        "file picker",
        "file chooser",
        "save as",
        "open dialog",
        "save dialog",
    ],
    // Search and indexing.
    &["index", "search cache"],
    &["watch", "monitor", "inotify", "auto refresh"],
    &["network", "smb", "samba", "nas", "remote"],
    // Folder sizes.
    &["folder size", "disk usage", "du"],
];

/// The search key of `word`: lower case, with a plural or verb ending
/// taken off, and then a final silent "e", so that "arrows" meets
/// "arrow" and "hiding", "hides" and "hide" meet "hid". Short words keep
/// their endings, which are part of them ("bus", "its"), and so do the
/// words in [`KEEP_WHOLE`].
fn stem(word: &str) -> String {
    let word = word.to_lowercase();
    if KEEP_WHOLE.contains(&word.as_str()) {
        return word;
    }
    let mut root = word.clone();
    for (ending, keep) in [("ies", "y"), ("ing", ""), ("ed", ""), ("es", ""), ("s", "")] {
        if let Some(rest) = word.strip_suffix(ending) {
            if rest.chars().count() >= 3 && !word.ends_with("ss") {
                root = format!("{rest}{keep}");
                break;
            }
        }
    }
    match root.strip_suffix('e') {
        Some(rest) if rest.chars().count() >= 3 => rest.to_owned(),
        _ => root,
    }
}

/// Words whose ending is no plural or verb form, which would otherwise
/// meet a word of another meaning: "spacing" (room between rows) is not
/// "space" (room on a disk).
const KEEP_WHOLE: &[&str] = &["spacing", "padding", "setting", "settings", "thing", "nothing"];

/// The stems of every word of `text`, in order.
fn stems_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(stem)
        .collect()
}

/// Whether the stems of `phrase` appear in `stems` one after the other.
fn contains_phrase(stems: &[String], phrase: &[String]) -> bool {
    !phrase.is_empty() && stems.windows(phrase.len()).any(|window| window == phrase)
}

/// One thing typed into "Search settings": a word, or a word of two parts
/// from [`SYNONYMS`], with the phrases it may be found by.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Term {
    /// What was typed, in lower case, each of its words found anywhere in
    /// a setting's text as before: "brav" still finds Brave, and "light
    /// mode" finds a setting with "light" and "mode" apart.
    typed: Vec<String>,
    /// The stems of it and of its synonyms, each one or more words.
    alternatives: Vec<Vec<String>>,
}

impl Term {
    fn new(typed: &str) -> Self {
        let typed = typed.to_lowercase();
        let own = stems_of(&typed);
        let words = typed.split_whitespace().map(str::to_owned).collect();
        let mut alternatives = vec![own.clone()];
        for group in SYNONYMS {
            if group.iter().any(|word| stems_of(word) == own) {
                for word in *group {
                    let stems = stems_of(word);
                    if !alternatives.contains(&stems) {
                        alternatives.push(stems);
                    }
                }
            }
        }
        Self {
            typed: words,
            alternatives,
        }
    }

    /// Whether a setting has this term: what was typed anywhere in its
    /// `text`, or a synonym among the stems of what names it, `names`.
    fn is_in(&self, text: &str, names: &[String]) -> bool {
        self.typed.iter().all(|word| text.contains(word.as_str()))
            || self
                .alternatives
                .iter()
                .any(|phrase| contains_phrase(names, phrase))
    }
}

/// The words typed into "Search settings", in lower case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SearchQuery {
    terms: Vec<Term>,
}

impl SearchQuery {
    /// The query `typed` asks for: its words, ignoring case and spacing,
    /// two words that are one in [`SYNONYMS`] ("dark mode") taken as one.
    pub(crate) fn parse(typed: &str) -> Self {
        let words: Vec<&str> = typed.split_whitespace().collect();
        let mut terms = Vec::new();
        let mut index = 0;
        while index < words.len() {
            let pair = words
                .get(index + 1)
                .map(|next| format!("{} {next}", words[index]));
            let is_phrase = pair.as_deref().is_some_and(|pair| {
                let stems = stems_of(pair);
                SYNONYMS
                    .iter()
                    .flat_map(|group| group.iter())
                    .any(|word| stems_of(word) == stems)
            });
            if let (true, Some(pair)) = (is_phrase, pair) {
                terms.push(Term::new(&pair));
                index += 2;
            } else {
                terms.push(Term::new(words[index]));
                index += 1;
            }
        }
        Self { terms }
    }

    /// True when nothing is typed, so every setting shows.
    pub(crate) fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// How a setting that says and shows `texts` stands against the query,
    /// when all of them name it: each word, or one of its synonyms, may
    /// appear in any of them.
    #[cfg(test)]
    pub(crate) fn find_in(&self, texts: &[&str]) -> Finding {
        self.find_in_parts(texts, &[])
    }

    /// How a setting stands against the query, `names` being what names it
    /// and `details` what it says at length: each word may appear in
    /// either, a synonym of it only in `names`.
    pub(crate) fn find_in_parts(&self, names: &[&str], details: &[&str]) -> Finding {
        if self.is_empty() {
            return Finding::NoSearch;
        }
        let name_text = names.join(" ").to_lowercase();
        let text = format!("{name_text} {}", details.join(" ").to_lowercase());
        let name_stems = stems_of(&name_text);
        if self.terms.iter().all(|term| term.is_in(&text, &name_stems)) {
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
    if count == 0 {
        gettext("No matching settings")
    } else {
        ngettext(
            "{count} matching setting",
            "{count} matching settings",
            count as u64,
        )
        .replace("{count}", &count.to_string())
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

    /// A typed word also matches the words people use for the same thing,
    /// a word of two parts is taken as one, and endings do not matter.
    #[test]
    fn a_word_finds_its_synonyms_and_other_endings() {
        let arrows = "Hide expand arrows in the sidebar";
        let theme = "Theme System Light Dark";
        let cases = [
            ("chevron", arrows, true),
            ("chevrons", arrows, true),
            ("arrow", arrows, true),
            ("triangle", arrows, true),
            ("navigation pane arrows", arrows, true),
            ("hiding arrows", arrows, true),
            ("dark mode", theme, true),
            ("night mode", theme, true),
            ("DARK   Mode", theme, true),
            ("theme", theme, true),
            ("dark mode", arrows, false),
            ("chevron", theme, false),
        ];
        for (typed, text, found) in cases {
            let finding = SearchQuery::parse(typed).find_in(&[text]);
            assert_eq!(finding == Finding::Match, found, "{typed:?} in {text:?}");
        }
    }

    /// "night mode" is one term, found by its synonyms; its words are
    /// still found apart, as before: "light mode" finds a setting that
    /// says "light" and "mode" in different places.
    #[test]
    fn two_words_that_are_one_term_are_read_together() {
        assert_eq!(SearchQuery::parse("dark mode").terms.len(), 1);
        assert_eq!(SearchQuery::parse("dark arrows").terms.len(), 2);
        assert_eq!(SearchQuery::parse("mode").find_in(&["Theme Dark"]), Finding::Miss);
        assert_eq!(
            SearchQuery::parse("night mode").find_in(&["Theme Dark"]),
            Finding::Match
        );
        assert_eq!(
            SearchQuery::parse("light mode").find_in(&["Theme System Light Dark", "colour mode"]),
            Finding::Match
        );
    }

    /// Endings come off only where enough of the word is left, so short
    /// words and words ending in "ss" keep their meaning.
    #[test]
    fn endings_come_off_only_long_enough_words() {
        assert_eq!(stem("arrows"), "arrow");
        assert_eq!(stem("folders"), "folder");
        assert_eq!(stem("hiding"), "hid");
        assert_eq!(stem("hide"), "hid");
        assert_eq!(stem("hides"), "hid");
        assert_eq!(stem("theme"), "them");
        assert_eq!(stem("themes"), "them");
        assert_eq!(stem("libraries"), "library");
        assert_eq!(stem("access"), "access");
        assert_eq!(stem("bus"), "bus");
        assert_eq!(stem("tabs"), "tab");
        assert_eq!(stem("its"), "its");
        assert_ne!(stem("spacing"), stem("space"));
    }

    /// The literal search still works as before: part of a word is found.
    #[test]
    fn part_of_a_word_still_finds_it() {
        let finding = SearchQuery::parse("brav").find_in(&["Brave saves to my Linux Downloads folder"]);
        assert_eq!(finding, Finding::Match);
    }

    /// No word is in two groups that mean different things, which would
    /// make a search find settings it should not.
    #[test]
    fn each_synonym_belongs_to_one_meaning() {
        let mut seen = std::collections::HashMap::new();
        // Words that are honestly two things in a file manager: none yet.
        let shared: [&str; 0] = [];
        for (index, group) in SYNONYMS.iter().enumerate() {
            assert!(group.len() > 1, "group {index} has one word");
            for word in *group {
                let key = stems_of(word);
                if let Some(other) = seen.insert(key.clone(), index) {
                    assert_ne!(other, index, "{word:?} is twice in group {index}");
                    assert!(shared.contains(word), "{word:?} is in groups {other} and {index}");
                }
            }
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
