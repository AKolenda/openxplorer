// SPDX-License-Identifier: AGPL-3.0-only
//! The app's modal dialog: a title, a message, fields, an error line and
//! buttons.
//!
//! Ports `showModal`, `showMessage` and `textField` of `desktop/ui/app.js`
//! with `.modal` of `desktop/ui/style.css`, refined to the `ContentDialog`
//! of `WinUI` (`native/docs/ui-spec.md` §4.11). The behaviour follows
//! `showModal` and ACC-004:
//!
//! - The first text field has focus with its text selected; without one,
//!   the first button (Cancel) has it, so Enter never confirms a
//!   destructive question by accident.
//! - Enter in a text field presses the primary button.
//! - Escape, the Cancel button and closing the window answer "cancelled";
//!   while the answer is carried out ([`Dialog::set_busy`]) the buttons
//!   are disabled and closing also cancels the running operation, so a
//!   stalled share cannot hold the dialog open.
//! - An error stays inside the dialog, which stays open for another try
//!   ([`Dialog::show_error`]).
//! - Opening a dialog closes the browser window's menus and ends its
//!   type-to-select prefix; closing it gives keyboard focus back to the
//!   control that had it (ACC-005), as GTK keeps a window's focus widget
//!   while a modal dialog covers it.
//!
//! [`Dialog`] is a `GtkWindow` subclass whose layout is the template
//! `resources/ui/dialog.ui`. It is a window of its own, modal and
//! transient for the browser window, so the desktop attaches and dims it
//! as it does every GNOME dialog, and the browser window's shortcuts do
//! not reach through it. A caller awaits [`Dialog::next_response`] on the
//! main loop, so nothing blocks while the dialog is open.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::transfer::Cancellation;

/// How a button looks, and whether Enter in a field presses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ButtonStyle {
    /// A bordered button, such as "Skip duplicates".
    Standard,
    /// The accent button Enter presses, such as "Save".
    Primary,
    /// The red button of a destructive question, such as "Move to Trash".
    Danger,
}

impl ButtonStyle {
    /// The CSS class the skin draws the button with.
    const fn css_class(self) -> &'static str {
        match self {
            ButtonStyle::Standard => "bordered",
            ButtonStyle::Primary => "accent",
            ButtonStyle::Danger => "danger",
        }
    }
}

/// A button of one dialog, as [`Dialog::next_response`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DialogButton(usize);

/// The answer a button, Escape or closing the window gives.
type Answer = Option<DialogButton>;

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use ox_core::transfer::Cancellation;

    use super::Answer;

    /// Private state of [`super::Dialog`].
    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/dialog.ui")]
    pub(crate) struct Dialog {
        /// The scrolling body, capped to the parent window's height.
        #[template_child]
        pub(super) scroller: TemplateChild<gtk::ScrolledWindow>,
        /// The heading, which is also the window's title.
        #[template_child]
        pub(super) title_label: TemplateChild<gtk::Label>,
        /// The question or the facts, with its line breaks kept.
        #[template_child]
        pub(super) message_label: TemplateChild<gtk::Label>,
        /// The fields, notes and check boxes, in the order added.
        #[template_child]
        pub(super) fields: TemplateChild<gtk::Box>,
        /// Why the last try failed (`.modal-error`).
        #[template_child]
        pub(super) error_label: TemplateChild<gtk::Label>,
        /// The buttons, right-aligned.
        #[template_child]
        pub(super) actions: TemplateChild<gtk::Box>,
        /// The buttons, in the order added; a [`super::DialogButton`] is
        /// an index into it.
        pub(super) buttons: RefCell<Vec<gtk::Button>>,
        /// Where buttons, Escape and closing send their answer.
        pub(super) answers: async_channel::Sender<Answer>,
        /// Where [`super::Dialog::next_response`] reads it.
        pub(super) answer_queue: async_channel::Receiver<Answer>,
        /// The operation carrying out an answer, while it runs; closing
        /// the dialog cancels it.
        pub(super) running: RefCell<Option<Cancellation>>,
    }

    impl Default for Dialog {
        fn default() -> Self {
            let (answers, answer_queue) = async_channel::unbounded();
            Self {
                scroller: TemplateChild::default(),
                title_label: TemplateChild::default(),
                message_label: TemplateChild::default(),
                fields: TemplateChild::default(),
                error_label: TemplateChild::default(),
                actions: TemplateChild::default(),
                buttons: RefCell::default(),
                answers,
                answer_queue,
                running: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Dialog {
        const NAME: &'static str = "OxDialog";
        type Type = super::Dialog;
        type ParentType = gtk::Window;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(dialog: &glib::subclass::InitializingObject<Self>) {
            dialog.init_template();
        }
    }

    impl ObjectImpl for Dialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().add_controller(super::escape_cancels());
        }

        fn dispose(&self) {
            // A dialog destroyed with its browser window cancels, so the
            // command awaiting it ends instead of waiting forever. After
            // `finish` nobody listens any more, which is fine.
            let _ = self.answers.try_send(None);
        }
    }

    impl WidgetImpl for Dialog {
        /// Fits the dialog to its parent window before its first frame, as
        /// it is realized when it shows.
        fn realize(&self) {
            crate::modal::fit_to_parent(&*self.obj(), &self.scroller);
            self.parent_realize();
        }
    }

    impl WindowImpl for Dialog {
        fn close_request(&self) -> glib::Propagation {
            if let Some(running) = self.running.take() {
                running.cancel();
            }
            // Closing is a cancellation; a caller that already has its
            // answer has stopped listening, which is fine.
            let _ = self.answers.try_send(None);
            self.parent_close_request()
        }
    }
}

glib::wrapper! {
    /// A modal dialog over a browser window.
    pub(crate) struct Dialog(ObjectSubclass<imp::Dialog>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::Root, gtk::ShortcutManager;
}

impl Dialog {
    /// A dialog over `parent` with the heading `title` and `message`
    /// under it; an empty message is not shown. Add fields and buttons,
    /// then [`Self::open`] it.
    pub(super) fn new(parent: &impl IsA<gtk::Window>, title: &str, message: &str) -> Self {
        let dialog: Self = glib::Object::builder()
            .property("transient-for", parent)
            .property("title", title)
            .build();
        super::actions::follow_text_size_keys(&dialog);
        let imp = dialog.imp();
        imp.title_label.set_text(title);
        imp.message_label.set_text(message);
        imp.message_label.set_visible(!message.is_empty());
        dialog.update_relation(&[gtk::accessible::Relation::LabelledBy(&[imp
            .title_label
            .upcast_ref()])]);
        dialog
    }

    /// Adds a labelled one-line text field showing `text`
    /// (`textField`), which Enter submits.
    pub(super) fn add_text_field(&self, label: &str, text: &str) -> gtk::Entry {
        let entry = gtk::Entry::builder().text(text).activates_default(true).build();
        self.add_labelled(label, &entry);
        entry
    }

    /// Adds `control` under a field label, which names it for screen
    /// readers too (`label.field-label`).
    pub(super) fn add_labelled(&self, label: &str, control: &impl IsA<gtk::Widget>) {
        let caption = gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .css_classes(["field-label"])
            .mnemonic_widget(control)
            .build();
        let control = control.upcast_ref::<gtk::Widget>();
        control.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
        self.imp().fields.append(&caption);
        self.imp().fields.append(control);
    }

    /// Adds a boxed note in muted text (`.modal-note`).
    pub(super) fn add_note(&self, text: &str) {
        self.add_text_line(text, "dialog-note");
    }

    /// Adds a line of small muted text that can be selected, such as a
    /// folder's path (`.template-path`), and returns it, so a caller can
    /// change it while the dialog is open.
    pub(super) fn add_hint(&self, text: &str) -> gtk::Label {
        self.add_text_line(text, "dialog-hint")
    }

    fn add_text_line(&self, text: &str, css_class: &str) -> gtk::Label {
        let line = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(60)
            .selectable(true)
            .css_classes([css_class])
            .build();
        // Selectable with the pointer, but no stop for the keyboard.
        line.set_focusable(false);
        self.imp().fields.append(&line);
        line
    }

    /// Adds a long selectable text, such as a licence, in a scrolled
    /// box `height` pixels tall.
    pub(super) fn add_scrolled_text(&self, text: &str, height: i32) {
        let label = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .yalign(0.0)
            .selectable(true)
            .css_classes(["dialog-hint", "scrolled-text"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&label)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(height)
            .build();
        self.imp().fields.append(&scrolled);
    }

    /// The text of the scrolled box, for tests.
    #[cfg(test)]
    pub(crate) fn scrolled_text(&self) -> String {
        let scrolled = super::widget_tree::children(&*self.imp().fields)
            .find_map(|child| child.downcast::<gtk::ScrolledWindow>().ok());
        let label = scrolled
            .and_then(|scrolled| scrolled.child())
            .and_then(|child| child.downcast::<gtk::Viewport>().ok()?.child())
            .and_then(|child| child.downcast::<gtk::Label>().ok());
        label.map(|label| label.text().to_string()).unwrap_or_default()
    }

    /// Adds a check box (`.checkbox-row`).
    pub(super) fn add_check_button(&self, label: &str, active: bool) -> gtk::CheckButton {
        let check = gtk::CheckButton::builder().label(label).active(active).build();
        check.add_css_class("dialog-check");
        self.imp().fields.append(&check);
        check
    }

    /// Adds the Cancel button, which answers "cancelled" like Escape.
    pub(super) fn add_cancel_button(&self) {
        let button = self.new_button("Cancel", ButtonStyle::Standard);
        let answers = self.imp().answers.clone();
        button.connect_clicked(move |_| {
            let _ = answers.try_send(None);
        });
    }

    /// Adds a button labelled `label` in `style`; clicking it answers the
    /// returned [`DialogButton`]. A primary button is the one Enter in a
    /// field presses.
    pub(super) fn add_button(&self, label: &str, style: ButtonStyle) -> DialogButton {
        let button = self.new_button(label, style);
        let answer = DialogButton(self.imp().buttons.borrow().len() - 1);
        let answers = self.imp().answers.clone();
        button.connect_clicked(move |_| {
            let _ = answers.try_send(Some(answer));
        });
        if style == ButtonStyle::Primary {
            self.set_default_widget(Some(&button));
        }
        answer
    }

    /// Makes Enter in `entry` press `button` instead of the primary
    /// button, for a field that belongs to one of several answers.
    pub(super) fn submit_with(&self, entry: &gtk::Entry, button: DialogButton) {
        entry.set_activates_default(false);
        let answers = self.imp().answers.clone();
        entry.connect_activate(move |_| {
            let _ = answers.try_send(Some(button));
        });
    }

    /// A button appended to the actions and remembered.
    fn new_button(&self, label: &str, style: ButtonStyle) -> gtk::Button {
        let button = gtk::Button::builder()
            .label(label)
            .css_classes([style.css_class(), "dialog-button"])
            .build();
        self.imp().actions.append(&button);
        self.imp().buttons.borrow_mut().push(button.clone());
        button
    }

    /// Shows the dialog, focusing its first text field with the text
    /// selected, or else its first button.
    pub(super) fn open(&self) {
        let first_field = super::widget_tree::children(&*self.imp().fields)
            .find_map(|child| child.downcast::<gtk::Entry>().ok());
        let first_button = self.imp().buttons.borrow().first().cloned();
        // Set before the window shows, so GTK's initial focus lands there.
        let initial_focus: Option<gtk::Widget> = match (&first_field, first_button) {
            (Some(field), _) => Some(field.clone().upcast()),
            (None, button) => button.map(Cast::upcast),
        };
        GtkWindowExt::set_focus(self, initial_focus.as_ref());
        if let Some(parent) = self.transient_for().and_downcast::<super::BrowserWindow>() {
            parent.quiet_for_dialog();
        }
        self.present();
        if let Some(field) = first_field {
            field.grab_focus();
            field.select_region(0, -1);
        }
    }

    /// Shows the dialog with its first button focused even when it has a
    /// text field, for a question whose field is only one of the answers:
    /// a reflexive Enter then does not choose it.
    pub(super) fn open_on_first_button(&self) {
        let first_button = self.imp().buttons.borrow().first().cloned();
        GtkWindowExt::set_focus(self, first_button.as_ref());
        self.present();
        if let Some(button) = first_button {
            button.grab_focus();
        }
    }

    /// Waits for the next answer: the button pressed, or `None` for
    /// Cancel, Escape or closing, which also closes the dialog. After a
    /// button the dialog stays open; call [`Self::finish`] once the answer
    /// is accepted.
    pub(super) async fn next_response(&self) -> Option<DialogButton> {
        let queue = self.imp().answer_queue.clone();
        let answer = queue.recv().await.ok().flatten();
        if answer.is_none() {
            self.finish();
        }
        answer
    }

    /// Shows why the last try failed and keeps the dialog open.
    pub(super) fn show_error(&self, message: &str) {
        let error_label = &self.imp().error_label;
        error_label.set_text(message);
        error_label.set_visible(true);
    }

    /// Disables the buttons while `running` carries out an answer, as
    /// `Create` is disabled while it runs, and enables them again with
    /// `None`. Closing the dialog meanwhile cancels `running`.
    pub(super) fn set_busy(&self, running: Option<&Cancellation>) {
        let busy = running.is_some();
        self.imp().running.replace(running.cloned());
        for button in self.imp().buttons.borrow().iter() {
            button.set_sensitive(!busy);
        }
    }

    /// Closes the dialog for good.
    pub(super) fn finish(&self) {
        self.destroy();
    }

    /// The heading, for tests.
    #[cfg(test)]
    pub(crate) fn title_text(&self) -> String {
        self.imp().title_label.text().to_string()
    }

    /// The message, for tests.
    #[cfg(test)]
    pub(crate) fn message_text(&self) -> String {
        self.imp().message_label.text().to_string()
    }

    /// The error line while shown, for tests.
    #[cfg(test)]
    pub(crate) fn error_text(&self) -> Option<String> {
        let error_label = &self.imp().error_label;
        error_label.is_visible().then(|| error_label.text().to_string())
    }

    /// The button labels in order, for tests.
    #[cfg(test)]
    pub(crate) fn button_labels(&self) -> Vec<String> {
        let buttons = self.imp().buttons.borrow();
        buttons
            .iter()
            .filter_map(gtk::Button::label)
            .map(String::from)
            .collect()
    }

    /// Presses the button labelled `label`, as a click does, for tests.
    #[cfg(test)]
    pub(crate) fn press(&self, label: &str) {
        let button = self
            .imp()
            .buttons
            .borrow()
            .iter()
            .find(|button| button.label().as_deref() == Some(label))
            .cloned()
            .unwrap_or_else(|| panic!("the dialog has a {label} button"));
        button.emit_clicked();
    }
}

/// Escape closes the dialog, which cancels it (`closeModal` in app.js).
fn escape_cancels() -> gtk::ShortcutController {
    let shortcuts = gtk::ShortcutController::new();
    let trigger = gtk::KeyvalTrigger::new(gdk::Key::Escape, gdk::ModifierType::empty());
    let close = gtk::CallbackAction::new(|widget, _| {
        if let Some(window) = widget.downcast_ref::<gtk::Window>() {
            window.close();
        }
        glib::Propagation::Stop
    });
    shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(close)));
    shortcuts
}

/// Shows `text` under `title` with one OK button, as `showMessage` does,
/// and returns once it is dismissed.
pub(super) async fn show_message(parent: &impl IsA<gtk::Window>, title: &str, text: &str) {
    let dialog = Dialog::new(parent, title, text);
    dialog.add_button("OK", ButtonStyle::Primary);
    dialog.open();
    dialog.next_response().await;
    dialog.finish();
}
