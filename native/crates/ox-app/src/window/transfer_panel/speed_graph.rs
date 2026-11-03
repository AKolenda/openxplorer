// SPDX-License-Identifier: AGPL-3.0-only
//! The speed graph of "More details" (OPS-021): how fast the action went
//! as it progressed, the part done shaded, a line at the current speed
//! and "Speed: 32.8 MB/s" above it, as in the Windows progress window,
//! whose detailed view has "a real-time throughput graph" (Building
//! Windows 8, "Improving our file management basics", 2011).
//!
//! The graph is made of widgets, not drawn: a row of one hundred
//! columns, one per percent of the action, each holding a bar as high as
//! the speed was there, styled in `dialogs.css`. The line and its label
//! lie over them. Nothing moves frame by frame: the graph changes only
//! when the action reports, so reduced motion has nothing to stop
//! (ACC-010).
//!
//! For assistive technologies the graph is the action's progress bar
//! while the details show (the plain bar hides then), named "Operation
//! progress", with the speed as its description.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Columns of the graph: one per percent of the action.
const COLUMNS: usize = 100;
/// The graph's height, in logical pixels.
const HEIGHT: i32 = 72;
/// Room above the highest speed, so the line and its label stay inside.
const HEADROOM: f64 = 1.3;

mod imp {
    use super::*;

    /// Private state of [`super::SpeedGraph`].
    #[derive(Debug, Default)]
    pub(crate) struct SpeedGraph {
        /// The columns, one per percent.
        pub(super) columns: RefCell<Vec<gtk::Box>>,
        /// The bar in each column.
        pub(super) bars: RefCell<Vec<gtk::Box>>,
        /// The speed at each percent, once reached.
        pub(super) speeds: RefCell<Vec<Option<u64>>>,
        /// The columns reached so far.
        pub(super) reached: Cell<usize>,
        /// The line at the current speed.
        pub(super) line: RefCell<Option<gtk::Box>>,
        /// "Speed: 32.8 MB/s".
        pub(super) label: RefCell<Option<gtk::Label>>,
        /// Where the line is, in pixels from the bottom.
        pub(super) line_target: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpeedGraph {
        const NAME: &'static str = "OxSpeedGraph";
        type Type = super::SpeedGraph;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("speedgraph");
            klass.set_accessible_role(gtk::AccessibleRole::ProgressBar);
        }
    }

    impl ObjectImpl for SpeedGraph {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }
    impl WidgetImpl for SpeedGraph {}
    impl BoxImpl for SpeedGraph {}
}

glib::wrapper! {
    /// The speed graph of an action's details.
    pub(crate) struct SpeedGraph(ObjectSubclass<imp::SpeedGraph>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl SpeedGraph {
    fn build(&self) {
        let imp = self.imp();
        self.set_height_request(HEIGHT);
        self.set_overflow(gtk::Overflow::Hidden);
        let overlay = gtk::Overlay::new();
        overlay.set_hexpand(true);
        let row = gtk::Box::builder().homogeneous(true).hexpand(true).build();
        let mut columns = Vec::with_capacity(COLUMNS);
        let mut bars = Vec::with_capacity(COLUMNS);
        for _ in 0..COLUMNS {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.add_css_class("speed-column");
            let bar = gtk::Box::builder().valign(gtk::Align::End).vexpand(true).build();
            bar.add_css_class("speed-bar");
            bar.set_height_request(0);
            column.append(&bar);
            row.append(&column);
            columns.push(column);
            bars.push(bar);
        }
        overlay.set_child(Some(&row));
        let line = gtk::Box::builder()
            .valign(gtk::Align::End)
            .hexpand(true)
            .height_request(1)
            .visible(false)
            .build();
        line.add_css_class("speed-line");
        overlay.add_overlay(&line);
        let label = gtk::Label::builder()
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .visible(false)
            .build();
        label.add_css_class("speed-label");
        overlay.add_overlay(&label);
        self.append(&overlay);
        imp.columns.replace(columns);
        imp.bars.replace(bars);
        imp.speeds.replace(vec![None; COLUMNS]);
        imp.line.replace(Some(line));
        imp.label.replace(Some(label));
        self.update_property(&[
            gtk::accessible::Property::Label(&ox_core::i18n::gettext("Operation progress")),
            gtk::accessible::Property::ValueMin(0.0),
            gtk::accessible::Property::ValueMax(100.0),
            gtk::accessible::Property::ValueNow(0.0),
        ]);
    }

    /// Empties the graph for a new action.
    pub(crate) fn reset(&self) {
        let imp = self.imp();
        imp.speeds.replace(vec![None; COLUMNS]);
        imp.reached.set(0);
        for column in imp.columns.borrow().iter() {
            column.remove_css_class("reached");
        }
        for bar in imp.bars.borrow().iter() {
            bar.set_height_request(0);
        }
        imp.line_target.set(0);
        if let Some(line) = imp.line.borrow().as_ref() {
            line.set_visible(false);
            line.set_margin_bottom(0);
        }
        if let Some(label) = imp.label.borrow().as_ref() {
            label.set_visible(false);
        }
        self.update_property(&[gtk::accessible::Property::ValueNow(0.0)]);
    }

    /// Shows the action at `fraction` (0–1), going at `speed` bytes per
    /// second when known, which `speed_text` says.
    pub(crate) fn show(&self, fraction: f64, speed: Option<(u64, String)>) {
        let imp = self.imp();
        let percent = (fraction.clamp(0.0, 1.0) * 100.0).round();
        let reached = percent_index(percent);
        let before = imp.reached.get();
        if reached > before {
            let columns = imp.columns.borrow();
            for column in &columns[before..reached] {
                column.add_css_class("reached");
            }
            imp.reached.set(reached);
        }
        self.update_property(&[gtk::accessible::Property::ValueNow(percent)]);
        let Some((speed, text)) = speed else {
            return;
        };
        {
            let mut speeds = imp.speeds.borrow_mut();
            let first = before.min(reached.saturating_sub(1));
            for slot in speeds.iter_mut().take(reached.max(1)).skip(first) {
                *slot = Some(speed);
            }
        }
        self.scale_bars();
        let target = self.height_of(speed);
        if let Some(label) = imp.label.borrow().as_ref() {
            label.set_text(&text);
            label.set_visible(true);
            label.set_margin_bottom((target + 2).min(HEIGHT - 18).max(0));
        }
        self.update_property(&[gtk::accessible::Property::Description(&text)]);
        self.move_line(target);
    }

    /// Sets each bar's height for the highest speed so far.
    fn scale_bars(&self) {
        let imp = self.imp();
        let speeds = imp.speeds.borrow();
        for (bar, speed) in imp.bars.borrow().iter().zip(speeds.iter()) {
            bar.set_height_request(speed.map_or(0, |speed| self.height_of(speed)));
        }
    }

    /// The height of `speed` in the graph, the highest speed so far
    /// leaving some room above it.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "a pixel height needs far less precision than f64 keeps"
    )]
    fn height_of(&self, speed: u64) -> i32 {
        let highest = self
            .imp()
            .speeds
            .borrow()
            .iter()
            .flatten()
            .copied()
            .max()
            .unwrap_or(0);
        if highest == 0 {
            return 0;
        }
        let share = speed as f64 / (highest as f64 * HEADROOM);
        (share * f64::from(HEIGHT)).round() as i32
    }

    /// Moves the speed line to `target` pixels from the bottom.
    fn move_line(&self, target: i32) {
        let imp = self.imp();
        imp.line_target.set(target);
        if let Some(line) = imp.line.borrow().as_ref() {
            line.set_visible(true);
            line.set_margin_bottom(target);
        }
    }

    /// Where the speed line is now and where it goes, in pixels from the
    /// bottom, for tests.
    #[cfg(test)]
    pub(crate) fn line_offsets(&self) -> (i32, i32) {
        let imp = self.imp();
        let shown = imp.line.borrow().as_ref().map_or(0, |line| line.margin_bottom());
        (shown, imp.line_target.get())
    }

    /// How many columns are shaded as done, for tests.
    #[cfg(test)]
    pub(crate) fn reached_columns(&self) -> usize {
        self.imp().reached.get()
    }

    /// The speed label's text, for tests.
    #[cfg(test)]
    pub(crate) fn speed_text(&self) -> Option<String> {
        let label = self.imp().label.borrow().clone()?;
        label.is_visible().then(|| label.text().to_string())
    }
}

/// The number of columns `percent` (0–100) reaches.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a rounded percentage between 0 and 100"
)]
fn percent_index(percent: f64) -> usize {
    (percent.clamp(0.0, 100.0) as usize).min(COLUMNS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The graph shades the part done, draws a bar per percent at the
    /// speed it went, and says the current speed.
    ///
    /// parity: OPS-021
    #[gtk::test]
    fn the_graph_shades_the_part_done_and_says_the_speed() {
        let graph: SpeedGraph = glib::Object::new();

        graph.show(0.2, Some((10, "Speed: 10 bytes/s".to_owned())));
        graph.show(0.44, Some((20, "Speed: 20 bytes/s".to_owned())));

        assert_eq!(graph.reached_columns(), 44);
        assert_eq!(graph.speed_text().as_deref(), Some("Speed: 20 bytes/s"));
        let bars = graph.imp().bars.borrow();
        assert!(bars[10].height_request() > 0 && bars[10].height_request() < bars[30].height_request());
        assert_eq!(bars[60].height_request(), 0, "nothing beyond the progress");
        assert_eq!(graph.accessible_role(), gtk::AccessibleRole::ProgressBar);
        drop(bars);

        graph.reset();
        assert_eq!((graph.reached_columns(), graph.speed_text()), (0, None));
    }
}
