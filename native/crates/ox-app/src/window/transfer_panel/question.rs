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
//! opens the File Conflict window ([`FileConflictWindow`]). Its Cancel,
//! close button and Escape close only that window, and the same question
//! shows again, unanswered, as in Windows 11. The action's Cancel stays
//! beside the question and stops the action; nothing has changed then. Each row is a
//! button named by its text, reachable with Tab, with the glyph Windows
//! shows: a check mark, a turning arrow and two documents.
//!
//! While it asks, the window's "More details" shows greyed and the
//! details stay hidden, whatever was remembered, as in Windows 11: the
//! files are compared only in the File Conflict window.

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

/// What the action says when the destination's drive is full, in
/// Windows' words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpaceTexts {
    /// "There is not enough space on USB Drive."
    pub(crate) summary: String,
    /// "4.4 GB is needed to copy this item. Delete or move files so you
    /// have enough space."
    pub(crate) needed: String,
    /// "Space free: 3.8 GB" and "Total size: 7.5 GB", each when known.
    pub(crate) drive: Vec<String>,
}

impl TransferPanel {
    /// Sets the rows' glyphs and buttons; called once the template is
    /// built.
    pub(super) fn connect_question(&self) {
        let imp = self.imp();
        for (button, retry) in [
            (&*imp.space_retry_button, true),
            (&*imp.space_cancel_button, false),
        ] {
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = panel)]
                self,
                move |_| panel.answer_space(retry)
            ));
        }
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
        let answers = loop {
            match self.ask_row(scan).await {
                None => break None,
                Some(Row::Replace) => break Some(vec![Resolution::Replace; count]),
                Some(Row::Skip) => break Some(vec![Resolution::Skip; count]),
                Some(Row::Decide) => {
                    if let Some(answers) = self.decide_each(scan).await {
                        break Some(answers);
                    }
                    if self.is_cancelled_now() {
                        break None;
                    }
                    // Closed unanswered: the same question asks again.
                }
            }
        };
        self.end_question();
        answers
    }

    /// Opens the File Conflict window and waits for its answers. Its
    /// Cancel, close button and Escape close only that window, unanswered
    /// (`None`), and the action goes back to its question, as in Windows
    /// 11, still waiting.
    async fn decide_each(&self, scan: &ConflictScan) -> Option<Vec<Resolution>> {
        let window = self.imp().window.upgrade()?;
        let source = self.source_folder();
        let conflicts = FileConflictWindow::new(&window, self, scan, &source, &scan.destination_folder);
        self.imp().conflict_window.set(Some(&conflicts));
        let answers = conflicts.answers().await;
        self.imp().conflict_window.set(None);
        answers
    }

    /// Whether the action was cancelled, by its Cancel or the progress
    /// window's close button.
    fn is_cancelled_now(&self) -> bool {
        self.imp()
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(ox_core::transfer::Cancellation::is_cancelled)
    }

    /// Asks, in place of the progress, about a drive too full for what
    /// the action writes, and waits: true for Try Again, false for Cancel
    /// or once the action was cancelled.
    pub(crate) async fn ask_about_space(&self, texts: &SpaceTexts) -> bool {
        let imp = self.imp();
        self.show_asking(&texts.summary);
        imp.space_label.set_text(&texts.needed);
        let drive = texts.drive.join("\n");
        imp.space_drive_label.set_text(&drive);
        imp.space_drive_label.set_visible(!drive.is_empty());
        imp.space_question.set_visible(true);
        let (sender, receiver) = async_channel::bounded(1);
        imp.space_answer.replace(Some(sender));
        if imp
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(ox_core::transfer::Cancellation::is_cancelled)
        {
            self.answer_space(false);
        }
        if let Some(window) = imp.window.upgrade() {
            window.show_question();
        }
        let retry = receiver.recv().await.unwrap_or(false);
        self.end_question();
        retry
    }

    /// Answers the question about a full drive, if one waits.
    fn answer_space(&self, retry: bool) {
        if let Some(sender) = self.imp().space_answer.take() {
            // The receiver waits for exactly one answer.
            let _ = sender.try_send(retry);
        }
    }

    /// True while the action asks about taken names or a full drive.
    pub(crate) fn is_asking(&self) -> bool {
        let imp = self.imp();
        imp.question.get_visible() || imp.space_question.get_visible() || self.asks_about_failure()
    }

    /// What the action says about a full drive, for tests.
    #[cfg(test)]
    pub(crate) fn space_texts_shown(&self) -> (String, String) {
        let imp = self.imp();
        (imp.percent_label.text().into(), imp.space_label.text().into())
    }

    /// Presses Try Again (true) or Cancel in the question about a full
    /// drive, for tests.
    #[cfg(test)]
    pub(crate) fn press_space_answer(&self, retry: bool) {
        let imp = self.imp();
        let button = if retry {
            &imp.space_retry_button
        } else {
            &imp.space_cancel_button
        };
        button.emit_clicked();
    }

    /// True while the action asks Replace or Skip Files.
    pub(crate) fn asks_about_names(&self) -> bool {
        self.imp().question.get_visible()
    }

    /// True while a row of Replace or Skip Files answers, for tests.
    #[cfg(test)]
    pub(crate) fn waits_for_a_row(&self) -> bool {
        self.asks_about_names() && self.imp().question_answer.borrow().is_some()
    }

    /// True while the action asks about a full drive.
    pub(crate) fn asks_about_space(&self) -> bool {
        self.imp().space_question.get_visible()
    }

    /// Says `summary` in the percentage's place, wrapping, and hides the
    /// bar, the details and Pause, as the action does while it asks.
    pub(super) fn show_asking(&self, summary: &str) {
        let imp = self.imp();
        imp.percent_label.set_text(summary);
        imp.percent_label.set_wrap(true);
        imp.percent_label.set_ellipsize(gtk::pango::EllipsizeMode::None);
        imp.pause_button.set_visible(false);
        imp.progress_bar.set_visible(false);
        imp.details.set_visible(false);
        self.add_css_class("asking");
    }

    /// Shows the question and waits for a row; `None` once cancelled.
    async fn ask_row(&self, scan: &ConflictScan) -> Option<Row> {
        let imp = self.imp();
        let [summary, replace, skip, decide] = question_texts(scan);
        self.show_asking(&summary);
        imp.replace_label.set_text(&replace);
        imp.skip_label.set_text(&skip);
        imp.decide_label.set_text(&decide);
        // The details stay hidden, and the window greys its toggle.
        imp.question.set_visible(true);
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
    pub(super) fn end_question(&self) {
        self.remove_failure_question();
        let imp = self.imp();
        imp.cancel_button.set_visible(true);
        imp.question_answer.replace(None);
        imp.space_answer.replace(None);
        imp.question.set_visible(false);
        imp.space_question.set_visible(false);
        self.remove_css_class("asking");
        imp.percent_label.set_wrap(false);
        imp.percent_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        imp.pause_button
            .set_visible(imp.kind.get() == Some(super::TransferKind::Files));
        let details = imp.window.upgrade().is_some_and(|window| window.shows_details());
        self.show_details(details);
        self.show_details_text();
        self.show_percent();
        self.refresh_window();
    }

    /// Stops asking because the action was cancelled: the question, and
    /// the File Conflict window it opened, close unanswered.
    pub(super) fn stop_asking(&self) {
        self.answer(None);
        self.answer_space(false);
        self.answer_failure(super::failure::FailureChoice::Cancel);
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
