// SPDX-License-Identifier: AGPL-3.0-only
//! Confirm Attribute Changes: the question Windows asks before Read-only
//! changes on a folder, whether the change stops at the folder or goes on
//! into its subfolders and files (PROP-003).
//!
//! On Linux a folder's own Read-only changes nothing, as "Only applies to
//! files in folder" says, so "this folder only" leaves the folder and
//! everything inside it as they are, and only "subfolders and files"
//! makes the files inside read-only or writable again. The second answer
//! starts chosen, as in Windows; Cancel, Escape or closing makes no change
//! at all. The wording is Windows 11's ("set read-only", "unset
//! read-only"). Windows greys "this folder only" out for Read-only; here it
//! stays available, as it is the one way to change the box without
//! touching a single file inside, and it changes nothing.

use gtk::prelude::*;

use crate::dialog::Dialog;
use crate::window::ButtonStyle;

/// The dialog's caption.
pub(crate) const TITLE: &str = crate::i18n::message_id("Confirm Attribute Changes");
/// What the dialog says first.
const CHOSEN: &str = crate::i18n::message_id("You have chosen to make the following attribute changes:");
/// The change, when Read-only is ticked.
pub(crate) const MAKE_READ_ONLY: &str = crate::i18n::message_id("set read-only");
/// The change, when Read-only is cleared.
pub(crate) const CLEAR_READ_ONLY: &str = crate::i18n::message_id("unset read-only");
/// The question for one folder.
const ASK_FOLDER: &str = crate::i18n::message_id("Do you want to apply this change to this folder only, or do you want to apply it to all subfolders and files as well?");
/// The question for several items.
const ASK_SELECTION: &str = crate::i18n::message_id("Do you want to apply this change to the selected items only, or do you want to apply it to all subfolders and files as well?");
/// The answer that stops at one folder.
pub(crate) const FOLDER_ONLY: &str = crate::i18n::message_id("Apply changes to this folder only");
/// The answer that goes into one folder.
pub(crate) const FOLDER_AND_CONTENT: &str =
    crate::i18n::message_id("Apply changes to this folder, subfolders and files");
/// The answer that stops at the selected items.
pub(crate) const SELECTION_ONLY: &str = crate::i18n::message_id("Apply changes to the selected items only");
/// The answer that goes into the selected folders.
pub(crate) const SELECTION_AND_CONTENT: &str =
    crate::i18n::message_id("Apply changes to the selected items, subfolders and files");

/// Asks over `parent` whether making the items read-only (`read_only`) or
/// writable again reaches into their folders: `Some(true)` for
/// subfolders and files, `Some(false)` for the items alone, `None` when
/// the question is cancelled. `several` words it for a selection.
pub(crate) async fn ask_reach(
    parent: &impl IsA<gtk::Window>,
    read_only: bool,
    several: bool,
) -> Option<bool> {
    let gettext = ox_core::i18n::gettext_static;
    let dialog = Dialog::with_title_bar(parent, gettext(TITLE), gettext(CHOSEN));
    let change = gtk::Label::builder()
        .label(gettext(if read_only {
            MAKE_READ_ONLY
        } else {
            CLEAR_READ_ONLY
        }))
        .xalign(0.0)
        .margin_start(24)
        .build();
    dialog.add_field(&change);
    let question = gtk::Label::builder()
        .label(gettext(if several { ASK_SELECTION } else { ASK_FOLDER }))
        .wrap(true)
        .xalign(0.0)
        .build();
    dialog.add_field(&question);
    let (only, inside) = if several {
        (SELECTION_ONLY, SELECTION_AND_CONTENT)
    } else {
        (FOLDER_ONLY, FOLDER_AND_CONTENT)
    };
    let only = gtk::CheckButton::with_label(gettext(only));
    let inside = gtk::CheckButton::with_label(gettext(inside));
    inside.set_group(Some(&only));
    inside.set_active(true);
    let choices = gtk::Box::new(gtk::Orientation::Vertical, 4);
    choices.append(&only);
    choices.append(&inside);
    choices.update_relation(&[gtk::accessible::Relation::LabelledBy(&[question.upcast_ref()])]);
    dialog.add_field(&choices);
    let ok = dialog.add_button(&ox_core::i18n::gettext("OK"), ButtonStyle::Accent);
    dialog.add_cancel_button();
    dialog.open();
    let answer = dialog.next_response().await;
    let reaches_inside = inside.is_active();
    if answer.is_some() {
        dialog.finish();
    }
    (answer == Some(ok)).then_some(reaches_inside)
}
