// SPDX-License-Identifier: AGPL-3.0-only
//! The private state of [`super::SignInDialog`]: the template's widgets,
//! the handler of its answers, and closing as Cancel.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::subclass::prelude::*;

use super::AnswerHandler;

/// Private state of [`super::SignInDialog`].
#[derive(Default, gtk::CompositeTemplate)]
#[template(file = "../../../resources/ui/network-sign-in.ui")]
pub(crate) struct SignInDialog {
    /// The scrolling body, capped to the parent window's height.
    #[template_child]
    pub(super) scroller: TemplateChild<gtk::ScrolledWindow>,
    /// The shield of the caption bar.
    #[template_child]
    pub(super) caption_glyph: TemplateChild<gtk::Image>,
    /// × in the caption bar: Cancel sign-in.
    #[template_child]
    pub(super) close_button: TemplateChild<gtk::Button>,
    /// Its glyph.
    #[template_child]
    pub(super) close_glyph: TemplateChild<gtk::Image>,
    /// The password page and the question page.
    #[template_child]
    pub(super) pages: TemplateChild<gtk::Stack>,
    /// The computer in the badge beside the heading.
    #[template_child]
    pub(super) device_glyph: TemplateChild<gtk::Image>,
    /// "Connect to <host>".
    #[template_child]
    pub(super) host_label: TemplateChild<gtk::Label>,
    /// What to enter, or why the last account was refused.
    #[template_child]
    pub(super) description_label: TemplateChild<gtk::Label>,
    /// The glyph before the server's name.
    #[template_child]
    pub(super) target_glyph: TemplateChild<gtk::Image>,
    /// The server's name.
    #[template_child]
    pub(super) target_label: TemplateChild<gtk::Label>,
    /// The user name, optionally `DOMAIN\user`.
    #[template_child]
    pub(super) username_entry: TemplateChild<gtk::Entry>,
    /// The password.
    #[template_child]
    pub(super) password_entry: TemplateChild<gtk::Entry>,
    /// Shows or hides the password.
    #[template_child]
    pub(super) reveal_button: TemplateChild<gtk::ToggleButton>,
    /// Its eye.
    #[template_child]
    pub(super) reveal_glyph: TemplateChild<gtk::Image>,
    /// "Remember my credentials".
    #[template_child]
    pub(super) remember_check: TemplateChild<gtk::CheckButton>,
    /// The shield before the note.
    #[template_child]
    pub(super) note_glyph: TemplateChild<gtk::Image>,
    /// Where the credentials are kept.
    #[template_child]
    pub(super) note_label: TemplateChild<gtk::Label>,
    /// "Connect as guest", where the server allows it.
    #[template_child]
    pub(super) guest_button: TemplateChild<gtk::Button>,
    /// Why the last answer was refused.
    #[template_child]
    pub(super) error_label: TemplateChild<gtk::Label>,
    /// The server's question.
    #[template_child]
    pub(super) question_label: TemplateChild<gtk::Label>,
    /// One button per answer to the question, then Cancel.
    #[template_child]
    pub(super) choices: TemplateChild<gtk::FlowBox>,
    /// The footer with Cancel and Connect.
    #[template_child]
    pub(super) actions: TemplateChild<gtk::Box>,
    /// Cancel.
    #[template_child]
    pub(super) cancel_button: TemplateChild<gtk::Button>,
    /// Connect.
    #[template_child]
    pub(super) connect_button: TemplateChild<gtk::Button>,
    /// The server needs a user name, unless the user signs in as guest.
    pub(super) needs_username: Cell<bool>,
    /// Set once the challenge is over, so closing asks nothing more.
    pub(super) is_dismissed: Cell<bool>,
    /// Hears the user's answers.
    pub(super) answered: RefCell<Option<AnswerHandler>>,
}

impl std::fmt::Debug for SignInDialog {
    /// Shows whether the dialog is still open, never its fields.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SignInDialog")
            .field("is_dismissed", &self.is_dismissed.get())
            .finish_non_exhaustive()
    }
}

#[glib::object_subclass]
impl ObjectSubclass for SignInDialog {
    const NAME: &'static str = "OxSignInDialog";
    type Type = super::SignInDialog;
    type ParentType = gtk::Window;

    fn class_init(klass: &mut Self::Class) {
        klass.bind_template();
    }

    fn instance_init(dialog: &glib::subclass::InitializingObject<Self>) {
        dialog.init_template();
    }
}

impl ObjectImpl for SignInDialog {
    fn constructed(&self) {
        self.parent_constructed();
        let dialog = self.obj();
        dialog.show_icons();
        dialog.connect_controls();
    }
}

impl WidgetImpl for SignInDialog {
    /// Fits the dialog to its parent window before its first frame, as
    /// it is realized when it shows.
    fn realize(&self) {
        crate::modal::fit_to_parent(&*self.obj(), &self.scroller);
        self.parent_realize();
    }
}

impl WindowImpl for SignInDialog {
    /// Closing the window is Cancel. The dialog closes once the
    /// challenge is dismissed, which cancelling normally does at once;
    /// GTK ignores a close asked for while this one runs.
    fn close_request(&self) -> glib::Propagation {
        if !self.is_dismissed.get() {
            self.obj().answer(ox_core::network::Answer::Cancel);
        }
        if self.is_dismissed.get() {
            self.parent_close_request()
        } else {
            glib::Propagation::Stop
        }
    }
}
