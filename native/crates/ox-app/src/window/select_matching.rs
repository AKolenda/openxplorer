// SPDX-License-Identifier: AGPL-3.0-only
//! Select items matching… (SEL-037): asks for a wildcard pattern such as
//! `*.pdf` or `IMG_2024*` and selects every shown item whose name matches.
//!
//! Dolphin's "Select Items Matching…" (Nautilus has the same). `*` stands
//! for any run of characters and `?` for one character; case is ignored.
//! Only the shown items are searched, so hidden files and a search filter
//! count as they do on screen. A blank pattern selects nothing.

use gtk::glib;
use gtk::prelude::*;

use super::dialog::{ButtonStyle, Dialog};
use super::BrowserWindow;

/// Whether `name` matches `pattern`, with `*` for any run of characters
/// and `?` for exactly one, ignoring case.
fn matches_pattern(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    // Greedy matching that backtracks to the last `*`.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&wanted) if wanted == '?' || wanted == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((star_p, star_n)) => {
                    p = star_p + 1;
                    n = star_n + 1;
                    star = Some((star_p, star_n + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&rest| rest == '*')
}

impl BrowserWindow {
    /// Asks for a pattern, then selects exactly the shown items whose
    /// names match it.
    pub(super) fn ask_to_select_matching(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let dialog = Dialog::new(
                    &window,
                    "Select items matching",
                    "Use * for any characters and ? for one character, such as *.pdf.",
                );
                let field = dialog.add_text_field("Pattern", "*");
                dialog.add_cancel_button();
                dialog.add_button("Select", ButtonStyle::Primary);
                dialog.open();
                let answer = dialog.next_response().await;
                dialog.finish();
                if answer.is_some() {
                    window.select_matching(&field.text());
                }
            }
        ));
    }

    /// Selects exactly the shown items whose names match `pattern`; a
    /// blank pattern selects nothing.
    pub(super) fn select_matching(&self, pattern: &str) {
        let pattern = pattern.trim();
        let model = self.folder_pane().model();
        let matching: Vec<u32> = (0..model.n_items())
            .filter(|&position| {
                !pattern.is_empty()
                    && model
                        .name_at(position)
                        .is_some_and(|name| matches_pattern(pattern, &name))
            })
            .collect();
        model.select_positions(&matching);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SEL-037
    #[test]
    fn wildcards_match_whole_names_ignoring_case() {
        assert!(matches_pattern("*.pdf", "Report.PDF"));
        assert!(matches_pattern("IMG_2024*", "img_20240101.jpg"));
        assert!(matches_pattern("notes ?.txt", "Notes 2.txt"));
        assert!(matches_pattern("*a*b*", "xaYbz"));
        assert!(!matches_pattern("*.pdf", "report.pdf.txt"));
        assert!(!matches_pattern("notes ?.txt", "Notes 10.txt"));
        assert!(!matches_pattern("", "anything"));
    }
}
