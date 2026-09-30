// SPDX-License-Identifier: AGPL-3.0-only
//! The frame of the network dialogs with fields: Map network location
//! and Sign out of server.
//!
//! Ports `showModal` and `textField` of `desktop/ui/app.js` as those two
//! dialogs use them, with `.modal` of `desktop/ui/style.css`; the layout
//! is the template `resources/ui/network-form.ui`. The behaviour follows
//! `showModal`:
//!
//! - The first text field has focus; without one, Cancel has it, so Enter
//!   never signs out by accident. Enter in a text field presses the
//!   primary button.
//! - An error stays inside the dialog, which stays open for another try
//!   ([`NetworkFormDialog::show_error`]).
//! - Escape, Cancel and closing the window end the dialog and drop the
//!   work it runs ([`NetworkFormDialog::run`]), so a late success is
//!   ignored (the cancel token of `connectDialog`).

use std::future::Future;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

/// A handler of [`NetworkFormDialog::connect_confirmed`].
type ConfirmHandler = Rc<dyn Fn(&NetworkFormDialog)>;

/// Whether a check box starts checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckState {
    /// Checked, such as "Save in the sidebar".
    Checked,
    /// Unchecked.
    Unchecked,
}

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use super::ConfirmHandler;

    /// Private state of [`super::NetworkFormDialog`].
    #[derive(Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/network-form.ui")]
    pub(crate) struct NetworkFormDialog {
        /// The heading, which is also the window's title.
        #[template_child]
        pub(super) title_label: TemplateChild<gtk::Label>,
        /// What the dialog does.
        #[template_child]
        pub(super) message_label: TemplateChild<gtk::Label>,
        /// The fields, check boxes and notes, in the order added.
        #[template_child]
        pub(super) fields: TemplateChild<gtk::Box>,
        /// Why the last try failed.
        #[template_child]
        pub(super) error_label: TemplateChild<gtk::Label>,
        /// Cancel.
        #[template_child]
        pub(super) cancel_button: TemplateChild<gtk::Button>,
        /// The primary button, such as Connect.
        #[template_child]
        pub(super) confirm_button: TemplateChild<gtk::Button>,
        /// The primary button's own label, shown again after a failure.
        pub(super) confirm_label: RefCell<String>,
        /// Hears the primary button.
        pub(super) confirmed: RefCell<Option<ConfirmHandler>>,
        /// The work the primary button started; aborted when the dialog
        /// closes.
        pub(super) work: RefCell<Option<glib::JoinHandle<()>>>,
    }

    impl std::fmt::Debug for NetworkFormDialog {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("NetworkFormDialog")
                .field("title", &self.title_label.text())
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NetworkFormDialog {
        const NAME: &'static str = "OxNetworkFormDialog";
        type Type = super::NetworkFormDialog;
        type ParentType = gtk::Window;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(dialog: &glib::subclass::InitializingObject<Self>) {
            dialog.init_template();
        }
    }

    impl ObjectImpl for NetworkFormDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().connect_buttons();
        }
    }

    impl WidgetImpl for NetworkFormDialog {}

    impl WindowImpl for NetworkFormDialog {
        /// Closing drops the work the dialog runs, so its result is
        /// ignored.
        fn close_request(&self) -> glib::Propagation {
            if let Some(work) = self.work.take() {
                work.abort();
            }
            self.confirmed.take();
            self.parent_close_request()
        }
    }
}

glib::wrapper! {
    /// A modal network dialog: a heading, a message, fields and buttons.
    pub(crate) struct NetworkFormDialog(ObjectSubclass<imp::NetworkFormDialog>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::Root, gtk::ShortcutManager;
}

impl NetworkFormDialog {
    /// A dialog over `parent` headed `title`, saying `message`, whose
    /// primary button reads `confirm_label`. Add fields, then present it.
    pub(crate) fn new(
        parent: &impl IsA<gtk::Window>,
        title: &str,
        message: &str,
        confirm_label: &str,
    ) -> Self {
        let dialog: Self = glib::Object::builder()
            .property("transient-for", parent)
            .property("title", title)
            .build();
        let imp = dialog.imp();
        imp.title_label.set_text(title);
        imp.message_label.set_text(message);
        imp.confirm_button.set_label(confirm_label);
        imp.confirm_label.replace(confirm_label.to_owned());
        dialog.set_default_widget(Some(&*imp.confirm_button));
        GtkWindowExt::set_focus(&dialog, Some(&*imp.cancel_button));
        dialog
    }

    /// Adds a labelled text field (`textField` in app.js) and returns its
    /// entry. The first one has focus; Enter in it presses the primary
    /// button.
    pub(crate) fn add_text_field(&self, label: &str, placeholder: &str) -> gtk::Entry {
        let entry = gtk::Entry::builder()
            .placeholder_text(placeholder)
            .activates_default(true)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(label)]);
        let caption = gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .mnemonic_widget(&entry)
            .css_classes(["field-label"])
            .build();
        let fields = &self.imp().fields;
        let is_first_field = !self.has_text_field();
        fields.append(&caption);
        fields.append(&entry);
        if is_first_field {
            GtkWindowExt::set_focus(self, Some(&entry));
        }
        entry
    }

    /// Adds a labelled drop-down of `choices`, the first one selected.
    pub(crate) fn add_drop_down(&self, label: &str, choices: &[&str]) -> gtk::DropDown {
        let drop_down = gtk::DropDown::from_strings(choices);
        drop_down.update_property(&[gtk::accessible::Property::Label(label)]);
        let caption = gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .mnemonic_widget(&drop_down)
            .css_classes(["field-label"])
            .build();
        let fields = &self.imp().fields;
        fields.append(&caption);
        fields.append(&drop_down);
        drop_down
    }

    /// Shows or hides the field `field` with its caption.
    pub(crate) fn set_field_visible(field: &impl IsA<gtk::Widget>, visible: bool) {
        let caption = field
            .prev_sibling()
            .filter(glib::object::ObjectExt::is::<gtk::Label>);
        if let Some(caption) = caption {
            caption.set_visible(visible);
        }
        field.set_visible(visible);
    }

    /// Adds a check box reading `label`, as `initially` says.
    pub(crate) fn add_check_box(&self, label: &str, initially: CheckState) -> gtk::CheckButton {
        let check = gtk::CheckButton::builder()
            .label(label)
            .active(initially == CheckState::Checked)
            .css_classes(["checkbox-row"])
            .build();
        self.imp().fields.append(&check);
        check
    }

    /// Adds a note in a box under the fields (`.modal-note`).
    pub(crate) fn add_note(&self, text: &str) {
        let note = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(60)
            .css_classes(["dialog-note"])
            .build();
        self.imp().fields.append(&note);
    }

    /// Calls `on_confirm` when the primary button is pressed (or Enter in
    /// a text field).
    pub(crate) fn connect_confirmed(&self, on_confirm: impl Fn(&Self) + 'static) {
        self.imp().confirmed.replace(Some(Rc::new(on_confirm)));
    }

    /// Runs `work`, with the primary button disabled and reading
    /// `busy_label` meanwhile. Closing the dialog drops it.
    pub(crate) fn run(&self, busy_label: &str, work: impl Future<Output = ()> + 'static) {
        let imp = self.imp();
        imp.error_label.set_visible(false);
        imp.confirm_button.set_sensitive(false);
        imp.confirm_button.set_label(busy_label);
        let running = glib::spawn_future_local(work);
        if let Some(earlier) = imp.work.replace(Some(running)) {
            earlier.abort();
        }
    }

    /// Shows why the work failed, and lets the user try again.
    pub(crate) fn show_error(&self, message: &str) {
        let imp = self.imp();
        imp.work.take();
        imp.error_label.set_text(message);
        imp.error_label.set_visible(true);
        imp.confirm_button.set_sensitive(true);
        imp.confirm_button.set_label(&imp.confirm_label.borrow());
    }

    /// Closes the dialog once its work is done.
    pub(crate) fn finish(&self) {
        // The work that calls this is finishing; nothing is left to abort.
        self.imp().work.take();
        self.close();
    }

    /// Whether a text field was added already.
    fn has_text_field(&self) -> bool {
        let mut child = self.imp().fields.first_child();
        while let Some(widget) = child {
            if widget.is::<gtk::Entry>() {
                return true;
            }
            child = widget.next_sibling();
        }
        false
    }

    /// Cancel and Escape close the dialog; the primary button calls the
    /// handler.
    fn connect_buttons(&self) {
        let imp = self.imp();
        imp.cancel_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.close()
        ));
        imp.confirm_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.confirm()
        ));
        let escape = gtk::ShortcutController::new();
        let trigger = gtk::KeyvalTrigger::new(gdk::Key::Escape, gdk::ModifierType::empty());
        let close = gtk::NamedAction::new("window.close");
        escape.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(close)));
        self.add_controller(escape);
    }

    /// The primary button: hands the dialog to the handler.
    fn confirm(&self) {
        let handler = self.imp().confirmed.borrow().clone();
        if let Some(handler) = handler {
            handler(self);
        }
    }
}

/// What tests read and do, as a user would.
#[cfg(test)]
impl NetworkFormDialog {
    /// Presses the primary button.
    pub(crate) fn press_confirm(&self) {
        self.imp().confirm_button.emit_clicked();
    }

    /// Presses Cancel.
    pub(crate) fn press_cancel(&self) {
        self.imp().cancel_button.emit_clicked();
    }

    /// The error shown, if any.
    pub(crate) fn error_text(&self) -> Option<String> {
        let error = &self.imp().error_label;
        error.is_visible().then(|| error.text().to_string())
    }

    /// The primary button's label and whether it can be pressed.
    pub(crate) fn confirm_state(&self) -> (String, bool) {
        let confirm = &self.imp().confirm_button;
        let label = confirm.label().map(|label| label.to_string()).unwrap_or_default();
        (label, confirm.is_sensitive())
    }

    /// Every text the dialog shows, in order.
    pub(crate) fn texts(&self) -> Vec<String> {
        let labels = crate::test_support::harness::descendants::<gtk::Label>(self);
        labels.iter().map(|label| label.text().to_string()).collect()
    }
}
