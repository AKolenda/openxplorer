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
//! middle-click closes it; a right-click opens the tab's menu
//! ([`super::tab_menu`]). The close button inside claims its own clicks.
//!
//! [`TabStrip`] is a widget subclass whose scroller and tab list are the
//! template `resources/ui/tab-strip.ui`; the tabs are built here. What it
//! shows while tabs and files are dragged is [`drag_marks`]'s.

mod drag_marks;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use crate::icons::{self, Art, ArtImage, Icon};

use super::gestures;
use super::menu_popover::MenuEntry;
use super::session::TabId;
use super::tab_menu::tab_menu;
use super::widget_tree::remove_children;
use super::window_action::WindowAction;

pub(super) use drag_marks::TabInsertion;

/// The tab icon's edge: 16 pixels (ui-spec.md I03; the web app's was 17).
const ICON_SIZE: i32 = 16;
/// The gap between the icon and the title (ui-spec.md S06).
const ICON_TO_TITLE: i32 = 10;
/// The glyph of a tab's close button.
const CLOSE_GLYPH: i32 = 12;

/// The clock in a snapshot tab's badge (`icon('clock', 12)`).
const SNAPSHOT_BADGE_GLYPH: i32 = 12;

/// The CSS class of a tab's icon.
const TAB_ICON_CLASS: &str = "tab-icon";

/// What the strip shows of one tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabView {
    /// The tab shown.
    pub id: TabId,
    /// The location it shows.
    pub uri: String,
    /// The tab's title.
    pub title: String,
    /// The full address.
    pub tooltip: String,
    /// The tab's icon: a glyph for a landing page or a device, else the
    /// folder, on the network bar for a share.
    pub icon: Art,
    /// The tab is in front.
    pub active: bool,
    /// The name of the snapshot the tab shows a previous version from,
    /// which marks the tab (PROP-022).
    pub previous_version: Option<String>,
}

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use crate::window::gestures;
    use crate::window::menu_popover::MenuPopover;
    use crate::window::tab_layout::TabLayout;

    use super::TabView;

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
        /// The tabs' context menu, built by `constructed`.
        pub(super) menu: OnceCell<MenuPopover>,
        /// Each tab shown and its widget, left to right.
        pub(super) shown: RefCell<Vec<(TabView, gtk::Box)>>,
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
            let menu = MenuPopover::new(Vec::new());
            menu.set_offset(0, 0);
            menu.set_parent(&*self.obj());
            self.menu.set(menu).expect("constructed runs once per object");
        }

        fn dispose(&self) {
            if let Some(menu) = self.menu.get() {
                menu.unparent();
            }
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
    pub(super) fn set_tabs(&self, tabs: &[TabView]) {
        let imp = self.imp();
        remove_children(&*imp.tab_list);
        let mut active = None;
        let mut shown = Vec::with_capacity(tabs.len());
        for tab in tabs {
            let widget = tab_widget(tab);
            imp.tab_list.append(&widget);
            if tab.active {
                active = Some(widget.clone());
            }
            shown.push((tab.clone(), widget));
        }
        imp.shown.replace(shown);
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

    /// The tab at (`x`, `y`) in the strip, if any: where a file drop
    /// would go (TAB-018).
    pub(super) fn tab_at(&self, x: f64, y: f64) -> Option<TabView> {
        let picked = self.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let shown = self.imp().shown.borrow();
        std::iter::successors(Some(picked), WidgetExt::parent).find_map(|widget| {
            shown
                .iter()
                .find(|(_, tab_widget)| *tab_widget.upcast_ref::<gtk::Widget>() == widget)
                .map(|(tab, _)| tab.clone())
        })
    }

    /// Opens the tab menu `entries` at `point` in the strip.
    fn show_menu(&self, entries: Vec<MenuEntry>, point: graphene::Point) {
        let Some(menu) = self.imp().menu.get() else {
            return;
        };
        menu.set_entries(entries);
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let target = gdk::Rectangle::new(point.x() as i32, point.y() as i32, 1, 1);
        menu.set_pointing_to(Some(&target));
        menu.popup();
    }

    /// The tab list, for tests.
    #[cfg(test)]
    pub(super) fn tab_list(&self) -> gtk::Box {
        self.imp().tab_list.get()
    }

    /// The tabs' context menu, for tests.
    #[cfg(test)]
    pub(super) fn menu(&self) -> super::menu_popover::MenuPopover {
        self.imp()
            .menu
            .get()
            .expect("constructed builds the menu")
            .clone()
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
    if let Some(snapshot) = &tab.previous_version {
        mark_previous_version(&widget, &tab.title, snapshot);
    }
    widget.append(&close_button(tab));
    let id = tab.id.to_variant();
    widget.add_controller(select_on_click(id.clone()));
    widget.add_controller(select_on_enter(id.clone()));
    widget.add_controller(gestures::middle_click(move |gesture, _, _| {
        run_on(gesture.widget(), WindowAction::CloseTabById, &id);
    }));
    widget.add_controller(menu_on_right_click(tab.id, tab.uri.clone()));
    widget
}

/// A right-click on the tab `id`, which shows `uri`, opens its menu
/// there.
fn menu_on_right_click(id: TabId, uri: String) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    click.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(tab) = gesture.widget() else {
            return;
        };
        let Some(strip) = tab.ancestor(TabStrip::static_type()).and_downcast::<TabStrip>() else {
            return;
        };
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let in_tab = graphene::Point::new(x as f32, y as f32);
        if let Some(point) = tab.compute_point(&strip, &in_tab) {
            strip.show_menu(tab_menu(id, &uri), point);
        }
    });
    click
}

/// Marks a tab that shows a previous version from the snapshot `snapshot`:
/// the amber top edge and the "Previous version" badge (`renderTabs` in
/// app.js).
fn mark_previous_version(widget: &gtk::Box, title: &str, snapshot: &str) {
    widget.add_css_class("snapshot-tab");
    let badge = gtk::Box::builder()
        .spacing(4)
        .valign(gtk::Align::Center)
        .tooltip_text(format!("Previous version · {snapshot}"))
        .css_classes(["snapshot-tab-badge"])
        .build();
    badge.append(&icons::image(Icon::Clock, SNAPSHOT_BADGE_GLYPH));
    badge.append(&gtk::Label::new(Some("Previous version")));
    widget.append(&badge);
    let name = format!("{title} — Previous version — {snapshot}");
    widget.update_property(&[gtk::accessible::Property::Label(&name)]);
}

/// Runs the tab action `action` on the tab `id` from `widget`, when the
/// gesture still has one.
fn run_on(widget: Option<gtk::Widget>, action: WindowAction, id: &glib::Variant) {
    if let Some(widget) = widget {
        action.activate_from(&widget, Some(id));
    }
}

/// A primary click anywhere on the tab shows it, on release as `click` in
/// app.js. Showing a tab draws the strip anew, which on the press would
/// cancel a drag of the tab before it starts; a drag cancels the click, so
/// a dragged tab is not shown (TAB-004).
fn select_on_click(id: glib::Variant) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.connect_released(move |gesture, _, _, _| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::descendants;

    /// A tab on a snapshot folder, showing the previous version
    /// `previous_version` when there is one.
    fn tab_view(previous_version: Option<&str>) -> TabView {
        TabView {
            id: TabId::from_raw(1),
            uri: "file:///srv/Documents/.snapshot/daily/Plans".to_owned(),
            title: "Plans".to_owned(),
            tooltip: "/srv/Documents/.snapshot/daily/Plans".to_owned(),
            icon: Art::Folder,
            active: true,
            previous_version: previous_version.map(str::to_owned),
        }
    }

    /// parity: TAB-011
    #[gtk::test]
    fn a_tab_in_a_snapshot_carries_the_previous_version_badge() {
        let widget = tab_widget(&tab_view(Some("daily")));

        assert!(
            widget.has_css_class("snapshot-tab"),
            "the amber edge and the wider tab"
        );
        let badge = descendants::<gtk::Box>(&widget)
            .into_iter()
            .find(|child| child.has_css_class("snapshot-tab-badge"))
            .expect("a snapshot tab has its badge");
        assert_eq!(badge.tooltip_text().as_deref(), Some("Previous version · daily"));
        let texts: Vec<String> = descendants::<gtk::Label>(&badge)
            .iter()
            .map(|label| label.text().to_string())
            .collect();
        assert_eq!(texts, ["Previous version"]);
    }

    /// parity: TAB-010
    #[gtk::test]
    fn a_live_tab_has_no_badge_a_close_button_and_an_ellipsized_title() {
        let widget = tab_widget(&tab_view(None));

        assert!(!widget.has_css_class("snapshot-tab"));
        let close = descendants::<gtk::Button>(&widget)
            .into_iter()
            .next()
            .expect("a tab has its close button");
        assert_eq!(close.tooltip_text().as_deref(), Some("Close tab"));
        let title = descendants::<gtk::Label>(&widget)
            .into_iter()
            .next()
            .expect("a tab shows its title");
        assert_eq!(title.ellipsize(), gtk::pango::EllipsizeMode::End);
    }
}
