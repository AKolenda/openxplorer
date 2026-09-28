// SPDX-License-Identifier: AGPL-3.0-only
//! The tab strip in the title bar.
//!
//! Ports `renderTabs` in `desktop/ui/app.js` and `.tab` in `style.css`:
//! each tab is 215 pixels wide with its icon, title and close button, and
//! tabs shrink toward 100 pixels and then scroll sideways
//! ([`TabLayout`](super::tab_layout::TabLayout)), so opening many tabs
//! never widens the window. The whole tab is the click target, as in
//! app.js: it is one focusable widget announced as a tab of the "Folder
//! tabs" list with its selected state; a click or Enter shows it and a
//! middle-click closes it. The close button inside claims its own clicks.
//!
//! [`TabStrip`] is a widget subclass whose scroller and tab list are the
//! template `resources/ui/tab-strip.ui`; the tabs are built here.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::icons::{self, Art, ArtImage, Icon};

use super::gestures;
use super::session::TabId;
use super::widget_tree::remove_children;
use super::window_action::WindowAction;

/// The tab icon's edge: 16 pixels (ui-spec.md I03; the web app's was 17).
const ICON_SIZE: i32 = 16;
/// The gap between the icon and the title (ui-spec.md S06).
const ICON_TO_TITLE: i32 = 10;
/// The glyph of a tab's close button.
const CLOSE_GLYPH: i32 = 12;

/// The CSS class of a tab's icon.
const TAB_ICON_CLASS: &str = "tab-icon";

/// What the strip shows of one tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabView {
    /// The tab shown.
    pub id: TabId,
    /// The tab's title.
    pub title: String,
    /// The full address.
    pub tooltip: String,
    /// The tab's icon: a glyph for a landing page or a device, else the
    /// folder, on the network bar for a share.
    pub icon: Art,
    /// The tab is in front.
    pub active: bool,
}

mod imp {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use crate::window::gestures;
    use crate::window::tab_layout::TabLayout;

    /// Private state of [`super::TabStrip`]: the template's widgets.
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/tab-strip.ui")]
    pub(crate) struct TabStrip {
        /// The strip, scrolling sideways when the tabs do not fit.
        #[template_child]
        pub(super) scroller: TemplateChild<gtk::ScrolledWindow>,
        /// Scrolls the active tab into view.
        #[template_child]
        pub(super) viewport: TemplateChild<gtk::Viewport>,
        /// The tabs, announced as the "Folder tabs" list.
        #[template_child]
        pub(super) tab_list: TemplateChild<gtk::Box>,
        /// Shares the strip's width between the tabs.
        #[template_child]
        pub(super) layout: TemplateChild<TabLayout>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TabStrip {
        const NAME: &'static str = "OxTabStrip";
        type Type = super::TabStrip;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            // GtkBuilder finds the template's own types by name.
            TabLayout::ensure_type();
            klass.bind_template();
        }

        fn instance_init(strip: &glib::subclass::InitializingObject<Self>) {
            strip.init_template();
        }
    }

    impl ObjectImpl for TabStrip {
        fn constructed(&self) {
            self.parent_constructed();
            gestures::scroll_sideways_with_wheel(&self.scroller);
        }

        fn dispose(&self) {
            self.dispose_template();
        }
    }

    impl WidgetImpl for TabStrip {}
}

glib::wrapper! {
    /// The tab strip in the title bar.
    pub(crate) struct TabStrip(ObjectSubclass<imp::TabStrip>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TabStrip {
    /// Makes tabs `width` pixels wide when there is room: 215, or less in
    /// a narrow window.
    pub(super) fn set_tab_width(&self, width: i32) {
        self.imp().layout.set_tab_width(width);
    }

    /// Replaces the tabs with `tabs` and scrolls the active one into view.
    pub(super) fn show(&self, tabs: &[TabView]) {
        let imp = self.imp();
        remove_children(&*imp.tab_list);
        let mut active = None;
        for tab in tabs {
            let widget = tab_widget(tab);
            imp.tab_list.append(&widget);
            if tab.active {
                active = Some(widget);
            }
        }
        let Some(active) = active else {
            return;
        };
        // After the new tabs are laid out, so their positions are known.
        let viewport = imp.viewport.get();
        glib::idle_add_local_once(glib::clone!(
            #[weak]
            viewport,
            move || viewport.scroll_to(&active, None)
        ));
    }

    /// The tab list, for tests.
    #[cfg(test)]
    pub(super) fn tab_list(&self) -> gtk::Box {
        self.imp().tab_list.get()
    }
}

fn tab_icon(icon: Art) -> ArtImage {
    let image = ArtImage::new(icon, ICON_SIZE);
    image.add_css_class(TAB_ICON_CLASS);
    image
}

/// The widget of `tab`: its icon, title and close button, one focusable
/// target that shows the tab on a click or Enter and closes it on a
/// middle-click.
fn tab_widget(tab: &TabView) -> gtk::Box {
    let widget = gtk::Box::builder()
        .spacing(ICON_TO_TITLE)
        .focusable(true)
        .accessible_role(gtk::AccessibleRole::Tab)
        .tooltip_text(&tab.tooltip)
        .css_classes(["tab"])
        .build();
    if tab.active {
        widget.add_css_class("active");
    }
    widget.update_property(&[gtk::accessible::Property::Label(&tab.title)]);
    widget.update_state(&[gtk::accessible::State::Selected(Some(tab.active))]);
    widget.append(&tab_icon(tab.icon));
    widget.append(&title(&tab.title));
    widget.append(&close_button(tab));
    let id = tab.id.to_variant();
    widget.add_controller(select_on_click(id.clone()));
    widget.add_controller(select_on_enter(id.clone()));
    widget.add_controller(gestures::middle_click(move |gesture, _, _| {
        run_on(gesture.widget(), WindowAction::CloseTabById, &id);
    }));
    widget
}

/// Runs the tab action `action` on the tab `id` from `widget`, when the
/// gesture still has one.
fn run_on(widget: Option<gtk::Widget>, action: WindowAction, id: &glib::Variant) {
    if let Some(widget) = widget {
        action.activate_from(&widget, Some(id));
    }
}

/// A primary click anywhere on the tab shows it.
fn select_on_click(id: glib::Variant) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.connect_pressed(move |gesture, _, _, _| {
        run_on(gesture.widget(), WindowAction::SelectTab, &id);
    });
    click
}

/// Enter or Space on a focused tab shows it.
fn select_on_enter(id: glib::Variant) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |keys, key, _, _| {
        let activates = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space);
        if !activates {
            return glib::Propagation::Proceed;
        }
        run_on(keys.widget(), WindowAction::SelectTab, &id);
        glib::Propagation::Stop
    });
    keys
}

fn title(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["tab-title"])
        .build()
}

/// The tab's close button, named `Close <title>` for screen readers.
fn close_button(tab: &TabView) -> gtk::Button {
    let close = gtk::Button::builder()
        .child(&icons::image(Icon::Dismiss16, CLOSE_GLYPH))
        .tooltip_text("Close tab")
        .action_name(WindowAction::CloseTabById.detailed_name())
        .action_target(&tab.id.to_variant())
        .focus_on_click(false)
        .valign(gtk::Align::Center)
        .css_classes(["tab-close"])
        .build();
    let name = format!("Close {}", tab.title);
    close.update_property(&[gtk::accessible::Property::Label(&name)]);
    close
}
