// SPDX-License-Identifier: AGPL-3.0-only
//! The characters Windows forbids in a name, refused as they are typed in
//! the field that renames an item in place (OPS-010).
//!
//! As in Windows File Explorer, typing or pasting `\ / : * ? " < > |` in
//! the field inserts nothing and leaves the text and its selection as
//! they were (a key, a paste or an input method that replaced the
//! selection puts it back); a balloon under the field says, in Windows' words, which
//! characters a name can't contain. It goes away after a few seconds or
//! at the next key. Linux would store every one of them but the slash,
//! yet they are refused on every drive, as Windows refuses them: a name
//! with one cannot be copied to a Windows share, an NTFS or exFAT drive or
//! a USB stick as it is (XFER-028). Names that already have one keep it
//! while they are edited, and the Rename dialog still takes them.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::{has_windows_forbidden_character, WINDOWS_FORBIDDEN_CHARACTERS};

use crate::announcement::announce;

/// The balloon's first line, in Windows' words.
const BALLOON_TEXT: &str =
    crate::i18n::message_id("A file name can't contain any of the following characters:");

/// How long the balloon shows when no key is pressed.
const BALLOON_TIMEOUT: Duration = Duration::from_secs(5);

/// The CSS class of the balloon.
const BALLOON_CLASS: &str = "name-balloon";

/// The forbidden characters as the balloon's second line lists them:
/// `\ / : * ? " < > |`.
fn forbidden_characters_line() -> String {
    let characters: Vec<String> = WINDOWS_FORBIDDEN_CHARACTERS.iter().map(char::to_string).collect();
    characters.join(" ")
}

/// True for a key that only changes what the next key types, such as
/// Shift for `?`: it leaves the balloon up.
fn is_modifier_key(key: gdk::Key) -> bool {
    matches!(
        key,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::ISO_Level3_Shift
            | gdk::Key::ISO_Level5_Shift
            | gdk::Key::Caps_Lock
            | gdk::Key::Num_Lock
    )
}

/// True when pressing `key` with `modifiers` types a forbidden character,
/// which the balloon stays up for. Ctrl and Alt make it a shortcut, which
/// types nothing.
fn types_forbidden_character(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    if modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
        return false;
    }
    key.to_unicode()
        .is_some_and(|character| WINDOWS_FORBIDDEN_CHARACTERS.contains(&character))
}

/// The balloon under one name field.
#[derive(Debug, Clone)]
struct Balloon {
    /// The field it points at.
    editor: glib::WeakRef<gtk::Entry>,
    /// The balloon itself, a child of the field.
    popover: gtk::Popover,
    /// The timer that hides it, while it shows.
    timeout: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Balloon {
    /// A hidden balloon under `editor`, which lets it go when it goes.
    fn new(editor: &gtk::Entry) -> Self {
        let text = ox_core::i18n::gettext_static(BALLOON_TEXT);
        let first = gtk::Label::builder().label(text).xalign(0.0).build();
        let characters = gtk::Label::builder()
            .label(forbidden_characters_line())
            .xalign(0.0)
            .css_classes(["name-balloon-characters"])
            .build();
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.append(&first);
        lines.append(&characters);
        // It only tells: the keyboard stays in the field, and typing goes
        // on there while it shows.
        let popover = gtk::Popover::builder()
            .child(&lines)
            .autohide(false)
            .can_focus(false)
            .position(gtk::PositionType::Bottom)
            .css_classes([BALLOON_CLASS])
            .build();
        popover.set_parent(editor);
        editor.connect_destroy(glib::clone!(
            #[strong]
            popover,
            move |_| popover.unparent()
        ));
        let balloon = Self {
            editor: editor.downgrade(),
            popover,
            timeout: Rc::default(),
        };
        editor.connect_unmap(glib::clone!(
            #[strong]
            balloon,
            move |_| balloon.hide()
        ));
        balloon
    }

    /// Shows the balloon for [`BALLOON_TIMEOUT`] from now and reads it to
    /// screen readers.
    fn show(&self) {
        self.stop_timer();
        let popover = self.popover.downgrade();
        let timeout = Rc::downgrade(&self.timeout);
        let source = glib::timeout_add_local_once(BALLOON_TIMEOUT, move || {
            // The timer has fired: it is no longer there to remove.
            if let Some(timeout) = timeout.upgrade() {
                timeout.take();
            }
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
        });
        self.timeout.replace(Some(source));
        if !self.popover.is_visible() {
            self.popover.popup();
        }
        if let Some(editor) = self.editor.upgrade() {
            let message = format!(
                "{} {}",
                ox_core::i18n::gettext_static(BALLOON_TEXT),
                forbidden_characters_line()
            );
            announce(&editor, &message, gtk::AccessibleAnnouncementPriority::High);
        }
    }

    /// Hides the balloon, if it shows.
    fn hide(&self) {
        self.stop_timer();
        if self.popover.is_visible() {
            self.popover.popdown();
        }
    }

    /// Stops the timer that would hide the balloon.
    fn stop_timer(&self) {
        if let Some(source) = self.timeout.take() {
            source.remove();
        }
    }
}

/// Refuses the characters Windows forbids in names as they are typed,
/// pasted or dropped into `editor`, with Windows' balloon. GTK runs the
/// last key handler added first, so call it after the field's others: it
/// then sees every key first.
pub(super) fn refuse_forbidden_characters(editor: &gtk::Entry) {
    let balloon = Balloon::new(editor);
    // The next key hides the balloon. Every key still reaches the field
    // and its input method, so Compose and dead-key sequences that start
    // with a forbidden character work; what they insert is checked below.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[strong]
        balloon,
        move |_, key, _, modifiers| {
            if !types_forbidden_character(key, modifiers) && !is_modifier_key(key) {
                balloon.hide();
            }
            glib::Propagation::Proceed
        }
    ));
    editor.add_controller(keys);
    let Some(text) = editor.delegate().and_downcast::<gtk::Text>() else {
        return;
    };
    // Typing, a paste or an input method replaces the selection before it
    // inserts: what a change deletes is kept until the change ends, so a
    // refused insertion gives the text and selection back.
    let before_change: Rc<RefCell<Option<FieldState>>> = Rc::default();
    let restoring = Rc::new(std::cell::Cell::new(false));
    text.connect_delete_text(glib::clone!(
        #[strong]
        before_change,
        move |text, _, _| {
            if before_change.borrow().is_none() {
                before_change.replace(Some(FieldState::of(text)));
            }
        }
    ));
    text.connect_changed(glib::clone!(
        #[strong]
        before_change,
        move |_| {
            before_change.take();
        }
    ));
    text.connect_insert_text(move |text, inserted, _| {
        if restoring.get() || !has_windows_forbidden_character(inserted) {
            return;
        }
        text.stop_signal_emission_by_name("insert-text");
        balloon.show();
        if let Some(state) = before_change.take() {
            // Once GTK has finished the change it is in.
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                text,
                #[strong]
                restoring,
                move || {
                    restoring.set(true);
                    state.restore(&text);
                    restoring.set(false);
                }
            ));
        }
    });
}

/// A name field's text and selection, to give back.
#[derive(Debug)]
struct FieldState {
    /// The text.
    text: glib::GString,
    /// The selection's ends, or the cursor at both ends.
    selection: (i32, i32),
}

impl FieldState {
    /// What `text` holds now.
    fn of(text: &gtk::Text) -> Self {
        let cursor = text.position();
        Self {
            text: text.text(),
            selection: text.selection_bounds().unwrap_or((cursor, cursor)),
        }
    }

    /// Puts it back into `text`.
    fn restore(&self, text: &gtk::Text) {
        text.set_text(&self.text);
        text.select_region(self.selection.0, self.selection.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-010
    #[test]
    fn the_balloon_lists_windows_forbidden_characters_in_its_order() {
        assert_eq!(forbidden_characters_line(), "\\ / : * ? \" < > |");
        let none = gdk::ModifierType::empty();
        let shift = gdk::ModifierType::SHIFT_MASK;
        assert!(types_forbidden_character(gdk::Key::colon, shift));
        assert!(types_forbidden_character(gdk::Key::slash, none));
        assert!(types_forbidden_character(gdk::Key::backslash, none));
        assert!(types_forbidden_character(gdk::Key::question, shift));
        assert!(!types_forbidden_character(gdk::Key::a, none));
        assert!(!types_forbidden_character(gdk::Key::parenleft, shift));
        assert!(
            !types_forbidden_character(gdk::Key::slash, gdk::ModifierType::CONTROL_MASK),
            "Ctrl+/ is a shortcut"
        );
        assert!(is_modifier_key(gdk::Key::Shift_L));
        assert!(!is_modifier_key(gdk::Key::BackSpace));
    }
}
