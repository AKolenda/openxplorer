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

/// The edge of a message box's icon, as Windows draws it.
const MESSAGE_ICON: i32 = 32;

/// The close glyph of a message box's caption, as in a dialog window's
/// title bar.
const CLOSE_GLYPH: i32 = 12;

/// The characters a message box's line holds before it wraps: Windows'
/// Rename question keeps "If you change a file name extension, the file
/// might become unusable." on one line, and a longer line, such as a long
/// file name, wraps.
const MESSAGE_BOX_LINE: i32 = 62;

/// How wide a dialog is, as the width classes of `style.css` set it
/// (ui-spec.md §4.11: 510 by default, the archive browser 720).
/// Properties are windows of their own ([`crate::properties::PropertiesWindow`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogWidth {
    /// A question or a short form.
    Standard,
    /// The archive browser.
    Archive,
    /// A Windows message box ([`DialogFrame::make_message_box`]): as wide
    /// as its words, its caption and its buttons need, never wider than
    /// its message wraps at.
    MessageBox,
}

impl DialogWidth {
    /// The width in pixels, before the window's own width limits it;
    /// `None` for a message box, which is as wide as its content.
    pub(crate) const fn pixels(self) -> Option<i32> {
        match self {
            DialogWidth::Standard => Some(510),
            DialogWidth::Archive => Some(720),
            DialogWidth::MessageBox => None,
        }
    }
}

mod imp {
    use std::cell::{Cell, RefCell};
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
        /// The icon left of the message, shown only in a message box.
        #[template_child]
        pub(super) message_icon: TemplateChild<gtk::Image>,
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
        /// A message box's caption, with its title and close button.
        pub(super) caption: RefCell<Option<gtk::Label>>,
        /// A message box's Alt+letter keys, one for each button.
        pub(super) mnemonics: RefCell<Option<gtk::ShortcutController>>,
    }

    impl Default for DialogFrame {
        fn default() -> Self {
            Self {
                column: TemplateChild::default(),
                title_label: TemplateChild::default(),
                message_label: TemplateChild::default(),
                message_icon: TemplateChild::default(),
                scroller: TemplateChild::default(),
                body: TemplateChild::default(),
                error_label: TemplateChild::default(),
                actions: TemplateChild::default(),
                width: Cell::new(DialogWidth::Standard),
                is_closed: Cell::new(false),
                caption: RefCell::default(),
                mnemonics: RefCell::default(),
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

/// The key that types `character` in lower case, for any script: GDK's
/// keyval for its Unicode code point, read through a `GValue` as GDK keeps
/// keyvals, so no unchecked conversion is needed.
pub(crate) fn alt_key(character: char) -> Option<gtk::gdk::Key> {
    let lower = character.to_lowercase().next()?;
    let keyval = gtk::gdk::unicode_to_keyval(u32::from(lower));
    keyval.to_value().get::<gtk::gdk::Key>().ok()
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

    /// Shows `title` as the heading, or in a message box's caption.
    pub(crate) fn set_title(&self, title: &str) {
        self.imp().title_label.set_text(title);
        if let Some(caption) = self.imp().caption.borrow().as_ref() {
            caption.set_text(title);
        }
    }

    /// Lays the dialog out as a Windows message box, as Windows 11 asks
    /// "Rename" before an extension changes and "Confirm Save As" before
    /// a file is replaced: a caption with the title and a close button,
    /// `icon` left of the message in the text colour, no heading or body,
    /// and the buttons on the footer; the box is as wide as that needs,
    /// so a long message wraps rather than widening it, and no empty room
    /// is left. Alt with a button's first letter presses it (Alt+Y for
    /// Yes, Alt+N for No). Its close button, like Escape, dismisses it.
    /// Call it before adding the buttons.
    pub(crate) fn make_message_box(&self, icon: crate::icons::Icon) {
        let imp = self.imp();
        if imp.caption.borrow().is_some() {
            return;
        }
        self.add_css_class("message-box");
        self.set_width(DialogWidth::MessageBox);
        imp.title_label.set_visible(false);
        imp.scroller.set_visible(false);
        crate::icons::set_icon(&imp.message_icon, icon, MESSAGE_ICON);
        imp.message_icon.set_visible(true);
        imp.message_label.set_max_width_chars(MESSAGE_BOX_LINE);
        let (caption, label) = self.message_box_caption();
        imp.column.prepend(&caption);
        imp.caption.replace(Some(label));
        let mnemonics = gtk::ShortcutController::new();
        self.add_controller(mnemonics.clone());
        imp.mnemonics.replace(Some(mnemonics));
    }

    /// A message box's caption: the title, small, and a close button the
    /// height of the caption, as a Windows message box's title bar. The
    /// close button is not a keyboard stop, as in Windows.
    fn message_box_caption(&self) -> (gtk::Box, gtk::Label) {
        let caption = gtk::Box::builder()
            .css_classes(["ox-titlebar", "dialog-titlebar", "message-box-caption"])
            .build();
        let label = gtk::Label::builder()
            .label(self.imp().title_label.text())
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(1)
            .css_classes(["dialog-window-title"])
            .build();
        caption.append(&label);
        let close_text = ox_core::i18n::gettext("Close");
        let close = gtk::Button::builder()
            .child(&crate::icons::image(crate::icons::Icon::Dismiss, CLOSE_GLYPH))
            .css_classes(["caption", "close"])
            .focus_on_click(false)
            .focusable(false)
            .tooltip_text(close_text.as_str())
            .build();
        close.update_property(&[gtk::accessible::Property::Label(&close_text)]);
        close.connect_clicked(glib::clone!(
            #[weak(rename_to = frame)]
            self,
            move |_| frame.dismiss()
        ));
        caption.append(&close);
        (caption, label)
    }

    /// Dismisses the dialog as Escape does: a dialog window closes, which
    /// answers "cancelled", and a dialog on a layer closes.
    fn dismiss(&self) {
        let in_layer = self.ancestor(super::DialogLayer::static_type()).is_some();
        match self.root().and_downcast::<gtk::Window>() {
            Some(window) if !in_layer => window.close(),
            _ => self.close(),
        }
    }

    /// Lets Alt with the first character of `button`'s label press it, in
    /// a message box: Alt+Y for Yes, and in a translation whose label does
    /// not start with a Latin letter, such as `zh_CN`'s 是, Alt with that
    /// character ([`Self::also_press_with`] adds Windows' Latin key there).
    fn add_mnemonic(&self, button: &gtk::Button) {
        let first = button.label().and_then(|label| label.chars().next());
        if let Some(character) = first.filter(|character| character.is_alphanumeric()) {
            self.add_mnemonic_key(button, character);
        }
    }

    /// Lets Alt with `letter` press `button` too, in a message box, as
    /// Windows' translated message boxes keep "是(Y)" and "否(N)": the
    /// English mnemonic of the button's message id.
    pub(crate) fn also_press_with(&self, button: &gtk::Button, letter: char) {
        self.add_mnemonic_key(button, letter);
    }

    /// Lets Alt with `character` press `button`, once.
    fn add_mnemonic_key(&self, button: &gtk::Button, character: char) {
        let Some(mnemonics) = self.imp().mnemonics.borrow().clone() else {
            return;
        };
        let Some(key) = alt_key(character) else {
            return;
        };
        let trigger = gtk::KeyvalTrigger::new(key, gtk::gdk::ModifierType::ALT_MASK);
        let taken = (0..mnemonics.n_items())
            .filter_map(|position| mnemonics.item(position).and_downcast::<gtk::Shortcut>())
            .filter_map(|shortcut| shortcut.trigger())
            .any(|existing| existing.equal(&trigger));
        if taken {
            return;
        }
        let action = gtk::CallbackAction::new(glib::clone!(
            #[weak]
            button,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _| {
                if !button.is_sensitive() || !button.is_visible() {
                    return glib::Propagation::Proceed;
                }
                button.emit_clicked();
                glib::Propagation::Stop
            }
        ));
        mnemonics.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
    }

    /// The width the dialog wants: its width class, or for a message box
    /// the natural width of its content.
    pub(crate) fn wanted_width(&self) -> i32 {
        self.width()
            .pixels()
            .unwrap_or_else(|| self.measure(gtk::Orientation::Horizontal, -1).1)
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
        if self.imp().caption.borrow().is_some() {
            // One size in every host: the dialog window's and the dialog
            // layer's own button rules differ (message-box-button,
            // dialogs.css).
            button.add_css_class("message-box-button");
        }
        self.add_mnemonic(&button);
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
