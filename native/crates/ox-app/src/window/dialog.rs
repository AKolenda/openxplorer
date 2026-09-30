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
//!   stalled share cannot hold the dialog open. Work that cannot be
//!   cancelled, such as connecting a share, runs with [`Dialog::run`]:
//!   its button reads "Connecting…" meanwhile, Cancel stays usable, and
//!   cancelling drops the work so a late success is ignored.
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

use std::future::Future;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::transfer::Cancellation;

use super::ButtonStyle;

/// A button of one dialog, as [`Dialog::next_response`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DialogButton(usize);

/// The answer a button, Escape or closing the window gives.
type Answer = Option<DialogButton>;

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use ox_core::transfer::Cancellation;

    use super::Answer;
    use crate::dialog_layer::DialogFrame;

    /// Private state of [`super::Dialog`].
    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/dialog.ui")]
    pub(crate) struct Dialog {
        /// The heading, message, fields, error line and buttons; its body
        /// scrolls, capped to the parent window's height.
        #[template_child]
        pub(super) frame: TemplateChild<DialogFrame>,
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
        /// The work [`super::Dialog::run`] started, while it runs;
        /// cancelling the dialog drops it.
        pub(super) work: RefCell<Option<glib::JoinHandle<()>>>,
        /// The button that started the work, disabled meanwhile, with its
        /// own label to show again.
        pub(super) busy_button: RefCell<Option<(gtk::Button, glib::GString)>>,
    }

    impl Default for Dialog {
        fn default() -> Self {
            let (answers, answer_queue) = async_channel::unbounded();
            Self {
                frame: TemplateChild::default(),
                buttons: RefCell::default(),
                answers,
                answer_queue,
                running: RefCell::default(),
                work: RefCell::default(),
                busy_button: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Dialog {
        const NAME: &'static str = "OxDialog";
        type Type = super::Dialog;
        type ParentType = gtk::Window;

        fn class_init(klass: &mut Self::Class) {
            DialogFrame::ensure_type();
            klass.bind_template();
        }

        fn instance_init(dialog: &glib::subclass::InitializingObject<Self>) {
            dialog.init_template();
        }
    }

    impl ObjectImpl for Dialog {
        fn constructed(&self) {
            self.parent_constructed();
            // Escape closes the dialog, which cancels it (`closeModal` in
            // app.js).
            self.obj().add_controller(crate::modal::escape_closes());
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
            crate::modal::fit_to_parent(&*self.obj(), &self.frame.scroller());
            self.parent_realize();
        }
    }

    impl WindowImpl for Dialog {
        fn close_request(&self) -> glib::Propagation {
            if let Some(running) = self.running.take() {
                running.cancel();
            }
            self.obj().drop_work();
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
    pub(crate) fn new(parent: &impl IsA<gtk::Window>, title: &str, message: &str) -> Self {
        let dialog: Self = glib::Object::builder()
            .property("transient-for", parent)
            .property("title", title)
            .build();
        super::actions::follow_text_size_keys(&dialog);
        let frame = &dialog.imp().frame;
        frame.set_title(title);
        frame.set_message(message);
        dialog.update_relation(&[gtk::accessible::Relation::LabelledBy(&[frame
            .title_label()
            .upcast_ref()])]);
        dialog
    }

    /// The box the fields, notes and check boxes go in, in the order added.
    fn fields(&self) -> gtk::Box {
        self.imp().frame.body()
    }

    /// Adds a labelled one-line text field showing `text`
    /// (`textField`), which Enter submits.
    pub(crate) fn add_text_field(&self, label: &str, text: &str) -> gtk::Entry {
        let entry = gtk::Entry::builder().text(text).activates_default(true).build();
        self.add_labelled(label, &entry);
        entry
    }

    /// Adds `control` under a field label, which names it for screen
    /// readers too (`label.field-label`).
    pub(crate) fn add_labelled(&self, label: &str, control: &impl IsA<gtk::Widget>) {
        let caption = gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .css_classes(["field-label"])
            .mnemonic_widget(control)
            .build();
        let control = control.upcast_ref::<gtk::Widget>();
        control.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
        self.fields().append(&caption);
        self.fields().append(control);
    }

    /// Adds a boxed note in muted text (`.modal-note`).
    pub(crate) fn add_note(&self, text: &str) {
        self.add_text_line(text, "dialog-note");
    }

    /// Adds a line of small muted text that can be selected, such as a
    /// folder's path (`.template-path`), and returns it, so a caller can
    /// change it while the dialog is open.
    pub(crate) fn add_hint(&self, text: &str) -> gtk::Label {
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
        self.fields().append(&line);
        line
    }

    /// Adds a long selectable text, such as a licence, in a scrolled
    /// box `height` pixels tall.
    pub(crate) fn add_scrolled_text(&self, text: &str, height: i32) {
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
        self.fields().append(&scrolled);
    }

    /// The text of the scrolled box, for tests.
    #[cfg(test)]
    pub(crate) fn scrolled_text(&self) -> String {
        let scrolled = super::widget_tree::children(&self.fields())
            .find_map(|child| child.downcast::<gtk::ScrolledWindow>().ok());
        let label = scrolled
            .and_then(|scrolled| scrolled.child())
            .and_then(|child| child.downcast::<gtk::Viewport>().ok()?.child())
            .and_then(|child| child.downcast::<gtk::Label>().ok());
        label.map(|label| label.text().to_string()).unwrap_or_default()
    }

    /// Shows or hides `field`, added with [`Self::add_labelled`] or
    /// [`Self::add_text_field`], together with its label.
    pub(crate) fn set_field_visible(field: &impl IsA<gtk::Widget>, visible: bool) {
        let caption = field
            .prev_sibling()
            .filter(|caption| caption.has_css_class("field-label"));
        if let Some(caption) = caption {
            caption.set_visible(visible);
        }
        field.set_visible(visible);
    }

    /// Adds a check box (`.checkbox-row`).
    pub(crate) fn add_check_button(&self, label: &str, active: bool) -> gtk::CheckButton {
        let check = gtk::CheckButton::builder().label(label).active(active).build();
        check.add_css_class("dialog-check");
        self.fields().append(&check);
        check
    }

    /// Adds the Cancel button, which answers "cancelled" like Escape.
    pub(crate) fn add_cancel_button(&self) {
        let button = self.new_button("Cancel", ButtonStyle::Bordered);
        let answers = self.imp().answers.clone();
        button.connect_clicked(move |_| {
            let _ = answers.try_send(None);
        });
    }

    /// Adds a button labelled `label` in `style`; clicking it answers the
    /// returned [`DialogButton`]. The accent button is the primary one,
    /// which Enter in a field presses.
    pub(crate) fn add_button(&self, label: &str, style: ButtonStyle) -> DialogButton {
        let button = self.new_button(label, style);
        let answer = DialogButton(self.imp().buttons.borrow().len() - 1);
        let answers = self.imp().answers.clone();
        button.connect_clicked(move |_| {
            let _ = answers.try_send(Some(answer));
        });
        if style == ButtonStyle::Accent {
            self.set_default_widget(Some(&button));
        }
        answer
    }

    /// Makes Enter in `entry` press `button` instead of the primary
    /// button, for a field that belongs to one of several answers.
    pub(crate) fn submit_with(&self, entry: &gtk::Entry, button: DialogButton) {
        entry.set_activates_default(false);
        let answers = self.imp().answers.clone();
        entry.connect_activate(move |_| {
            let _ = answers.try_send(Some(button));
        });
    }

    /// A button appended to the actions and remembered.
    fn new_button(&self, label: &str, style: ButtonStyle) -> gtk::Button {
        let button = self.imp().frame.add_button(label, style);
        button.add_css_class("dialog-button");
        self.imp().buttons.borrow_mut().push(button.clone());
        button
    }

    /// Shows the dialog, focusing its first text field with the text
    /// selected, or else its first button.
    pub(crate) fn open(&self) {
        let first_field = super::widget_tree::children(&self.fields())
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
    pub(crate) fn open_on_first_button(&self) {
        let first_button = self.imp().buttons.borrow().first().cloned();
        GtkWindowExt::set_focus(self, first_button.as_ref());
        self.present();
        if let Some(button) = first_button {
            button.grab_focus();
        }
    }

    /// Waits for the next answer: the button pressed, or `None` for
    /// Cancel, Escape or closing, which also closes the dialog and drops
    /// the work [`Self::run`] started. After a button the dialog stays
    /// open; call [`Self::finish`] once the answer is accepted.
    pub(crate) async fn next_response(&self) -> Option<DialogButton> {
        let queue = self.imp().answer_queue.clone();
        let answer = queue.recv().await.ok().flatten();
        if answer.is_none() {
            self.drop_work();
            self.finish();
        }
        answer
    }

    /// Calls `on_confirm` each time a button other than Cancel is pressed
    /// (or Enter in a text field presses the primary one), until the
    /// dialog is cancelled or finished; for a dialog that asks one thing
    /// and stays open while the answer is carried out.
    pub(crate) fn connect_confirmed(&self, on_confirm: impl Fn(&Self) + 'static) {
        let dialog = self.clone();
        glib::spawn_future_local(async move {
            // Cancelling and finishing both end the answers.
            while dialog.next_response().await.is_some() {
                on_confirm(&dialog);
            }
        });
    }

    /// Carries out the primary button's answer with `work`, such as
    /// connecting a share: meanwhile the error line hides and the primary
    /// button is disabled and reads `busy_label`, while Cancel stays
    /// usable. Cancelling the dialog drops `work`, so a late success is
    /// ignored (the cancel token of `connectDialog`); [`Self::show_error`]
    /// or [`Self::finish`] ends it.
    pub(crate) fn run(&self, busy_label: &str, work: impl Future<Output = ()> + 'static) {
        let imp = self.imp();
        imp.frame.show_error("");
        if let Some(primary) = self.default_widget().and_downcast::<gtk::Button>() {
            if imp.busy_button.borrow().is_none() {
                let label = primary.label().unwrap_or_default();
                imp.busy_button.replace(Some((primary.clone(), label)));
            }
            primary.set_sensitive(false);
            primary.set_label(busy_label);
        }
        if let Some(earlier) = imp.work.replace(Some(glib::spawn_future_local(work))) {
            earlier.abort();
        }
    }

    /// Shows why the last try failed and keeps the dialog open; the
    /// button whose work failed can be pressed again.
    pub(crate) fn show_error(&self, message: &str) {
        let imp = self.imp();
        imp.work.take();
        if let Some((button, label)) = imp.busy_button.take() {
            button.set_label(&label);
            button.set_sensitive(true);
        }
        imp.frame.show_error(message);
    }

    /// Drops the work [`Self::run`] started, so its result is ignored.
    fn drop_work(&self) {
        if let Some(work) = self.imp().work.take() {
            work.abort();
        }
    }

    /// Disables the buttons while `running` carries out an answer, as
    /// `Create` is disabled while it runs, and enables them again with
    /// `None`. Closing the dialog meanwhile cancels `running`.
    pub(crate) fn set_busy(&self, running: Option<&Cancellation>) {
        let busy = running.is_some();
        self.imp().running.replace(running.cloned());
        for button in self.imp().buttons.borrow().iter() {
            button.set_sensitive(!busy);
        }
    }

    /// Closes the dialog for good. Work [`Self::run`] started that calls
    /// this runs to its end.
    pub(crate) fn finish(&self) {
        self.imp().work.take();
        self.destroy();
    }

    /// The heading, for tests.
    #[cfg(test)]
    pub(crate) fn title_text(&self) -> String {
        self.imp().frame.title().to_string()
    }

    /// The message, for tests.
    #[cfg(test)]
    pub(crate) fn message_text(&self) -> String {
        self.imp().frame.message().to_string()
    }

    /// The error line while shown, for tests.
    #[cfg(test)]
    pub(crate) fn error_text(&self) -> Option<String> {
        let error = self.imp().frame.error_text();
        (!error.is_empty()).then(|| error.to_string())
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

    /// Whether the button labelled `label` can be pressed, for tests.
    #[cfg(test)]
    pub(crate) fn can_press(&self, label: &str) -> bool {
        self.button_labelled(label).is_sensitive()
    }

    /// Every text the dialog holds, shown or not, in order, for tests.
    #[cfg(test)]
    pub(crate) fn texts(&self) -> Vec<String> {
        let labels = crate::test_support::harness::descendants::<gtk::Label>(self);
        labels.iter().map(|label| label.text().to_string()).collect()
    }

    /// Presses the button labelled `label`, as a click does, for tests.
    #[cfg(test)]
    pub(crate) fn press(&self, label: &str) {
        self.button_labelled(label).emit_clicked();
    }

    /// Presses the primary button, the one Enter presses, for tests.
    #[cfg(test)]
    pub(crate) fn press_primary(&self) {
        let primary = self.default_widget().and_downcast::<gtk::Button>();
        primary.expect("the dialog has a primary button").emit_clicked();
    }

    /// The button labelled `label`, for tests.
    #[cfg(test)]
    fn button_labelled(&self, label: &str) -> gtk::Button {
        self.imp()
            .buttons
            .borrow()
            .iter()
            .find(|button| button.label().as_deref() == Some(label))
            .cloned()
            .unwrap_or_else(|| panic!("the dialog has a {label} button"))
    }
}

/// Shows `text` under `title` with one OK button, as `showMessage` does,
/// and returns once it is dismissed.
pub(super) async fn show_message(parent: &impl IsA<gtk::Window>, title: &str, text: &str) {
    let dialog = Dialog::new(parent, title, text);
    dialog.add_button("OK", ButtonStyle::Accent);
    dialog.open();
    dialog.next_response().await;
    dialog.finish();
}
