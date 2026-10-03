// SPDX-License-Identifier: AGPL-3.0-only
//! One dialog: its title, message, body, error line and buttons.
//!
//! Ports the box `showModal` in `v2.0.0:desktop/ui/app.js` fills (`#modal`, its
//! `h2`, `p`, body, `.modal-error` and `.modal-actions`), drawn as
//! `native/docs/ui-spec.md` §4.11 refines `.modal`. [`DialogFrame`] is a
//! widget subclass whose layout is the template
//! `resources/ui/dialog-frame.ui`; the code that opens a dialog fills its
//! body and adds its buttons.
//!
//! Every dialog of the app is drawn by a frame. An in-window dialog's
//! frame is shown by a [`DialogLayer`](super::DialogLayer); it emits
//! `closed` once, when it is dismissed for good (its Close button, Escape,
//! or its tab closing), so its owner can cancel the work it started. A
//! modal dialog window ([`crate::window::Dialog`]) holds a frame as its
//! content and answers through its own buttons.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::window::ButtonStyle;

/// Emitted once when the dialog is dismissed for good.
const CLOSED: &str = "closed";

/// How wide a dialog is, as the width classes of `style.css` set it
/// (ui-spec.md §4.11: 510 by default, Properties 600, the archive browser
/// 720, Previous versions 860).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogWidth {
    /// A question or a short form.
    Standard,
    /// The Properties dialog.
    Properties,
    /// The archive browser.
    Archive,
    /// The Properties dialog on its Previous versions tab.
    Versions,
}

impl DialogWidth {
    /// The width in pixels, before the window's own width limits it.
    pub(crate) const fn pixels(self) -> i32 {
        match self {
            DialogWidth::Standard => 510,
            DialogWidth::Properties => 600,
            DialogWidth::Archive => 720,
            DialogWidth::Versions => 860,
        }
    }
}

mod imp {
    use std::cell::Cell;
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;

    use super::{DialogWidth, CLOSED};

    /// Private state of [`super::DialogFrame`].
    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/dialog-frame.ui")]
    pub(crate) struct DialogFrame {
        /// The frame's one child. Bound so that `dispose_template`
        /// unparents it with the frame.
        #[template_child]
        pub(super) column: TemplateChild<gtk::Box>,
        /// The heading.
        #[template_child]
        pub(super) title_label: TemplateChild<gtk::Label>,
        /// The sentence under the heading, when there is one.
        #[template_child]
        pub(super) message_label: TemplateChild<gtk::Label>,
        /// Scrolls the body when the dialog would be taller than its room.
        #[template_child]
        pub(super) scroller: TemplateChild<gtk::ScrolledWindow>,
        /// What the dialog shows: fields, notes and lists.
        #[template_child]
        pub(super) body: TemplateChild<gtk::Box>,
        /// Why the last try failed (`.modal-error`).
        #[template_child]
        pub(super) error_label: TemplateChild<gtk::Label>,
        /// The footer; buttons go after its spacer.
        #[template_child]
        pub(super) actions: TemplateChild<gtk::Box>,
        /// How wide the dialog is.
        pub(super) width: Cell<DialogWidth>,
        /// Set once `closed` was emitted, so it is emitted only once.
        pub(super) is_closed: Cell<bool>,
    }

    impl Default for DialogFrame {
        fn default() -> Self {
            Self {
                column: TemplateChild::default(),
                title_label: TemplateChild::default(),
                message_label: TemplateChild::default(),
                scroller: TemplateChild::default(),
                body: TemplateChild::default(),
                error_label: TemplateChild::default(),
                actions: TemplateChild::default(),
                width: Cell::new(DialogWidth::Standard),
                is_closed: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DialogFrame {
        const NAME: &'static str = "OxDialogFrame";
        type Type = super::DialogFrame;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("ox-dialog-frame");
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.bind_template();
        }

        fn instance_init(frame: &glib::subclass::InitializingObject<Self>) {
            frame.init_template();
        }
    }

    impl ObjectImpl for DialogFrame {
        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "dialog-frame.ui");
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder(CLOSED).build()])
        }

        fn dispose(&self) {
            self.dispose_template();
        }
    }

    impl WidgetImpl for DialogFrame {}
}

glib::wrapper! {
    /// One in-window dialog.
    pub(crate) struct DialogFrame(ObjectSubclass<imp::DialogFrame>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DialogFrame {
    /// An empty dialog titled `title`, `width` wide.
    pub(crate) fn new(title: &str, width: DialogWidth) -> Self {
        let frame: Self = glib::Object::new();
        frame.set_title(title);
        frame.update_property(&[gtk::accessible::Property::Label(title)]);
        frame.set_width(width);
        frame
    }

    /// Shows `title` as the heading.
    pub(crate) fn set_title(&self, title: &str) {
        self.imp().title_label.set_text(title);
    }

    /// The heading, which names the window of a dialog window.
    pub(crate) fn title_label(&self) -> gtk::Label {
        self.imp().title_label.get()
    }

    /// The scrolling part: the body.
    pub(crate) fn scroller(&self) -> gtk::ScrolledWindow {
        self.imp().scroller.get()
    }

    /// Changes how wide the dialog is, as Properties does on its Previous
    /// versions tab (`versions-modal`).
    pub(crate) fn set_width(&self, width: DialogWidth) {
        self.imp().width.set(width);
        self.queue_resize();
    }

    /// How wide the dialog wants to be.
    pub(crate) fn width(&self) -> DialogWidth {
        self.imp().width.get()
    }

    /// The heading, for tests.
    #[cfg(test)]
    pub(crate) fn title(&self) -> glib::GString {
        self.imp().title_label.text()
    }

    /// The message under the heading, for tests.
    #[cfg(test)]
    pub(crate) fn message(&self) -> glib::GString {
        self.imp().message_label.text()
    }

    /// Shows `message` under the heading.
    pub(crate) fn set_message(&self, message: &str) {
        let label = &self.imp().message_label;
        label.set_text(message);
        label.set_visible(!message.is_empty());
    }

    /// The box the dialog's fields, notes and lists go in.
    pub(crate) fn body(&self) -> gtk::Box {
        self.imp().body.get()
    }

    /// Adds a button labelled `label` at the right of the footer.
    pub(crate) fn add_button(&self, label: &str, style: ButtonStyle) -> gtk::Button {
        let button = gtk::Button::with_label(label);
        button.add_css_class(style.css_class());
        self.imp().actions.append(&button);
        button
    }

    /// Puts `widget` at the left of the footer, before the buttons, such
    /// as the Extract dialog's information button.
    pub(crate) fn add_footer_start(&self, widget: &impl IsA<gtk::Widget>) {
        self.imp().actions.prepend(widget);
    }

    /// Adds a button labelled `label` that closes the dialog, then runs
    /// `then` (Close, Cancel, OK, or a button that hands over to another
    /// dialog or application).
    pub(crate) fn add_closing_button(
        &self,
        label: &str,
        style: ButtonStyle,
        then: impl Fn() + 'static,
    ) -> gtk::Button {
        let button = self.add_button(label, style);
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = frame)]
            self,
            move |_| {
                frame.close();
                then();
            }
        ));
        button
    }

    /// Shows why the last try failed; the dialog stays open for another.
    pub(crate) fn show_error(&self, message: &str) {
        let label = &self.imp().error_label;
        label.set_text(message);
        label.set_visible(!message.is_empty());
    }

    /// The error shown, empty without one, for tests.
    #[cfg(test)]
    pub(crate) fn error_text(&self) -> glib::GString {
        self.imp().error_label.text()
    }

    /// Dismisses the dialog for good: its layer lets go of it and it
    /// emits `closed`. Closing it again does nothing.
    pub(crate) fn close(&self) {
        if let Some(layer) = self
            .ancestor(super::DialogLayer::static_type())
            .and_downcast::<super::DialogLayer>()
        {
            layer.withdraw();
        }
        if self.imp().is_closed.replace(true) {
            return;
        }
        self.emit_by_name::<()>(CLOSED, &[]);
    }

    /// True once the dialog was dismissed for good, for tests.
    #[cfg(test)]
    pub(crate) fn is_closed(&self) -> bool {
        self.imp().is_closed.get()
    }

    /// Calls `callback` when the dialog is dismissed for good.
    pub(crate) fn connect_closed(&self, callback: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local(CLOSED, false, move |values| {
            let frame = values[0]
                .get::<Self>()
                .expect("the closed signal's first value is the frame");
            callback(&frame);
            None
        })
    }

    /// The labels of the footer's shown buttons, for tests.
    #[cfg(test)]
    pub(crate) fn button_labels(&self) -> Vec<String> {
        crate::window::children(&self.imp().actions.get())
            .filter(gtk::Widget::is_visible)
            .filter_map(|child| child.downcast::<gtk::Button>().ok())
            .filter_map(|button| button.label())
            .map(String::from)
            .collect()
    }
}
