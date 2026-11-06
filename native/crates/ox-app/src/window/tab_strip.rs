// SPDX-License-Identifier: AGPL-3.0-only
//! The tab strip in the title bar.
//!
//! Ports `renderTabs` in `v2.0.0:desktop/ui/app.js` and `.tab` in `style.css`:
//! each tab is 215 pixels wide with its icon, title and close button, and
//! tabs shrink toward 100 pixels and then scroll sideways
//! ([`TabLayout`](super::tab_layout::TabLayout)), so opening many tabs
//! never widens the window. The whole tab is the click target, as in
//! app.js: it is one widget announced as a tab of the "Folder tabs" list
//! with its selected state; a press or Enter shows it, a double-click
//! opens a copy of it and a middle-click closes it; a right-click opens
//! the tab's menu ([`super::tab_menu`]). The close button inside claims
//! its own clicks. Only the tab in front takes keyboard focus (a roving
//! tab stop, ACC-008), and the arrow keys, Home and End show another tab
//! and move focus with it, as GTK's own tab bars do.
//!
//! [`TabStrip`] is a widget subclass whose scroller and tab list are the
//! template `resources/ui/tab-strip.ui`; the tabs are built here. What it
//! shows while tabs and files are dragged is [`drag_marks`]'s, and the
//! separators between tabs and the curves at the foot of the tab in front,
//! which a tab cannot show itself, are placed by [`edges`].

mod carried_tab;
mod drag_marks;
mod edge_scroll;
mod edges;
mod live_drag;

use std::cell::Cell;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use crate::icons::{self, Art, ArtImage, Icon};

use super::gestures;
use super::menu_popover::MenuEntry;
use super::session::TabId;
use super::tab_menu::tab_menu;
use super::window_action::WindowAction;

pub(in crate::window) use carried_tab::MARGIN as CARRIED_MARGIN;
pub(in crate::window) use live_drag::Landing;

/// The tab icon's edge: 16 pixels (ui-spec.md I03; the web app's was 17).
const ICON_SIZE: i32 = 16;
/// The gap between the icon and the title (ui-spec.md S06).
const ICON_TO_TITLE: i32 = 10;
/// The glyph of a tab's close button.
const CLOSE_GLYPH: i32 = 12;

/// The clock in a snapshot tab's badge (`icon('clock', 12)`).
const SNAPSHOT_BADGE_GLYPH: i32 = 12;

/// The share of the window's width the strip takes at most, in percent.
const MAX_STRIP_PERCENT: i32 = 70;

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
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use crate::window::gestures;
    use crate::window::menu_popover::MenuPopover;
    use crate::window::session::TabId;
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
        /// The list of open windows, shown below the tab in front.
        pub(super) windows_menu: OnceCell<MenuPopover>,
        /// The widgets the skin draws the separators and the curves at the
        /// foot of the tab in front on, built by `constructed`.
        pub(super) edges: OnceCell<super::edges::Edges>,
        /// Each tab shown and its widget, left to right.
        pub(super) shown: RefCell<Vec<(TabView, gtk::Box)>>,
        /// The tab last grabbed for a drag, and where along it.
        pub(super) grab: Cell<Option<(TabId, (f64, f64))>>,
        /// Tabs given while a drag lasts, shown once it is over.
        pub(super) waiting_tabs: RefCell<Option<Vec<TabView>>>,
        /// A stand-in for a tab dragged here from another window.
        pub(super) guest: RefCell<Option<gtk::Widget>>,
        /// Where the pointer holding that stand-in is along the strip, and
        /// the share of its width it holds, which it follows again as the
        /// strip scrolls.
        pub(super) guest_pointer: Cell<Option<(f64, f64)>>,
        /// The tab following the pointer and where the pointer is, along
        /// the strip, which it follows again as the strip scrolls.
        pub(super) pointer: Cell<Option<(TabId, f64)>>,
        /// How far each tick of a drag dwelling near an edge scrolls the
        /// strip.
        pub(super) edge_step: Cell<f64>,
        /// The timer of that scroll.
        pub(super) edge_scroll: RefCell<Option<glib::SourceId>>,
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
            crate::i18n::translate_template(&*self.obj(), "tab-strip.ui");
            gestures::scroll_sideways_with_wheel(&self.scroller);
            self.obj().follow_the_pointer_as_it_scrolls();
            self.obj().add_controller(super::copy_on_double_click());
            let menu = MenuPopover::new(Vec::new());
            menu.set_offset(0, 0);
            menu.set_parent(&*self.obj());
            self.menu.set(menu).expect("constructed runs once per object");
            let windows_menu = MenuPopover::new(Vec::new());
            windows_menu.set_position(gtk::PositionType::Bottom);
            windows_menu.set_parent(&*self.obj());
            self.windows_menu
                .set(windows_menu)
                .expect("constructed runs once per object");
            let edges = super::edges::Edges::new(&self.obj());
            self.edges.set(edges).expect("constructed runs once per object");
        }

        fn dispose(&self) {
            for menu in [self.menu.get(), self.windows_menu.get()].into_iter().flatten() {
                menu.unparent();
            }
            if let Some(edges) = self.edges.get() {
                edges.unparent();
            }
            self.dispose_template();
        }
    }

    impl WidgetImpl for TabStrip {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            // Not the parent's snapshot, which would also draw the edge
            // widgets at the strip's top left: the tabs, then the edges.
            self.obj().snapshot_child(&*self.scroller, snapshot);
            self.obj().draw_edges(snapshot);
        }
    }
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

    /// Keeps the strip within 70% of a window `window_width` pixels wide
    /// (`.tabs{max-width:70%}`), so the drag area beside it stays wide
    /// enough to grab; tabs that do not fit shrink and then scroll. The
    /// cap changes after the allocation that reports the width, because
    /// changing it during one would start it again.
    pub(super) fn cap_width(&self, window_width: i32) {
        let cap = window_width * MAX_STRIP_PERCENT / 100;
        let scroller = self.imp().scroller.get();
        if scroller.max_content_width() == cap {
            return;
        }
        glib::idle_add_local_once(glib::clone!(
            #[weak]
            scroller,
            move || scroller.set_max_content_width(cap)
        ));
    }

    /// Replaces the tabs with `tabs` and scrolls the active one into view.
    pub(super) fn set_tabs(&self, tabs: &[TabView]) {
        let imp = self.imp();
        // A drag keeps the tab widgets it moves until it is over; which tab
        // is in front shows at once.
        if imp.layout.is_live() {
            let had_focus = imp.tab_list.focus_child().is_some();
            let is_front_only = self.show_front(tabs);
            imp.waiting_tabs.replace((!is_front_only).then(|| tabs.to_vec()));
            // A stand-in for a tab dragged in stays the one in front.
            if imp.guest.borrow().is_some() {
                self.show_own_front(false);
            }
            // The focus goes with the front, as when the strip is drawn anew.
            if had_focus {
                if let Some(front) = self.front_widget() {
                    front.grab_focus();
                }
            }
            return;
        }
        imp.waiting_tabs.replace(None);
        // A tab being shown by the keyboard keeps focus in the strip.
        let had_focus = imp.tab_list.focus_child().is_some();
        // A tab that only came to the front or moved keeps its widget, so a
        // press on it, which shows it (as Chrome does), is not ended by
        // drawing the strip anew.
        let mut old: Vec<(TabView, gtk::Box)> = imp.shown.take();
        let mut active = None;
        let mut shown = Vec::with_capacity(tabs.len());
        let mut previous: Option<gtk::Widget> = None;
        for tab in tabs {
            let kept = old
                .iter()
                .position(|(view, _)| is_same_but_for_front(view, tab))
                .map(|at| old.remove(at).1);
            let widget = match kept {
                Some(widget) => {
                    show_in_front(&widget, tab.active);
                    widget
                }
                None => tab_widget(tab),
            };
            if widget.parent().is_some() {
                imp.tab_list.reorder_child_after(&widget, previous.as_ref());
            } else {
                imp.tab_list.insert_child_after(&widget, previous.as_ref());
            }
            previous = Some(widget.clone().upcast());
            if tab.active {
                active = Some(widget.clone());
            }
            shown.push((tab.clone(), widget));
        }
        for (_, gone) in old {
            imp.tab_list.remove(&gone);
        }
        imp.shown.replace(shown);
        let Some(active) = active else {
            return;
        };
        if had_focus {
            active.grab_focus();
        }
        // After the new tabs are laid out, so their positions are known.
        let viewport = imp.viewport.get();
        glib::idle_add_local_once(glib::clone!(
            #[weak]
            viewport,
            move || viewport.scroll_to(&active, None)
        ));
    }

    /// The widget of the tab in front.
    fn front_widget(&self) -> Option<gtk::Box> {
        let shown = self.imp().shown.borrow();
        shown
            .iter()
            .find(|(view, _)| view.active)
            .map(|(_, widget)| widget.clone())
    }

    /// Shows which of the tabs shown is in front among `tabs`, by id:
    /// none when the tab in front is one the strip does not show yet, as
    /// one opened during a drag. True when nothing but the front changed;
    /// otherwise the rest waits for the drag to end.
    fn show_front(&self, tabs: &[TabView]) -> bool {
        let mut shown = self.imp().shown.borrow_mut();
        let is_front_only = shown.len() == tabs.len()
            && shown
                .iter()
                .zip(tabs)
                .all(|((view, _), tab)| is_same_but_for_front(view, tab));
        for (view, widget) in shown.iter_mut() {
            let given = tabs.iter().find(|tab| tab.id == view.id);
            let active = given.is_some_and(|tab| tab.active);
            show_in_front(widget, active);
            match given.filter(|_| is_front_only) {
                Some(tab) => *view = tab.clone(),
                None => view.active = active,
            }
        }
        is_front_only
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

    /// Opens the list of open windows `entries` below the tab in front,
    /// or at the start of the strip when no tab is shown.
    pub(super) fn show_windows_menu(&self, entries: Vec<MenuEntry>) {
        let Some(menu) = self.imp().windows_menu.get() else {
            return;
        };
        menu.set_entries(entries);
        let front = self.front_widget().and_then(|front| front.compute_bounds(self));
        #[expect(clippy::cast_possible_truncation, reason = "tab bounds are small")]
        let target = front.map_or_else(
            || gdk::Rectangle::new(0, 0, 1, self.height()),
            |front| {
                gdk::Rectangle::new(
                    front.x() as i32,
                    front.y() as i32,
                    front.width() as i32,
                    front.height() as i32,
                )
            },
        );
        menu.set_pointing_to(Some(&target));
        menu.popup();
    }

    /// The tab `step` leads to from the tab `widget`; `None` past either
    /// end.
    fn tab_beside(&self, widget: &gtk::Widget, step: TabStep) -> Option<TabId> {
        let shown = self.imp().shown.borrow();
        let index = shown
            .iter()
            .position(|(_, tab)| tab.upcast_ref::<gtk::Widget>() == widget)?;
        let target = match step {
            TabStep::Previous => index.checked_sub(1)?,
            TabStep::Next => index + 1,
            TabStep::First => 0,
            TabStep::Last => shown.len().checked_sub(1)?,
        };
        shown.get(target).map(|(tab, _)| tab.id)
    }

    /// The tab list, for tests.
    #[cfg(test)]
    pub(in crate::window) fn tab_list(&self) -> gtk::Box {
        self.imp().tab_list.get()
    }

    /// The list of open windows, for tests.
    #[cfg(test)]
    pub(in crate::window) fn windows_menu(&self) -> super::menu_popover::MenuPopover {
        self.imp()
            .windows_menu
            .get()
            .expect("constructed builds the menu")
            .clone()
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

/// Whether `old` shows `new` but for which tab is in front: its widget can
/// stay.
fn is_same_but_for_front(old: &TabView, new: &TabView) -> bool {
    TabView {
        active: new.active,
        ..old.clone()
    } == *new
}

/// Marks the tab `widget` as in front, or not: its look, its keyboard
/// focus (a roving tab stop) and its selected state.
fn show_in_front(widget: &gtk::Box, active: bool) {
    super::widget_tree::toggle_class(widget, "active", active);
    widget.set_focusable(active);
    widget.update_state(&[gtk::accessible::State::Selected(Some(active))]);
}

fn tab_icon(icon: Art) -> ArtImage {
    let image = ArtImage::new(icon, ICON_SIZE);
    image.add_css_class(TAB_ICON_CLASS);
    image
}

/// The widget of `tab`: its icon, title and close button, one target that
/// shows the tab on a click or Enter and closes it on a middle-click. Only
/// the tab in front takes keyboard focus.
fn tab_widget(tab: &TabView) -> gtk::Box {
    let widget = gtk::Box::builder()
        .spacing(ICON_TO_TITLE)
        .focusable(tab.active)
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
            let is_only_tab = strip.imp().shown.borrow().len() <= 1;
            strip.show_menu(tab_menu(id, &uri, is_only_tab), point);
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
        .tooltip_text(ox_core::i18n::format_message(
            "Previous version · {snapshot}",
            &[("snapshot", snapshot)],
        ))
        .css_classes(["snapshot-tab-badge"])
        .build();
    badge.append(&icons::image(Icon::Clock, SNAPSHOT_BADGE_GLYPH));
    badge.append(&gtk::Label::new(Some(&ox_core::i18n::gettext(
        "Previous version",
    ))));
    widget.append(&badge);
    let name = ox_core::i18n::format_message(
        "{title} — Previous version — {snapshot}",
        &[("title", title), ("snapshot", snapshot)],
    );
    widget.update_property(&[gtk::accessible::Property::Label(&name)]);
}

/// Runs the tab action `action` on the tab `id` from `widget`, when the
/// gesture still has one.
fn run_on(widget: Option<gtk::Widget>, action: WindowAction, id: &glib::Variant) {
    if let Some(widget) = widget {
        action.activate_from(&widget, Some(id));
    }
}

/// A primary press anywhere on the tab shows it at once, as Chrome and
/// Windows Explorer do, so a tab that is dragged is the one in front from
/// the start (TAB-004). Showing it keeps the tab widgets (`set_tabs`), so
/// the press goes on into a drag.
fn select_on_click(id: glib::Variant) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.connect_pressed(move |gesture, _, _, _| {
        run_on(gesture.widget(), WindowAction::SelectTab, &id);
    });
    click
}

/// A double-click on a tab opens a copy of it in front, as in Dolphin
/// (TAB-014). The strip counts the clicks itself: the first press shows
/// the tab, and when that draws a tab anew under the pointer, GTK counts
/// the second press as a first one. Claiming the second press keeps
/// it from the title bar, whose double-click maximizes the window, and
/// from the tab's own click. The close button claims its own presses, so
/// a double-click on it never reaches the strip.
fn copy_on_double_click() -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let last_press = Cell::new(None);
    click.connect_pressed(move |gesture, presses, x, y| {
        let press = Press {
            time: gesture.current_event_time(),
            x,
            y,
        };
        let previous = last_press.replace(Some(press));
        let is_second = presses == 2 || previous.is_some_and(|first| press.follows(first));
        if !is_second {
            return;
        }
        last_press.set(None);
        let Some(strip) = gesture.widget().and_downcast::<TabStrip>() else {
            return;
        };
        let Some(tab) = strip.tab_at(x, y) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        WindowAction::OpenTab.activate_from(&strip, Some(&tab.uri.to_variant()));
    });
    click
}

/// A primary press on the strip: when, in milliseconds, and where.
#[derive(Debug, Clone, Copy)]
struct Press {
    time: u32,
    x: f64,
    y: f64,
}

impl Press {
    /// Whether this press and `first` make a double-click, by the
    /// desktop's double-click time and distance.
    fn follows(self, first: Press) -> bool {
        let settings = gtk::Settings::default();
        let time = settings
            .as_ref()
            .map_or(400, gtk::Settings::gtk_double_click_time);
        let distance = settings
            .as_ref()
            .map_or(5, gtk::Settings::gtk_double_click_distance);
        let soon = i64::from(self.time.wrapping_sub(first.time)) <= i64::from(time);
        let near = (self.x - first.x).abs() <= f64::from(distance)
            && (self.y - first.y).abs() <= f64::from(distance);
        soon && near
    }
}

/// Where an arrow key, Home or End moves from a focused tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabStep {
    Previous,
    Next,
    First,
    Last,
}

impl TabStep {
    /// The step `key` asks for, if any.
    fn for_key(key: gdk::Key) -> Option<Self> {
        match key {
            gdk::Key::Left | gdk::Key::KP_Left => Some(TabStep::Previous),
            gdk::Key::Right | gdk::Key::KP_Right => Some(TabStep::Next),
            gdk::Key::Home | gdk::Key::KP_Home => Some(TabStep::First),
            gdk::Key::End | gdk::Key::KP_End => Some(TabStep::Last),
            _ => None,
        }
    }
}

/// Shows the tab `step` leads to from the focused tab `widget`, if any;
/// the strip then gives that tab focus.
fn step_to_tab(widget: &gtk::Widget, step: TabStep) {
    let strip = widget
        .ancestor(TabStrip::static_type())
        .and_downcast::<TabStrip>();
    if let Some(id) = strip.and_then(|strip| strip.tab_beside(widget, step)) {
        WindowAction::SelectTab.activate_from(widget, Some(&id.to_variant()));
    }
}

/// Enter or Space on a focused tab shows it; the arrow keys, Home and End
/// show another tab.
fn select_on_enter(id: glib::Variant) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |keys, key, _, _| {
        let Some(widget) = keys.widget() else {
            return glib::Propagation::Proceed;
        };
        if let Some(step) = TabStep::for_key(key) {
            step_to_tab(&widget, step);
            return glib::Propagation::Stop;
        }
        let activates = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space);
        if !activates {
            return glib::Propagation::Proceed;
        }
        run_on(Some(widget), WindowAction::SelectTab, &id);
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
        .tooltip_text(ox_core::i18n::gettext("Close tab"))
        .action_name(WindowAction::CloseTabById.detailed_name())
        .action_target(&tab.id.to_variant())
        .focus_on_click(false)
        .valign(gtk::Align::Center)
        .css_classes(["tab-close"])
        .build();
    let name = ox_core::i18n::format_message("Close {title}", &[("title", &tab.title)]);
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
