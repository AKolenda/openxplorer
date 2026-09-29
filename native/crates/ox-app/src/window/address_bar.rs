// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar: breadcrumbs, or an editable address.
//!
//! Ports `renderNavigation`, `editAddress` and `finishAddress` in
//! `desktop/ui/app.js`: the location's icon, then crumbs divided by `/`
//! (`\` on SMB, none after the `/` root) as the current app draws them.
//! The crumbs scroll sideways (a plain mouse wheel scrolls them) and stay
//! scrolled to the current folder, so a deep path never widens the
//! window. Clicking blank space or pressing Ctrl+L edits the address;
//! leaving the entry returns to the breadcrumbs.
//!
//! Each crumb activates `win.go-to`, names itself "Go to …" for screen
//! readers, shows its full address as a tooltip and opens in a background
//! tab on a middle-click. GTK's own Left and Right focus movement already
//! walks between crumbs.
//!
//! [`AddressBar`] is a `GtkBox` subclass laid out by the template
//! `resources/ui/address-bar.ui`. The window hears what the user typed
//! through [`AddressBar::connect_submitted`] and
//! [`AddressBar::connect_cancelled`], never through the entry itself.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::Crumb;

use crate::icons::{self, Icon};

use super::gestures;
use super::widget_tree::remove_children;
use super::window_action::WindowAction;

/// The location icon: 16 pixels (ui-spec.md §4.2; the web app's was 17).
const ICON_SIZE: i32 = 16;

/// The glyph of the edit chevron.
const CHEVRON_GLYPH: i32 = 12;

/// The CSS class of a crumb button.
const CRUMB_CLASS: &str = "crumb";

/// The CSS class of the crumb a drop would go into (DND-011).
const CRUMB_DROP_CLASS: &str = "file-drop-active";

/// What the address bar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressMode {
    /// One button per ancestor.
    Crumbs,
    /// A text entry holding the address.
    Entry,
}

impl AddressMode {
    /// The name of the mode's page in the template's stack.
    const fn name(self) -> &'static str {
        match self {
            AddressMode::Crumbs => "crumbs",
            AddressMode::Entry => "entry",
        }
    }
}

/// One crumb as the bar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CrumbButton {
    /// The folder or page.
    pub crumb: Crumb,
    /// The full address, for the tooltip.
    pub address: String,
    /// The `/` or `\\` drawn before the crumb, if any
    /// ([`ox_core::location::crumb_divider`]).
    pub divider_before: Option<&'static str>,
}

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::AddressBar`]: the template's widgets.
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/address-bar.ui")]
    pub(crate) struct AddressBar {
        /// The location's glyph or colour folder.
        #[template_child]
        pub(super) icon: TemplateChild<gtk::Image>,
        /// The breadcrumbs or the entry ([`super::AddressMode`]).
        #[template_child]
        pub(super) stack: TemplateChild<gtk::Stack>,
        /// Scrolls the crumbs sideways.
        #[template_child]
        pub(super) crumb_scroll: TemplateChild<gtk::ScrolledWindow>,
        /// The crumb buttons and their dividers.
        #[template_child]
        pub(super) crumbs: TemplateChild<gtk::Box>,
        /// The editable address.
        #[template_child]
        pub(super) entry: TemplateChild<gtk::Entry>,
        /// The chevron that edits the address.
        #[template_child]
        pub(super) edit_button: TemplateChild<gtk::Button>,
        /// The protocols offered under the empty entry (NET-029).
        pub(super) protocols: std::cell::OnceCell<gtk::Popover>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AddressBar {
        const NAME: &'static str = "OxAddressBar";
        type Type = super::AddressBar;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(bar: &glib::subclass::InitializingObject<Self>) {
            bar.init_template();
        }
    }

    impl ObjectImpl for AddressBar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().finish_template();
        }

        fn dispose(&self) {
            // The chooser is parented to the entry, which does not know it.
            if let Some(protocols) = self.protocols.get() {
                gtk::prelude::WidgetExt::unparent(protocols);
            }
        }
    }

    impl WidgetImpl for AddressBar {}
    impl BoxImpl for AddressBar {}
}

glib::wrapper! {
    /// The address bar in the navigation row.
    pub(crate) struct AddressBar(ObjectSubclass<imp::AddressBar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl AddressBar {
    /// Adds what the template cannot express: the glyphs, the chevron's
    /// action, and the crumbs' scrolling and click handling.
    fn finish_template(&self) {
        let imp = self.imp();
        icons::set_icon(&imp.icon, Icon::FileFolder, ICON_SIZE);
        let chevron = icons::image(Icon::ChevronDown16, CHEVRON_GLYPH);
        imp.edit_button.set_child(Some(&chevron));
        WindowAction::Location.assign_to(&*imp.edit_button);
        self.keep_current_folder_visible();
        gestures::scroll_sideways_with_wheel(&imp.crumb_scroll);
        self.edit_on_blank_click();
        self.show_crumbs_when_focus_leaves();
        let protocols = super::address_protocols::protocol_chooser(&imp.entry);
        imp.protocols.set(protocols).expect("the template is finished once");
    }

    /// Scrolls to the last crumb whenever the crumbs or the width change,
    /// as `crumbs.scrollLeft = crumbs.scrollWidth` in app.js. `changed`
    /// fires for new bounds only, so the user can still scroll back.
    fn keep_current_folder_visible(&self) {
        self.imp()
            .crumb_scroll
            .hadjustment()
            .connect_changed(|adjustment| {
                adjustment.set_value(adjustment.upper() - adjustment.page_size());
            });
    }

    /// A click on blank space around the crumbs starts editing. Crumb
    /// buttons claim their own clicks, so only blank space gets here.
    fn edit_on_blank_click(&self) {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.connect_released(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |gesture, _, _, _| {
                if bar.mode() != AddressMode::Crumbs {
                    return;
                }
                WindowAction::Location.activate_from(&bar, None);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        ));
        self.imp().crumb_scroll.add_controller(click);
    }

    /// Leaving the entry, for another widget or another window, returns to
    /// the breadcrumbs (`blur` → `finishAddress`).
    fn show_crumbs_when_focus_leaves(&self) {
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| {
                // Hiding the entry makes it lose focus again; do nothing then.
                if bar.mode() == AddressMode::Entry {
                    bar.imp().stack.set_visible_child_name(AddressMode::Crumbs.name());
                }
            }
        ));
        self.imp().entry.add_controller(focus);
    }

    /// Calls `on_submitted` with the typed address when Enter is pressed
    /// in the entry.
    pub(super) fn connect_submitted(&self, on_submitted: impl Fn(&str) + 'static) {
        self.imp()
            .entry
            .connect_activate(move |entry| on_submitted(entry.text().as_str()));
    }

    /// Calls `on_cancelled` when Escape is pressed in the entry, which
    /// the entry then ignores.
    pub(super) fn connect_cancelled(&self, on_cancelled: impl Fn() + 'static) {
        let escape = gtk::EventControllerKey::new();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            on_cancelled();
            glib::Propagation::Stop
        });
        self.imp().entry.add_controller(escape);
    }

    /// What the bar shows now.
    pub(super) fn mode(&self) -> AddressMode {
        let shown = self.imp().stack.visible_child_name();
        if shown.as_deref() == Some(AddressMode::Entry.name()) {
            AddressMode::Entry
        } else {
            AddressMode::Crumbs
        }
    }

    /// Shows the location's crumbs, address text and icon. Text the user
    /// is typing is left alone.
    pub(super) fn show_location(&self, crumbs: &[CrumbButton], address: &str, icon: Icon) {
        let imp = self.imp();
        icons::set_icon(&imp.icon, icon, ICON_SIZE);
        self.set_tooltip_text(Some(&format!(
            "{address} · Click blank space or press Ctrl+L to edit"
        )));
        if self.mode() == AddressMode::Crumbs {
            imp.entry.set_text(address);
        }
        self.show_crumb_buttons(crumbs);
    }

    /// Replaces the crumb buttons, the last one announced as the current
    /// location (`aria-current` in app.js).
    fn show_crumb_buttons(&self, crumbs: &[CrumbButton]) {
        let crumb_box = &*self.imp().crumbs;
        remove_children(crumb_box);
        let last = crumbs.len().saturating_sub(1);
        for (index, crumb) in crumbs.iter().enumerate() {
            if let Some(divider) = crumb.divider_before {
                crumb_box.append(&divider_label(divider));
            }
            let button = crumb_button(crumb);
            if index == last {
                button.update_property(&[gtk::accessible::Property::Description("Current location")]);
            }
            crumb_box.append(&button);
        }
    }

    /// Replaces the entry with the breadcrumbs, resetting the entry to
    /// `address` so typed text is discarded.
    pub(super) fn show_crumbs(&self, address: &str) {
        let imp = self.imp();
        imp.entry.set_text(address);
        imp.stack.set_visible_child_name(AddressMode::Crumbs.name());
    }

    /// Shows the entry holding `address`, focused with all text selected.
    pub(super) fn edit(&self, address: &str) {
        let imp = self.imp();
        imp.entry.set_text(address);
        imp.stack.set_visible_child_name(AddressMode::Entry.name());
        imp.entry.grab_focus();
        imp.entry.select_region(0, -1);
    }

    /// The editable address, for tests.
    #[cfg(test)]
    pub(super) fn entry(&self) -> gtk::Entry {
        self.imp().entry.get()
    }

    /// The protocol chooser, for tests.
    #[cfg(test)]
    pub(super) fn protocol_chooser(&self) -> gtk::Popover {
        self.imp().protocols.get().expect("built with the bar").clone()
    }

    /// The folder of the crumb at (`x`, `y`) in the bar, where a drop
    /// would go; `None` elsewhere and while the address is edited.
    pub(super) fn crumb_location_at(&self, x: f64, y: f64) -> Option<String> {
        if self.mode() != AddressMode::Crumbs {
            return None;
        }
        let picked = self.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let crumb = std::iter::successors(Some(picked), WidgetExt::parent)
            .find(|widget| widget.has_css_class(CRUMB_CLASS))?;
        let target = crumb.downcast::<gtk::Button>().ok()?.action_target_value()?;
        target.str().map(str::to_owned)
    }

    /// Highlights the crumb of `folder` as where a drop would go, or none
    /// (DND-011).
    pub(super) fn highlight_crumb(&self, folder: Option<&str>) {
        let buttons = super::widget_tree::children(&*self.imp().crumbs)
            .filter_map(|widget| widget.downcast::<gtk::Button>().ok());
        for button in buttons {
            let target = button.action_target_value();
            let is_target = folder.is_some() && target.as_ref().and_then(glib::Variant::str) == folder;
            if is_target {
                button.add_css_class(CRUMB_DROP_CLASS);
            } else {
                button.remove_css_class(CRUMB_DROP_CLASS);
            }
        }
    }

    /// The crumb buttons shown, for tests.
    #[cfg(test)]
    pub(super) fn crumb_buttons(&self) -> Vec<gtk::Button> {
        super::widget_tree::children(&*self.imp().crumbs)
            .filter_map(|child| child.downcast::<gtk::Button>().ok())
            .collect()
    }

    /// The crumbs' horizontal scroll position, for tests.
    #[cfg(test)]
    pub(super) fn crumb_adjustment(&self) -> gtk::Adjustment {
        self.imp().crumb_scroll.hadjustment()
    }
}

/// The `/` or `\\` between crumbs, hidden from screen readers as
/// `aria-hidden` hides it in app.js.
fn divider_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .css_classes(["crumb-divider"])
        .build()
}

/// A crumb that opens its folder, named "Go to …" for screen readers.
fn crumb_button(crumb: &CrumbButton) -> gtk::Button {
    let uri = crumb.crumb.uri.as_str();
    let button = gtk::Button::builder()
        .label(&crumb.crumb.label)
        .tooltip_text(&crumb.address)
        .action_name(WindowAction::GoTo.detailed_name())
        .action_target(&uri.to_variant())
        .css_classes([CRUMB_CLASS])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&format!(
        "Go to {}",
        crumb.crumb.label
    ))]);
    gestures::open_folder_on_middle_click(&button, uri);
    button
}
