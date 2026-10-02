// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar: breadcrumbs, or an editable address.
//!
//! Ports `renderNavigation`, `editAddress` and `finishAddress` in
//! `v2.0.0:desktop/ui/app.js`: the location's icon, then crumbs divided by `/`
//! (`\` on SMB, none after the `/` root) as the current app draws them.
//! The crumbs scroll sideways (a plain mouse wheel scrolls them) and stay
//! scrolled to the current folder, so a deep path never widens the
//! window. Clicking blank space or pressing Ctrl+L edits the address;
//! leaving the entry returns to the breadcrumbs.
//!
//! Each crumb activates `win.go-to`, names itself "Go to …" for screen
//! readers, shows its full address as a tooltip and opens in a background
//! tab on a middle-click; as in Dolphin, a Ctrl+click opens it in a tab
//! and a Shift+click in a window. Left and Right move focus between the
//! crumbs and stop at either end. The address can stay editable text
//! (NAV-029), and the `suggestions` module lists the typed history and
//! completions under the entry.
//!
//! [`AddressBar`] is a `GtkBox` subclass laid out by the template
//! `resources/ui/address-bar.ui`. The window hears what the user typed
//! through [`AddressBar::connect_submitted`] and
//! [`AddressBar::connect_cancelled`], never through the entry itself.

mod crumb_input;
mod suggestions;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::location::Crumb;

use crate::icons::{self, Icon};

use super::crumb_menus;
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
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
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
        /// The address stays editable text instead of crumbs (NAV-029).
        pub(super) always_editable: Cell<bool>,
        /// The typed history and completions under the entry
        /// ([`super::suggestions`]).
        pub(super) suggestion_popover: OnceCell<gtk::Popover>,
        /// The rows of the list.
        pub(super) suggestion_list: OnceCell<gtk::ListBox>,
        /// Addresses applied with Enter, most recent first (NAV-043).
        pub(super) typed_history: RefCell<Vec<String>>,
        /// The entry's text is being set from the list, not typed.
        pub(super) quiet_change: Cell<bool>,
        /// The protocols offered under the empty entry (NET-029).
        pub(super) protocols: OnceCell<gtk::Popover>,
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
            crate::i18n::translate_template(&*self.obj(), "address-bar.ui");
            self.obj().finish_template();
        }

        fn dispose(&self) {
            if let Some(popover) = self.suggestion_popover.get() {
                popover.unparent();
            }
            // The chooser is parented to the entry, which does not know it.
            if let Some(protocols) = self.protocols.get() {
                protocols.unparent();
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
        WindowAction::AddressHistory.assign_to(&*imp.edit_button);
        self.keep_current_folder_visible();
        gestures::scroll_sideways_with_wheel(&imp.crumb_scroll);
        self.edit_on_blank_click();
        self.show_crumbs_when_focus_leaves();
        self.move_between_crumbs_with_arrows();
        self.add_location_menu();
        imp.entry.set_extra_menu(Some(&address_options_menu()));
        self.add_suggestions();
        let protocols = super::address_protocols::protocol_chooser(&imp.entry);
        imp.protocols
            .set(protocols)
            .expect("the template is finished once");
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
                if bar.mode() == AddressMode::Entry && !bar.is_always_editable() {
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
        self.set_tooltip_text(Some(&ox_core::i18n::format_message(
            "{address} · Click blank space or press Ctrl+L to edit",
            &[("address", &(address).to_string())],
        )));
        let typing = self.mode() == AddressMode::Entry && imp.entry.focus_child().is_some();
        if !typing {
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
            let next = crumbs.get(index + 1).map(|next| next.crumb.uri.as_str());
            if let Some(divider) = crumb.divider_before {
                let label = divider_label(divider);
                if let Some(previous) = index.checked_sub(1).and_then(|previous| crumbs.get(previous)) {
                    crumb_menus::open_subfolders_on_click(&label, &previous.crumb.uri, &crumb.crumb.uri);
                }
                crumb_box.append(&label);
            }
            let button = crumb_button(crumb);
            self.add_crumb_menu_and_wheel(&button, &crumb.crumb.uri, next);
            if index == last {
                button.update_property(&[gtk::accessible::Property::Description(&ox_core::i18n::gettext(
                    "Current location",
                ))]);
            }
            crumb_box.append(&button);
        }
    }

    /// Replaces the entry with the breadcrumbs, resetting the entry to
    /// `address` so typed text is discarded. An address kept editable
    /// stays text.
    pub(super) fn show_crumbs(&self, address: &str) {
        let imp = self.imp();
        imp.entry.set_text(address);
        let mode = if self.is_always_editable() {
            AddressMode::Entry
        } else {
            AddressMode::Crumbs
        };
        imp.stack.set_visible_child_name(mode.name());
    }

    /// Keeps the address as editable text instead of crumbs, or returns to
    /// the crumbs (NAV-029).
    pub(super) fn set_always_editable(&self, editable: bool) {
        self.imp().always_editable.set(editable);
        let address = self.imp().entry.text();
        self.show_crumbs(&address);
    }

    /// Whether the address stays editable text.
    pub(super) fn is_always_editable(&self) -> bool {
        self.imp().always_editable.get()
    }

    /// Shows the entry holding `address`, focused with all text selected.
    pub(super) fn edit(&self, address: &str) {
        let imp = self.imp();
        imp.entry.set_text(address);
        imp.stack.set_visible_child_name(AddressMode::Entry.name());
        imp.entry.grab_focus();
        imp.entry.select_region(0, -1);
    }

    /// Whether the entry has keyboard focus with its whole text selected,
    /// as Ctrl+L leaves it: a second Ctrl+L then returns to the crumbs, as
    /// in Dolphin (NAV-028).
    pub(super) fn edits_whole_address(&self) -> bool {
        let entry = &*self.imp().entry;
        let focused = entry.has_focus() || entry.focus_child().is_some();
        let length = i32::try_from(entry.text().chars().count()).unwrap_or(i32::MAX);
        self.mode() == AddressMode::Entry && focused && entry.selection_bounds() == Some((0, length))
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
        let target = self.crumb_at(x, y)?.action_target_value()?;
        target.str().map(str::to_owned)
    }

    /// The crumb at (`x`, `y`) in the bar; `None` elsewhere and while the
    /// address is edited.
    pub(super) fn crumb_at(&self, x: f64, y: f64) -> Option<gtk::Button> {
        if self.mode() != AddressMode::Crumbs {
            return None;
        }
        let picked = self.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let crumb = std::iter::successors(Some(picked), WidgetExt::parent)
            .find(|widget| widget.has_css_class(CRUMB_CLASS))?;
        crumb.downcast::<gtk::Button>().ok()
    }

    /// Opens `text` as though it was typed into the address and Enter
    /// pressed.
    pub(super) fn submit_text(&self, text: &str) {
        let entry = &*self.imp().entry;
        entry.set_text(text);
        entry.emit_by_name::<()>("activate", &[]);
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

    /// The crumb buttons shown, first to last.
    pub(super) fn crumb_buttons(&self) -> Vec<gtk::Button> {
        super::widget_tree::children(&*self.imp().crumbs)
            .filter_map(|child| child.downcast::<gtk::Button>().ok())
            .collect()
    }

    /// Whether the crumbs are wider than the bar, so the wheel scrolls
    /// them rather than switching folders (NAV-022).
    pub(super) fn crumbs_overflow(&self) -> bool {
        let adjustment = self.imp().crumb_scroll.hadjustment();
        adjustment.upper() > adjustment.page_size() + 0.5
    }

    /// The crumbs' horizontal scroll position, for tests.
    #[cfg(test)]
    pub(super) fn crumb_adjustment(&self) -> gtk::Adjustment {
        self.imp().crumb_scroll.hadjustment()
    }
}

/// The address bar's options, added to the entry's own menu so an
/// address kept editable can return to crumbs (NAV-029).
fn address_options_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let editable = WindowAction::EditableLocation.detailed_name();
    menu.append(Some("Keep address editable"), Some(&editable));
    menu.append(
        Some("Show full path"),
        Some(&WindowAction::ShowFullPath.detailed_name()),
    );
    menu
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
    button.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::format_message(
        "Go to {label}",
        &[("label", &(crumb.crumb.label).to_string())],
    ))]);
    gestures::open_folder_on_middle_click(&button, uri);
    crumb_input::open_elsewhere_on_modified_click(&button, uri);
    button
}
