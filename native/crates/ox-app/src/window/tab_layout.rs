// SPDX-License-Identifier: AGPL-3.0-only
//! How the tab strip shares its width between tabs.
//!
//! Ports the flex rules of `.tabs` and `.tab` in `v2.0.0:desktop/ui/style.css`:
//! every tab is 215 pixels wide (180 or 150 in a narrow window), tabs
//! shrink evenly toward 100 pixels when the title bar runs out of room, and
//! below that the strip scrolls. A `GtkBox` cannot shrink its children
//! below their natural width without also dropping to their minimum, so
//! the tab strip uses this layout.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod live;

use live::{frame_time, slide_length, LiveTabs};

use super::widget_tree::{laid_out_children, widest_minimum_width};

/// A tab's width when there is room (`.tab{width:215px}`).
pub(super) const TAB_WIDTH: i32 = 215;
/// The narrowest a tab gets before the strip scrolls: the Windows 11 tab
/// minimum (ui-spec.md §4.1), where the web's `min-width:80px` left too
/// little of a title to read.
pub(super) const MIN_TAB_WIDTH: i32 = 100;
/// The space between tabs (`.tabs{gap:2px}`).
pub(super) const TAB_GAP: i32 = 2;
/// What a tab showing a previous version gets beyond the others, for its
/// "Previous version" badge (`.tab.snapshot-tab{width:330px}`).
const SNAPSHOT_TAB_EXTRA: i32 = 330 - TAB_WIDTH;
/// The narrowest a tab showing a previous version gets, so its badge
/// stays legible (`.tab.snapshot-tab{min-width:285px;flex-shrink:0}`).
const MIN_SNAPSHOT_TAB_WIDTH: i32 = 285;

/// The width `tab` gets while the other tabs get `shared`: a snapshot
/// tab's badge needs more room, and never less than 285 pixels.
fn width_of(tab: &gtk::Widget, shared: i32) -> i32 {
    if tab.has_css_class("snapshot-tab") {
        (shared + SNAPSHOT_TAB_EXTRA).max(MIN_SNAPSHOT_TAB_WIDTH)
    } else {
        shared
    }
}

/// The widths a tab may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TabWidths {
    /// The width when there is room.
    pub widest: i32,
    /// The narrowest a tab's contents let it get; a large text size can
    /// make this more than [`MIN_TAB_WIDTH`].
    pub narrowest: i32,
}

impl TabWidths {
    /// The narrowest a tab gets: [`MIN_TAB_WIDTH`], or more when its
    /// contents need it.
    fn narrowest_allowed(self) -> i32 {
        self.narrowest.max(MIN_TAB_WIDTH)
    }

    /// The widest a tab gets: its width when there is room, never less
    /// than [`Self::narrowest_allowed`].
    fn widest_allowed(self) -> i32 {
        self.widest.max(self.narrowest_allowed())
    }
}

/// The width each of `count` tabs gets in `available` pixels.
pub(super) fn tab_width(available: i32, count: i32, widths: TabWidths) -> i32 {
    if count == 0 {
        return widths.widest_allowed();
    }
    let gaps = TAB_GAP * (count - 1);
    let even_share = (available - gaps) / count;
    even_share.clamp(widths.narrowest_allowed(), widths.widest_allowed())
}

/// The strip's width when each of `tabs` gets `shared` pixels, or what
/// [`width_of`] gives a snapshot tab.
fn strip_width(tabs: &[gtk::Widget], shared: i32) -> i32 {
    let widths: i32 = tabs.iter().map(|tab| width_of(tab, shared)).sum();
    let count = i32::try_from(tabs.len()).unwrap_or(i32::MAX);
    widths + TAB_GAP * (count - 1).max(0)
}

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{
        laid_out_children, strip_width, tab_width, widest_minimum_width, width_of, LiveTabs, TabWidths,
        MIN_TAB_WIDTH, TAB_GAP, TAB_WIDTH,
    };

    /// Private state of [`super::TabLayout`].
    #[derive(Debug)]
    pub(crate) struct TabLayout {
        /// A tab's width when there is room.
        pub(super) widest: Cell<i32>,
        /// Where the tabs are while a tab is dragged over the strip.
        pub(super) live: RefCell<Option<LiveTabs>>,
        /// The width each tab got at the last allocation.
        pub(super) each: Cell<i32>,
    }

    impl Default for TabLayout {
        fn default() -> Self {
            Self {
                widest: Cell::new(TAB_WIDTH),
                live: RefCell::new(None),
                each: Cell::new(TAB_WIDTH),
            }
        }
    }

    impl TabLayout {
        fn widths(&self, tabs: &[gtk::Widget]) -> TabWidths {
            TabWidths {
                widest: self.widest.get(),
                narrowest: widest_minimum_width(tabs).unwrap_or(MIN_TAB_WIDTH),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TabLayout {
        const NAME: &'static str = "OxTabLayout";
        type Type = super::TabLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for TabLayout {}

    impl LayoutManagerImpl for TabLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            _for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let tabs = laid_out_children(widget);
            if orientation == gtk::Orientation::Vertical {
                let height = tabs
                    .iter()
                    .map(|tab| tab.measure(orientation, -1).0)
                    .max()
                    .unwrap_or(0);
                return (height, height, -1, -1);
            }
            let widths = self.widths(&tabs);
            let minimum = strip_width(&tabs, widths.narrowest_allowed());
            let natural = strip_width(&tabs, widths.widest_allowed());
            (minimum, natural, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let tabs = laid_out_children(widget);
            let count = i32::try_from(tabs.len()).unwrap_or(i32::MAX);
            let each = tab_width(width, count, self.widths(&tabs));
            self.each.set(each);
            if let Some(live) = self.live.borrow().as_ref() {
                let now = super::frame_time(widget);
                let width_of_tab = |tab: &gtk::Widget| f64::from(width_of(tab, each));
                for (tab, left) in live.placements(f64::from(TAB_GAP), f64::from(width), now, &width_of_tab) {
                    // On whole pixels, as Chrome places tabs: between them
                    // an icon blurs and sharpens again as its tab moves.
                    #[expect(clippy::cast_possible_truncation, reason = "tab positions are small")]
                    let at = gtk::graphene::Point::new(left.round() as f32, 0.0);
                    let transform = gtk::gsk::Transform::new().translate(&at);
                    tab.allocate(width_of(&tab, each), height, -1, Some(transform));
                }
                return;
            }
            let mut x = 0;
            for tab in tabs {
                let tab_width = width_of(&tab, each);
                let placement = gtk::Allocation::new(x, 0, tab_width, height);
                tab.size_allocate(&placement, -1);
                x += tab_width + TAB_GAP;
            }
        }
    }
}

glib::wrapper! {
    /// Lays out the tab strip's tabs side by side at an equal width.
    pub(crate) struct TabLayout(ObjectSubclass<imp::TabLayout>)
        @extends gtk::LayoutManager;
}

impl TabLayout {
    /// Makes tabs `width` pixels wide when there is room.
    pub(crate) fn set_tab_width(&self, width: i32) {
        if self.imp().widest.replace(width) != width {
            self.layout_changed();
        }
    }

    /// The width of the row the tabs are laid out in.
    fn row(&self) -> f64 {
        self.widget().map_or(0.0, |row| f64::from(row.width()))
    }

    /// The width `tab` got, padding and border included.
    pub(crate) fn width_of(&self, tab: &gtk::Widget) -> f64 {
        f64::from(width_of(tab, self.imp().each.get()))
    }

    /// The gap between tabs, the row's width, the frame time and how
    /// long a slide lasts now.
    fn frame(&self, row: &gtk::Widget) -> (f64, f64, i64, i64) {
        (f64::from(TAB_GAP), self.row(), frame_time(row), slide_length(row))
    }

    /// Starts laying `order` out live, `dragged` following the pointer from
    /// `left`. A drag of the same tab that is still sliding back resumes.
    pub(crate) fn begin_drag(&self, order: Vec<gtk::Widget>, dragged: &gtk::Widget, left: f64) {
        let resumes = self
            .imp()
            .live
            .borrow()
            .as_ref()
            .is_some_and(|live| live.dragged() == dragged);
        if resumes {
            if let Some(live) = self.imp().live.borrow_mut().as_mut() {
                live.take_on_settled();
            }
            self.follow(left);
            return;
        }
        self.end();
        self.imp()
            .live
            .replace(Some(LiveTabs::dragging(order, dragged.clone(), left)));
        // The others make room at once, also when a drop follows.
        self.follow(left);
    }

    /// The dragged tab's left edge follows the pointer to `left`.
    pub(crate) fn follow(&self, left: f64) {
        let Some(row) = self.widget() else { return };
        let frame = self.frame(&row);
        if let Some(live) = self.imp().live.borrow_mut().as_mut() {
            live.follow(left, frame, &|tab| self.width_of(tab));
        }
        self.animate();
    }

    /// The dragged tab leaves the strip: the others close up.
    pub(crate) fn detach(&self) {
        let Some(row) = self.widget() else { return };
        let frame = self.frame(&row);
        if let Some(live) = self.imp().live.borrow_mut().as_mut() {
            live.detach(frame, &|tab| self.width_of(tab));
        }
        self.animate();
    }

    /// Slides every tab to its place in `order`, then runs `then`.
    pub(crate) fn settle(&self, order: Vec<gtk::Widget>, then: impl FnOnce() + 'static) {
        let Some(row) = self.widget() else {
            then();
            return;
        };
        let frame = self.frame(&row);
        if let Some(live) = self.imp().live.borrow_mut().as_mut() {
            live.settle(order, frame, &|tab| self.width_of(tab), Box::new(then));
        } else {
            then();
            return;
        }
        self.animate();
    }

    /// Stops laying the tabs out live at once; they go back to their
    /// order as children.
    pub(crate) fn end(&self) {
        let Some(mut live) = self.imp().live.take() else {
            return;
        };
        if let Some(tick) = live.set_tick(None) {
            tick.remove();
        }
        self.layout_changed();
    }

    /// The tabs in the order shown, while they are laid out live.
    pub(crate) fn live_order(&self) -> Option<Vec<gtk::Widget>> {
        self.imp()
            .live
            .borrow()
            .as_ref()
            .map(|live| live.order().to_vec())
    }

    /// The tab following the pointer, if one is.
    pub(crate) fn following(&self) -> Option<gtk::Widget> {
        self.imp().live.borrow().as_ref()?.following().cloned()
    }

    /// The dragged tab, following or settling.
    pub(crate) fn dragged(&self) -> Option<gtk::Widget> {
        self.imp()
            .live
            .borrow()
            .as_ref()
            .map(|live| live.dragged().clone())
    }

    /// Ends the live layout at once, first running what waits for a
    /// released tab, as a new drag starts before it is in place.
    pub(crate) fn finish_now(&self) {
        let settled = self
            .imp()
            .live
            .borrow_mut()
            .as_mut()
            .and_then(LiveTabs::take_on_settled);
        self.end();
        if let Some(settled) = settled {
            settled.run();
        }
    }

    /// Whether the tabs are laid out live.
    pub(crate) fn is_live(&self) -> bool {
        self.imp().live.borrow().is_some()
    }

    /// Lays the tabs out again now and on every frame while one slides;
    /// once all are still, runs what waits for a released tab.
    fn animate(&self) {
        self.layout_changed();
        let Some(row) = self.widget() else { return };
        let now = frame_time(&row);
        let (still, has_tick) = {
            let live = self.imp().live.borrow();
            let Some(live) = live.as_ref() else { return };
            (live.is_still(now) || !row.is_mapped(), live.has_tick())
        };
        // Without animations every slide is over at once (`slide_length`).
        if still || !row.settings().is_gtk_enable_animations() {
            self.run_settled();
            return;
        }
        if has_tick {
            return;
        }
        let tick = row.add_tick_callback(glib::clone!(
            #[weak(rename_to = layout)]
            self,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move |_, clock| {
                layout.layout_changed();
                let still = layout
                    .imp()
                    .live
                    .borrow()
                    .as_ref()
                    .is_none_or(|live| live.is_still(clock.frame_time()));
                if !still {
                    return glib::ControlFlow::Continue;
                }
                if let Some(live) = layout.imp().live.borrow_mut().as_mut() {
                    live.set_tick(None);
                }
                layout.run_settled();
                glib::ControlFlow::Break
            }
        ));
        if let Some(live) = self.imp().live.borrow_mut().as_mut() {
            live.set_tick(Some(tick));
        }
    }

    /// Runs what waits for a released tab, once, after this frame.
    fn run_settled(&self) {
        let settled = self
            .imp()
            .live
            .borrow_mut()
            .as_mut()
            .and_then(LiveTabs::take_on_settled);
        if let Some(settled) = settled {
            glib::idle_add_local_once(move || settled.run());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(widest: i32, narrowest: i32) -> TabWidths {
        TabWidths { widest, narrowest }
    }

    #[test]
    fn tabs_are_full_width_while_there_is_room() {
        assert_eq!(tab_width(1000, 3, widths(TAB_WIDTH, MIN_TAB_WIDTH)), TAB_WIDTH);
        assert_eq!(
            tab_width(1000, 3, widths(180, MIN_TAB_WIDTH)),
            180,
            "a narrow window"
        );
    }

    #[test]
    fn tabs_shrink_evenly_then_stop_at_their_minimum() {
        assert_eq!(tab_width(302, 2, widths(TAB_WIDTH, MIN_TAB_WIDTH)), 150);
        assert_eq!(
            tab_width(100, 4, widths(TAB_WIDTH, MIN_TAB_WIDTH)),
            100,
            "WinUI's minimum"
        );
    }

    /// parity: TAB-019
    #[gtk::test]
    fn a_snapshot_tab_is_wider_and_never_narrower_than_285_pixels() {
        let plain = gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast::<gtk::Widget>();
        let snapshot = gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast::<gtk::Widget>();
        snapshot.add_css_class("snapshot-tab");
        assert_eq!(width_of(&plain, TAB_WIDTH), TAB_WIDTH);
        assert_eq!(width_of(&snapshot, TAB_WIDTH), 330);
        assert_eq!(width_of(&plain, MIN_TAB_WIDTH), MIN_TAB_WIDTH);
        assert_eq!(width_of(&snapshot, MIN_TAB_WIDTH), 285);
        assert_eq!(strip_width(&[plain, snapshot], MIN_TAB_WIDTH), 100 + 2 + 285);
    }

    /// A tab that follows the pointer, which on a touchpad stops between
    /// pixels, is drawn on whole pixels, as Chrome keeps tabs on whole
    /// pixels, so its icon and title stay as sharp as when it is still.
    ///
    /// parity: TAB-032
    #[gtk::test]
    fn a_moving_tab_is_drawn_on_whole_pixels() {
        let layout = glib::Object::new::<TabLayout>();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.set_layout_manager(Some(layout.clone()));
        let tabs: Vec<gtk::Widget> = (0..2)
            .map(|_| gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast())
            .collect();
        for tab in &tabs {
            row.append(tab);
        }
        let lay_out = || {
            row.measure(gtk::Orientation::Horizontal, -1);
            row.measure(gtk::Orientation::Vertical, -1);
            row.allocate(600, 30, -1, None);
        };
        lay_out();

        layout.begin_drag(tabs.clone(), &tabs[0], 40.4);
        lay_out();
        let at = tabs[0].compute_bounds(&row).expect("a laid out tab").x();
        layout.follow(41.6);
        lay_out();
        let then = tabs[0].compute_bounds(&row).expect("a laid out tab").x();

        assert!((at - 40.0).abs() < 1e-6, "on the nearest pixel: {at}");
        assert!((then - 42.0).abs() < 1e-6, "on the nearest pixel: {then}");
    }

    #[test]
    fn tabs_never_get_narrower_than_their_contents_need() {
        assert_eq!(tab_width(100, 4, widths(TAB_WIDTH, 112)), 112);
        assert_eq!(tab_width(2000, 1, widths(TAB_WIDTH, 240)), 240);
    }
}
