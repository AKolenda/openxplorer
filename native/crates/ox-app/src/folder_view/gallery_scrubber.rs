// SPDX-License-Identifier: AGPL-3.0-only
//! Gallery's date scrubber at the right edge (HOME-028), as Windows 11
//! File Explorer's Gallery has it in place of a scroll bar: an up arrow
//! at the top, a down arrow at the bottom, and between them a track that
//! marks where each year starts ("2025"). Hovering the track names the
//! month there in a tooltip ("November 2026"); a click or a drag on it
//! scrolls Gallery there; the arrows step a row up or down.
//!
//! The track maps its height to Gallery's whole scroll, top to bottom,
//! and reads the months from the timeline the view hands it: each row's
//! first photo's month at the fraction of the scroll that brings that row
//! to the top. A thin bar marks where the view is now.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::icons::{self, Icon};

/// The least room between two year marks, in pixels: a year that would
/// sit closer to the one above is left out.
const YEAR_SPACING: f64 = 18.0;

/// Runs with the fraction of the scroll a click or a drag points at.
type Jump = Box<dyn Fn(f64)>;
/// Runs with `true` for the up arrow, `false` for the down one.
type Step = Box<dyn Fn(bool)>;
/// Runs with the month under the pointer and its height, or `None`.
type Hover = Box<dyn Fn(Option<(String, f64)>)>;

/// The mark of where the view is: a short bar at the track's left edge,
/// clear of the years.
const THUMB_WIDTH: i32 = 8;
/// The room the years leave at their left for the mark.
const YEAR_INSET: i32 = 12;

/// A point of the timeline: a fraction of the scroll and the month there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mark {
    /// Thousandths of the whole scroll.
    at: u32,
    month: Option<String>,
    year: Option<i32>,
}

mod imp {
    use super::{Cell, Mark, RefCell};
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::GalleryScrubber`].
    #[derive(Default)]
    pub(crate) struct GalleryScrubber {
        pub(super) up: std::cell::OnceCell<gtk::Button>,
        pub(super) down: std::cell::OnceCell<gtk::Button>,
        pub(super) track: std::cell::OnceCell<gtk::Widget>,
        pub(super) thumb: std::cell::OnceCell<gtk::Widget>,
        pub(super) years: RefCell<Vec<gtk::Label>>,
        pub(super) marks: RefCell<Vec<Mark>>,
        /// Where the view is, as a fraction of its scroll.
        pub(super) position: Cell<f64>,
        pub(super) on_jump: RefCell<Option<super::Jump>>,
        pub(super) on_step: RefCell<Option<super::Step>>,
        /// Runs with the month under the pointer and where it is down the
        /// scrubber, or `None` once the pointer leaves.
        pub(super) on_hover: RefCell<Option<super::Hover>>,
        /// Where a drag on the track started, in the track's coordinates.
        pub(super) drag_start: Cell<f64>,
    }

    impl std::fmt::Debug for GalleryScrubber {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("GalleryScrubber")
                .field("marks", &self.marks.borrow().len())
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GalleryScrubber {
        const NAME: &'static str = "OxGalleryScrubber";
        type Type = super::GalleryScrubber;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("scrubber");
            klass.set_accessible_role(gtk::AccessibleRole::Scrollbar);
        }
    }

    impl ObjectImpl for GalleryScrubber {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for GalleryScrubber {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let children = [
                self.up.get().map(|up| up.clone().upcast::<gtk::Widget>()),
                self.down.get().map(|down| down.clone().upcast::<gtk::Widget>()),
            ];
            let mut minimum = 0;
            let mut natural = 0;
            for child in children.into_iter().flatten() {
                let (child_minimum, child_natural, _, _) = child.measure(orientation, -1);
                if orientation == gtk::Orientation::Horizontal {
                    minimum = minimum.max(child_minimum);
                    natural = natural.max(child_natural);
                } else {
                    minimum += child_minimum;
                    natural += child_natural;
                }
            }
            for label in self.years.borrow().iter() {
                let (child_minimum, child_natural, _, _) = label.measure(orientation, -1);
                if orientation == gtk::Orientation::Horizontal {
                    minimum = minimum.max(child_minimum);
                    natural = natural.max(child_natural);
                }
            }
            if orientation == gtk::Orientation::Horizontal {
                // Room for a year: four digits at the app's text size.
                let year =
                    self.obj().create_pango_layout(Some("2026")).pixel_size().0 + super::YEAR_INSET + 6;
                minimum = minimum.max(year);
                natural = natural.max(year);
            }
            (minimum, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            self.obj().place_children(width, height);
        }
    }
}

glib::wrapper! {
    /// Gallery's date scrubber.
    pub(crate) struct GalleryScrubber(ObjectSubclass<imp::GalleryScrubber>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for GalleryScrubber {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryScrubber {
    pub(crate) fn new() -> Self {
        glib::Object::new()
    }

    fn build(&self) {
        let imp = self.imp();
        let arrow = |glyph: Icon, name: &str, up: bool| {
            let button = gtk::Button::builder()
                .child(&icons::image(glyph, 12))
                .focusable(false)
                .css_classes(["flat", "scrubber-arrow"])
                .tooltip_text(name)
                .build();
            button.update_property(&[gtk::accessible::Property::Label(name)]);
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = scrubber)]
                self,
                move |_| scrubber.step(up)
            ));
            button.set_parent(self);
            button
        };
        let up = arrow(Icon::ChevronUp16, &ox_core::i18n::gettext("Scroll up"), true);
        let down = arrow(Icon::ChevronDown16, &ox_core::i18n::gettext("Scroll down"), false);
        let track = gtk::Box::builder().css_classes(["scrubber-track"]).build();
        track.set_parent(self);
        let thumb = gtk::Box::builder()
            .css_classes(["scrubber-thumb"])
            .can_target(false)
            .build();
        thumb.set_parent(self);
        imp.up.set(up).expect("built once");
        imp.down.set(down).expect("built once");
        imp.track.set(track.clone().upcast()).expect("built once");
        imp.thumb.set(thumb.upcast()).expect("built once");
        // The month under the pointer shows beside the track, as Windows'
        // Gallery shows it in a tooltip there.
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            #[weak]
            track,
            move |_, _, y| scrubber.hover_track(Some(y / f64::from(track.height().max(1))))
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            move |_| scrubber.hover_track(None)
        ));
        track.add_controller(motion);
        let press = gtk::GestureClick::new();
        press.set_button(gdk::BUTTON_PRIMARY);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            #[weak]
            track,
            move |_, _, _, y| scrubber.jump(y / f64::from(track.height().max(1)))
        ));
        track.add_controller(press);
        let drag = gtk::GestureDrag::new();
        drag.connect_drag_begin(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            move |_, _, y| scrubber.imp().drag_start.set(y)
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            #[weak]
            track,
            move |_, _, dy| {
                let y = scrubber.imp().drag_start.get() + dy;
                scrubber.jump(y / f64::from(track.height().max(1)));
            }
        ));
        track.add_controller(drag);
    }

    /// Runs `jump` with the fraction of the scroll a click or a drag
    /// points at.
    pub(crate) fn connect_jump(&self, jump: impl Fn(f64) + 'static) {
        self.imp().on_jump.replace(Some(Box::new(jump)));
    }

    /// Runs `step` with `true` for the up arrow, `false` for the down one.
    pub(crate) fn connect_step(&self, step: impl Fn(bool) + 'static) {
        self.imp().on_step.replace(Some(Box::new(step)));
    }

    /// Runs `hover` with the month under the pointer and its height in
    /// the scrubber, or `None` when the pointer leaves the track.
    pub(crate) fn connect_hover(&self, hover: impl Fn(Option<(String, f64)>) + 'static) {
        self.imp().on_hover.replace(Some(Box::new(hover)));
    }

    /// The pointer is over the track at `fraction` of its height, or left
    /// it.
    pub(crate) fn hover_track(&self, fraction: Option<f64>) {
        let shown = fraction.and_then(|fraction| {
            let month = self.month_at_fraction(fraction)?;
            let track = self.imp().track.get()?;
            let top = track.compute_point(self, &gtk::graphene::Point::zero())?.y();
            Some((
                month,
                f64::from(top) + fraction.clamp(0.0, 1.0) * f64::from(track.height()),
            ))
        });
        if let Some(hover) = self.imp().on_hover.borrow().as_ref() {
            hover(shown);
        }
    }

    fn jump(&self, fraction: f64) {
        let fraction = fraction.clamp(0.0, 1.0);
        if let Some(jump) = self.imp().on_jump.borrow().as_ref() {
            jump(fraction);
        }
    }

    fn step_by(&self, up: bool) {
        if let Some(step) = self.imp().on_step.borrow().as_ref() {
            step(up);
        }
    }

    /// Shows `timeline` (fractions of the scroll, each with the date of
    /// the photo there) with the view at `position`.
    pub(crate) fn show_timeline(&self, timeline: &[(f64, Option<u64>)], position: f64) {
        let marks: Vec<Mark> = timeline
            .iter()
            .map(|(fraction, date)| {
                let local = date
                    .and_then(|seconds| i64::try_from(seconds).ok())
                    .and_then(|seconds| glib::DateTime::from_unix_local(seconds).ok());
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a fraction"
                )]
                let at = (fraction.clamp(0.0, 1.0) * 1000.0).round() as u32;
                Mark {
                    at,
                    month: local
                        .as_ref()
                        .and_then(|date| date.format("%B %Y").ok())
                        .map(|text| text.to_string()),
                    year: local.as_ref().map(glib::DateTime::year),
                }
            })
            .collect();
        let imp = self.imp();
        let changed = *imp.marks.borrow() != marks;
        imp.position.set(position.clamp(0.0, 1.0));
        if changed {
            imp.marks.replace(marks);
            self.make_year_labels();
            self.queue_resize();
        } else {
            self.queue_allocate();
        }
        let month = self.month_at_fraction(position).unwrap_or_default();
        self.update_property(&[
            gtk::accessible::Property::ValueMin(0.0),
            gtk::accessible::Property::ValueMax(100.0),
            gtk::accessible::Property::ValueNow((position.clamp(0.0, 1.0) * 100.0).round()),
            gtk::accessible::Property::ValueText(&month),
        ]);
    }

    /// The years where the timeline turns to a new one, each with the
    /// fraction it starts at.
    fn year_starts(&self) -> Vec<(f64, i32)> {
        let mut starts: Vec<(f64, i32)> = Vec::new();
        for mark in self.imp().marks.borrow().iter() {
            let Some(year) = mark.year else {
                continue;
            };
            if starts.last().is_none_or(|(_, last)| *last != year) {
                starts.push((f64::from(mark.at) / 1000.0, year));
            }
        }
        starts
    }

    fn make_year_labels(&self) {
        let imp = self.imp();
        for label in imp.years.take() {
            label.unparent();
        }
        let labels: Vec<gtk::Label> = self
            .year_starts()
            .into_iter()
            .map(|(_, year)| {
                let label = gtk::Label::builder()
                    .label(year.to_string())
                    .can_target(false)
                    .css_classes(["scrubber-year"])
                    .xalign(1.0)
                    .build();
                // Over the position bar.
                label.set_parent(self);
                label
            })
            .collect();
        imp.years.replace(labels);
    }

    /// The month at `fraction` of the scroll: the last row's month that
    /// starts at or above it.
    fn month_at_fraction(&self, fraction: f64) -> Option<String> {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a fraction"
        )]
        let at = (fraction.clamp(0.0, 1.0) * 1000.0).round() as u32;
        let marks = self.imp().marks.borrow();
        marks
            .iter()
            .take_while(|mark| mark.at <= at)
            .last()
            .or_else(|| marks.first())
            .and_then(|mark| mark.month.clone())
    }

    fn place_children(&self, width: i32, height: i32) {
        let imp = self.imp();
        let (Some(up), Some(down), Some(track), Some(thumb)) =
            (imp.up.get(), imp.down.get(), imp.track.get(), imp.thumb.get())
        else {
            return;
        };
        let (_, arrow_height, _, _) = up.measure(gtk::Orientation::Vertical, width);
        up.allocate(width, arrow_height, -1, None);
        let bottom = (height - arrow_height).max(arrow_height);
        down.allocate(width, arrow_height, -1, Some(translate(0, bottom)));
        let track_height = (bottom - arrow_height).max(1);
        track.allocate(width, track_height, -1, Some(translate(0, arrow_height)));
        let thumb_height = 3;
        #[expect(clippy::cast_possible_truncation, reason = "pixel positions")]
        let thumb_y =
            arrow_height + (imp.position.get() * f64::from(track_height - thumb_height)).round() as i32;
        thumb.allocate(THUMB_WIDTH, thumb_height, -1, Some(translate(2, thumb_y)));
        let mut last_y = f64::NEG_INFINITY;
        for (label, (fraction, _)) in imp.years.borrow().iter().zip(self.year_starts()) {
            let (_, label_height, _, _) = label.measure(gtk::Orientation::Vertical, width);
            let y = f64::from(arrow_height) + fraction * f64::from(track_height - label_height).max(0.0);
            let shows = y - last_y >= YEAR_SPACING;
            label.set_child_visible(shows);
            if shows {
                last_y = y;
                #[expect(clippy::cast_possible_truncation, reason = "pixel positions")]
                label.allocate(
                    (width - YEAR_INSET).max(1),
                    label_height,
                    -1,
                    Some(translate(YEAR_INSET, y.round() as i32)),
                );
            }
        }
    }

    /// The years marked, for tests.
    #[cfg(test)]
    pub(crate) fn year_labels(&self) -> Vec<String> {
        self.imp()
            .years
            .borrow()
            .iter()
            .filter(|label| label.is_child_visible())
            .map(|label| label.text().to_string())
            .collect()
    }

    /// The month the scrubber names at `fraction` of its track, for tests.
    #[cfg(test)]
    pub(crate) fn month_at(&self, fraction: f64) -> Option<String> {
        self.month_at_fraction(fraction)
    }

    /// A click at `fraction` of the track, for tests.
    #[cfg(test)]
    pub(crate) fn click_at(&self, fraction: f64) {
        self.jump(fraction);
    }

    /// A drag from `from` to `to`, fractions of the track, for tests.
    #[cfg(test)]
    pub(crate) fn drag(&self, from: f64, to: f64) {
        for step in 0..=10 {
            self.jump(from + (to - from) * f64::from(step) / 10.0);
        }
    }

    /// A press on the up arrow (`up`) or the down arrow.
    pub(crate) fn step(&self, up: bool) {
        self.step_by(up);
    }
}

fn translate(x: i32, y: i32) -> gtk::gsk::Transform {
    #[expect(clippy::cast_precision_loss, reason = "pixel positions")]
    gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, y as f32))
}
