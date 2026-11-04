// SPDX-License-Identifier: AGPL-3.0-only
//! "Replace or Skip Files": what an action asks, in place of its
//! progress, when the copy or move meets names already taken (OPS-026,
//! OPS-028).
//!
//! Windows 8 and later ask inside the progress window, not in a dialog of
//! their own over the browsing window ("Improving our file management
//! basics", Building Windows 8, 2011), so every other action and window
//! goes on meanwhile. The action keeps its description ("Copying 1 item
//! from Downloads to Desktop", with the folders as links) and says what it
//! found, in Windows' words:
//!
//! | Conflicts | Says | Rows |
//! |---|---|---|
//! | 1 | The destination already has a file named "report.png" | Replace the file in the destination · Skip this file · Compare info for both files |
//! | N | The destination has N files with the same names | Replace the files in the destination · Skip these files · Let me decide for each file |
//!
//! A file meeting a folder of its name says "a folder named". The last row
//! opens the File Conflict window ([`FileConflictWindow`]), whose Cancel
//! stops the action, as in Windows. The action's Cancel stays beside the
//! question, and stops it too; nothing has changed then. Each row is a
//! button named by its text, reachable with Tab, with the glyph Windows
//! shows: a check mark, a turning arrow and two documents.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::ops::{ConflictScan, Resolution};

use super::TransferPanel;
use crate::icons::{self, Icon};
use crate::window::conflict_window::FileConflictWindow;

/// The edge of the rows' glyphs.
const ROW_GLYPH_SIZE: i32 = 20;

/// Which row the user chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row {
    /// Replace the file(s) in the destination.
    Replace,
    /// Skip this file / these files.
    Skip,
    /// Compare info for both files / Let me decide for each file.
    Decide,
}

/// The texts of the question about `scan`'s conflicts: the summary, then
/// the three rows.
pub(super) fn question_texts(scan: &ConflictScan) -> [String; 4] {
    use ox_core::i18n::{format_message, gettext};
    let count = scan.conflicts.len();
    if let [conflict] = scan.conflicts.as_slice() {
        let name = conflict.name();
        let summary = if conflict.existing.is_folder {
            format_message(
                "The destination already has a folder named \"{name}\"",
                &[("name", &name)],
            )
        } else {
            format_message(
                "The destination already has a file named \"{name}\"",
                &[("name", &name)],
            )
        };
        return [
            summary,
            gettext("Replace the file in the destination"),
            gettext("Skip this file"),
            gettext("Compare info for both files"),
        ];
    }
    let count = u64::try_from(count).unwrap_or(u64::MAX);
    [
        ox_core::i18n::ngettext(
            "The destination has {count} file with the same name",
            "The destination has {count} files with the same names",
            count,
        )
        .replace("{count}", &super::rate::grouped(count)),
        gettext("Replace the files in the destination"),
        gettext("Skip these files"),
        gettext("Let me decide for each file"),
    ]
}

impl TransferPanel {
    /// Sets the rows' glyphs and buttons; called once the template is
    /// built.
    pub(super) fn connect_question(&self) {
        let imp = self.imp();
        icons::set_icon(&imp.replace_icon, Icon::Checkmark, ROW_GLYPH_SIZE);
        icons::set_icon(&imp.skip_icon, Icon::ArrowUndo, ROW_GLYPH_SIZE);
        icons::set_icon(&imp.decide_icon, Icon::DocumentCopy, ROW_GLYPH_SIZE);
        for (button, row) in [
            (&*imp.replace_button, Row::Replace),
            (&*imp.skip_button, Row::Skip),
            (&*imp.decide_button, Row::Decide),
        ] {
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = panel)]
                self,
                move |_| panel.answer(Some(row))
            ));
        }
    }

    /// Asks about `scan`'s conflicts in place of the progress, and waits:
    /// an answer per conflict, or `None` when the user cancelled.
    pub(crate) async fn ask_about_conflicts(&self, scan: &ConflictScan) -> Option<Vec<Resolution>> {
        let count = scan.conflicts.len();
        let answers = match self.ask_row(scan).await {
            None => None,
            Some(Row::Replace) => Some(vec![Resolution::Replace; count]),
            Some(Row::Skip) => Some(vec![Resolution::Skip; count]),
            Some(Row::Decide) => self.decide_each(scan).await,
        };
        self.end_question();
        answers
    }

    /// Opens the File Conflict window and waits for its answers; Cancel
    /// there stops the whole action, as in Windows.
    async fn decide_each(&self, scan: &ConflictScan) -> Option<Vec<Resolution>> {
        let window = self.imp().window.upgrade()?;
        let source = self.source_folder();
        let conflicts = FileConflictWindow::new(&window, self, scan, &source, &scan.destination_folder);
        self.imp().conflict_window.set(Some(&conflicts));
        let answers = conflicts.answers().await;
        self.imp().conflict_window.set(None);
        if answers.is_none() {
            self.cancel();
        }
        answers
    }

    /// True while the action asks.
    pub(crate) fn is_asking(&self) -> bool {
        self.imp().question.get_visible()
    }

    /// Shows the question and waits for a row; `None` once cancelled.
    async fn ask_row(&self, scan: &ConflictScan) -> Option<Row> {
        let imp = self.imp();
        let [summary, replace, skip, decide] = question_texts(scan);
        imp.percent_label.set_text(&summary);
        imp.percent_label.set_wrap(true);
        imp.percent_label.set_ellipsize(gtk::pango::EllipsizeMode::None);
        imp.replace_label.set_text(&replace);
        imp.skip_label.set_text(&skip);
        imp.decide_label.set_text(&decide);
        imp.pause_button.set_visible(false);
        imp.progress_bar.set_visible(false);
        imp.details.set_visible(false);
        imp.question.set_visible(true);
        self.add_css_class("asking");
        let (sender, receiver) = async_channel::bounded(1);
        imp.question_answer.replace(Some(sender));
        if imp
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(ox_core::transfer::Cancellation::is_cancelled)
        {
            self.answer(None);
        }
        if let Some(window) = imp.window.upgrade() {
            window.show_question();
        }
        receiver.recv().await.ok().flatten()
    }

    /// Answers the question waiting, if any, with `row`; `None` cancels.
    pub(super) fn answer(&self, row: Option<Row>) {
        if let Some(sender) = self.imp().question_answer.take() {
            // The receiver waits for exactly one answer.
            let _ = sender.try_send(row);
        }
    }

    /// While the action asks, its Cancel shows only beside other actions:
    /// alone, as in Windows, the window's own close button cancels it.
    pub(crate) fn show_cancel_beside_others(&self, others: bool) {
        self.imp().cancel_button.set_visible(others || !self.is_asking());
    }

    /// The question ends: the progress shows again.
    fn end_question(&self) {
        let imp = self.imp();
        imp.cancel_button.set_visible(true);
        imp.question_answer.replace(None);
        imp.question.set_visible(false);
        self.remove_css_class("asking");
        imp.percent_label.set_wrap(false);
        imp.percent_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        imp.pause_button
            .set_visible(imp.kind.get() == Some(super::TransferKind::Files));
        let details = imp.window.upgrade().is_some_and(|window| window.shows_details());
        self.show_details(details);
        self.show_percent();
        self.refresh_window();
    }

    /// Stops asking because the action was cancelled: the question, and
    /// the File Conflict window it opened, close unanswered.
    pub(super) fn stop_asking(&self) {
        self.answer(None);
        if let Some(conflicts) = self.imp().conflict_window.upgrade() {
            conflicts.cancel();
        }
    }

    /// The folder the action's items come from, as it describes it.
    fn source_folder(&self) -> String {
        self.imp()
            .action
            .borrow()
            .as_ref()
            .map(|action| action.source.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use ox_core::ops::{ItemFacts, NameConflict};
    use ox_core::transfer::TransferMode;

    use super::*;

    fn scan(names: &[&str], existing_is_folder: bool) -> ConflictScan {
        ConflictScan {
            mode: TransferMode::Copy,
            destination_folder: "file:///home/sample/Desktop".into(),
            selection: Vec::new(),
            items: Vec::new(),
            conflicts: names
                .iter()
                .map(|name| NameConflict {
                    incoming_uri: format!("file:///home/sample/Downloads/{name}"),
                    existing_uri: format!("file:///home/sample/Desktop/{name}"),
                    incoming: ItemFacts::default(),
                    existing: ItemFacts {
                        is_folder: existing_is_folder,
                        ..ItemFacts::default()
                    },
                })
                .collect(),
            merged_folders: Vec::new(),
        }
    }

    /// The question takes Windows' singular and plural wording.
    ///
    /// parity: OPS-026, OPS-028
    #[test]
    fn the_question_takes_windows_wording() {
        assert_eq!(
            question_texts(&scan(&["report.png"], false)),
            [
                "The destination already has a file named \"report.png\"",
                "Replace the file in the destination",
                "Skip this file",
                "Compare info for both files",
            ]
        );
        assert_eq!(
            question_texts(&scan(&["Holiday"], true))[0],
            "The destination already has a folder named \"Holiday\""
        );
        let many: Vec<String> = (0..1200).map(|index| format!("photo {index}.jpg")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            question_texts(&scan(&many, false)),
            [
                "The destination has 1,200 files with the same names",
                "Replace the files in the destination",
                "Skip these files",
                "Let me decide for each file",
            ]
        );
    }
}
