// SPDX-License-Identifier: AGPL-3.0-only
//! A small spinning progress ring, as Windows 11 File Explorer shows on a
//! tab while its search's results are still coming in or its folder is
//! loading (checked on Windows 11 build 26200): an arc in the accent
//! colour that turns clockwise.
//!
//! Drawn as the drive ring of Properties is drawn
//! (`properties/space_ring.rs`): [`render`] computes the arc's pixels at
//! the device's scale, sampling each pixel sixteen times for smooth
//! edges, once for each of [`FRAMES`] turns of the arc, and the widget
//! shows them in turn as textures in a `gtk::Picture`, one per frame of
//! the frame clock. The colour is the skin's accent (`progressring` in
//! `title-bar.css`). With animations off (`gtk-enable-animations`, ACC-010)
//! the ring stands still, as the address bar's progress does.

use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

/// The arc's length, as a share of the circle.
const ARC: f64 = 0.3;
/// The line's width, as a share of the diameter: 2 pixels at 16.
const THICKNESS: f64 = 0.125;
/// Samples per pixel along each axis.
const SAMPLES: usize = 4;
/// Pictures of the arc in one turn.
const FRAMES: usize = 30;
/// How long one turn takes.
const TURN: Duration = Duration::from_millis(1100);

/// An RGBA colour, not premultiplied.
type Rgba = [u8; 4];

/// The pixels of a ring `size` pixels square whose arc starts `start`
/// turns (0–1) clockwise from the top, as premultiplied RGBA rows.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel coordinates and channels far inside every type's range"
)]
pub(crate) fn render(start: f64, size: usize, colour: Rgba) -> Vec<u8> {
    let mut image = vec![0_u8; size * size * 4];
    let centre = size as f64 / 2.0;
    let outer = centre;
    let inner = outer - size as f64 * THICKNESS;
    let samples = (SAMPLES * SAMPLES) as f64;
    let alpha = f64::from(colour[3]) / 255.0;
    for y in 0..size {
        for x in 0..size {
            let mut covered = 0.0_f64;
            for sub_y in 0..SAMPLES {
                for sub_x in 0..SAMPLES {
                    let dx = x as f64 + (sub_x as f64 + 0.5) / SAMPLES as f64 - centre;
                    let dy = y as f64 + (sub_y as f64 + 0.5) / SAMPLES as f64 - centre;
                    let distance = dx.hypot(dy);
                    if distance > outer || distance < inner {
                        continue;
                    }
                    // Turns from the top, clockwise, counted from the arc's start.
                    let turn = dx.atan2(-dy) / std::f64::consts::TAU;
                    if (turn - start).rem_euclid(1.0) < ARC {
                        covered += 1.0;
                    }
                }
            }
            let share = covered / samples * alpha;
            let at = (y * size + x) * 4;
            for channel in 0..3 {
                image[at + channel] = (f64::from(colour[channel]) * share).round() as u8;
            }
            image[at + 3] = (share * 255.0).round() as u8;
        }
    }
    image
}

/// `color` as RGBA bytes.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channels between 0 and 1"
)]
fn bytes_of(color: &gdk::RGBA) -> Rgba {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        byte(color.red()),
        byte(color.green()),
        byte(color.blue()),
        byte(color.alpha()),
    ]
}

/// The frame shown `elapsed` into the turning.
fn frame_at(elapsed: Duration) -> usize {
    let turn = TURN.as_micros();
    let into = elapsed.as_micros() % turn;
    usize::try_from(into * FRAMES as u128 / turn).unwrap_or_default()
}

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gdk, glib};

    use super::Rgba;

    /// Private state of [`super::ProgressRing`].
    #[derive(Debug, Default)]
    pub(crate) struct ProgressRing {
        /// Shows the frame of the moment.
        pub(super) picture: RefCell<Option<gtk::Picture>>,
        /// The diameter, in logical pixels.
        pub(super) diameter: Cell<i32>,
        /// One texture for each turn of the arc.
        pub(super) frames: RefCell<Vec<gdk::MemoryTexture>>,
        /// The frame shown.
        pub(super) shown: Cell<usize>,
        /// What the frames were drawn for: their size in pixels and the
        /// colour.
        pub(super) drawn: Cell<Option<(i32, Rgba)>>,
        /// Turns the arc while the ring is on screen and animations are on.
        pub(super) tick: RefCell<Option<gtk::TickCallbackId>>,
        /// Follows the desktop's animation setting.
        pub(super) setting: RefCell<Option<(gtk::Settings, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProgressRing {
        const NAME: &'static str = "OxProgressRing";
        type Type = super::ProgressRing;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("progressring");
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for ProgressRing {
        fn constructed(&self) {
            self.parent_constructed();
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Fill)
                .can_shrink(true)
                .accessible_role(gtk::AccessibleRole::Presentation)
                .build();
            picture.set_parent(&*self.obj());
            self.picture.replace(Some(picture));
        }

        fn dispose(&self) {
            self.obj().stop_turning();
            if let Some((settings, handler)) = self.setting.take() {
                settings.disconnect(handler);
            }
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for ProgressRing {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let diameter = self.diameter.get();
            (diameter, diameter, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().lay_out(width, height);
        }

        fn map(&self) {
            self.parent_map();
            self.obj().follow_setting();
        }

        fn unmap(&self) {
            self.obj().stop_turning();
            self.parent_unmap();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let recoloured = self.drawn.get().is_some_and(|(_, colour)| colour != obj.colour());
            if recoloured {
                obj.queue_allocate();
            }
            self.parent_snapshot(snapshot);
        }
    }
}

glib::wrapper! {
    /// A spinning ring that shows something is in progress.
    pub(crate) struct ProgressRing(ObjectSubclass<imp::ProgressRing>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ProgressRing {
    /// A ring `diameter` logical pixels across.
    pub(crate) fn new(diameter: i32) -> Self {
        let ring: Self = glib::Object::new();
        ring.imp().diameter.set(diameter);
        ring.set_valign(gtk::Align::Center);
        ring
    }

    /// Whether the arc is turning: on screen, with animations on.
    pub(crate) fn is_turning(&self) -> bool {
        self.imp().tick.borrow().is_some()
    }

    /// The frame shown now.
    #[cfg(test)]
    pub(crate) fn shown_frame(&self) -> usize {
        self.imp().shown.get()
    }

    /// Turns the arc while the desktop's animations are on and leaves it
    /// still while they are off, from now on.
    fn follow_setting(&self) {
        let settings = self.settings();
        if self.imp().setting.borrow().is_none() {
            let handler = settings.connect_gtk_enable_animations_notify(glib::clone!(
                #[weak(rename_to = ring)]
                self,
                move |_| ring.follow_setting()
            ));
            self.imp().setting.replace(Some((settings.clone(), handler)));
        }
        if settings.is_gtk_enable_animations() && self.is_mapped() {
            self.start_turning();
        } else {
            self.stop_turning();
            self.show_frame(0);
        }
    }

    fn start_turning(&self) {
        if self.is_turning() {
            return;
        }
        let started = std::cell::Cell::new(None::<i64>);
        let tick = self.add_tick_callback(move |ring, clock| {
            let now = clock.frame_time();
            let first = *started.get().get_or_insert(now);
            let elapsed = Duration::from_micros(u64::try_from(now - first).unwrap_or_default());
            ring.show_frame(frame_at(elapsed));
            glib::ControlFlow::Continue
        });
        self.imp().tick.replace(Some(tick));
    }

    fn stop_turning(&self) {
        if let Some(tick) = self.imp().tick.take() {
            tick.remove();
        }
    }

    /// Shows frame `index`, if it is drawn.
    fn show_frame(&self, index: usize) {
        let imp = self.imp();
        imp.shown.set(index);
        let frames = imp.frames.borrow();
        if let (Some(frame), Some(picture)) = (frames.get(index), imp.picture.borrow().as_ref()) {
            picture.set_paintable(Some(frame));
        }
    }

    /// Places the picture in `width` × `height`, and draws the frames
    /// again when their size, scale or colour changed.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "pixel sizes and frame counts far inside every type's range"
    )]
    fn lay_out(&self, width: i32, height: i32) {
        let imp = self.imp();
        if let Some(picture) = imp.picture.borrow().as_ref() {
            picture.allocate(width, height, -1, None);
        }
        let side = (f64::from(width.min(height)) * self.device_scale()).ceil() as i32;
        let colour = self.colour();
        let key = (side, colour);
        if imp.drawn.get() == Some(key) || side <= 0 {
            return;
        }
        imp.drawn.set(Some(key));
        let size = side as usize;
        let frames = (0..FRAMES)
            .map(|frame| {
                gdk::MemoryTexture::new(
                    side,
                    side,
                    gdk::MemoryFormat::R8g8b8a8Premultiplied,
                    &glib::Bytes::from_owned(render(frame as f64 / FRAMES as f64, size, colour)),
                    size * 4,
                )
            })
            .collect();
        imp.frames.replace(frames);
        self.show_frame(imp.shown.get());
    }

    /// The skin's colour for the ring.
    fn colour(&self) -> Rgba {
        bytes_of(&self.color())
    }

    /// The pixels per logical pixel of the surface the ring shows on.
    fn device_scale(&self) -> f64 {
        self.native()
            .and_then(|native| native.surface())
            .map_or_else(|| f64::from(self.scale_factor()), |surface| surface.scale())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCENT: Rgba = [0, 103, 192, 255];

    fn pixel(image: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let at = (y * size + x) * 4;
        image[at..at + 4].try_into().expect("four channels")
    }

    #[test]
    fn the_arc_starts_where_it_is_turned_to_and_runs_clockwise() {
        let size = 64;
        let edge = 2;
        let image = render(0.0, size, ACCENT);
        assert_eq!(pixel(&image, size, 40, edge), ACCENT, "just right of the top");
        assert_eq!(pixel(&image, size, 24, edge), [0; 4], "just left of the top");
        assert_eq!(pixel(&image, size, 32, 32), [0; 4], "the middle is empty");
        assert_eq!(
            pixel(&image, size, 32, size - 1 - edge),
            [0; 4],
            "the bottom is empty"
        );

        let turned = render(0.5, size, ACCENT);
        assert_eq!(
            pixel(&turned, size, 24, size - 1 - edge),
            ACCENT,
            "half a turn on"
        );
        assert_eq!(pixel(&turned, size, 40, edge), [0; 4]);
    }

    #[test]
    fn a_turn_takes_every_frame_once() {
        assert_eq!(frame_at(Duration::ZERO), 0);
        assert_eq!(frame_at(TURN / 2), FRAMES / 2);
        assert_eq!(frame_at(TURN), 0);
        assert_eq!(frame_at(TURN * 3 + TURN / 30 + Duration::from_millis(1)), 1);
    }
}
