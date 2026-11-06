// SPDX-License-Identifier: AGPL-3.0-only
//! The ring a drive's General tab draws under its sizes, as Windows 11's
//! does: the used share in the accent colour, from the top clockwise, and
//! the free share in light grey.
//!
//! Drawn as the transfer window's speed graph is drawn: [`render`]
//! computes the picture's pixels at the device's scale, sampling each
//! pixel sixteen times for smooth edges, and the widget shows them as a
//! texture in a `gtk::Picture`. The colours come from the skin
//! (`properties.css`) through two hidden helper widgets, and the ring is
//! drawn again only when its size, scale or colours change. It never
//! moves, so reduced motion has nothing to stop. For assistive
//! technologies it is a picture named "60.5 GB free of 237 GB".

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

/// The ring's outer diameter, in logical pixels.
const DIAMETER: i32 = 96;
/// Its width from the outer edge in, as a share of the diameter.
const THICKNESS: f64 = 0.22;
/// Samples per pixel along each axis.
const SAMPLES: usize = 4;

/// An RGBA colour, not premultiplied.
type Rgba = [u8; 4];

/// The ring's colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Palette {
    /// The used space.
    pub(super) used: Rgba,
    /// The free space.
    pub(super) free: Rgba,
}

/// The pixels of a ring `size` pixels square, `used` of it (0–1) in the
/// used colour from the top clockwise, as premultiplied RGBA rows.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel coordinates and channels far inside every type's range"
)]
pub(super) fn render(used: f64, size: usize, palette: Palette) -> Vec<u8> {
    let mut image = vec![0_u8; size * size * 4];
    let centre = size as f64 / 2.0;
    let outer = centre;
    let inner = outer * (1.0 - 2.0 * THICKNESS);
    let reach = used.clamp(0.0, 1.0) * std::f64::consts::TAU;
    let samples = (SAMPLES * SAMPLES) as f64;
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0.0_f64; 4];
            for sub_y in 0..SAMPLES {
                for sub_x in 0..SAMPLES {
                    let dx = x as f64 + (sub_x as f64 + 0.5) / SAMPLES as f64 - centre;
                    let dy = y as f64 + (sub_y as f64 + 0.5) / SAMPLES as f64 - centre;
                    let distance = dx.hypot(dy);
                    if distance > outer || distance < inner {
                        continue;
                    }
                    // Zero at the top, growing clockwise.
                    let angle = dx.atan2(-dy).rem_euclid(std::f64::consts::TAU);
                    let colour = if angle < reach { palette.used } else { palette.free };
                    let alpha = f64::from(colour[3]) / 255.0;
                    for channel in 0..3 {
                        sum[channel] += f64::from(colour[channel]) * alpha;
                    }
                    sum[3] += alpha * 255.0;
                }
            }
            let at = (y * size + x) * 4;
            for channel in 0..4 {
                image[at + channel] = (sum[channel] / samples).round().clamp(0.0, 255.0) as u8;
            }
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

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::Palette;

    /// Private state of [`super::SpaceRing`].
    #[derive(Debug, Default)]
    pub(crate) struct SpaceRing {
        /// Shows the rendered pixels.
        pub(super) picture: RefCell<Option<gtk::Picture>>,
        /// Hidden widgets whose CSS colour is the used and the free
        /// space's.
        pub(super) swatches: RefCell<Vec<gtk::Widget>>,
        /// The used share, 0–1.
        pub(super) used: Cell<f64>,
        /// What the texture was drawn for: its size in pixels, the share
        /// and the colours.
        pub(super) drawn: Cell<Option<(i32, u64, Palette)>>,
        /// The redraw after a theme change, when one is due.
        pub(super) recolour: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpaceRing {
        const NAME: &'static str = "OxSpaceRing";
        type Type = super::SpaceRing;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("spacering");
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for SpaceRing {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            if let Some(source) = self.recolour.take() {
                source.remove();
            }
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for SpaceRing {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (super::DIAMETER, super::DIAMETER, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().lay_out(width, height);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().follow_theme();
            self.parent_snapshot(snapshot);
        }
    }
}

glib::wrapper! {
    /// The ring of a drive's used and free space.
    pub(crate) struct SpaceRing(ObjectSubclass<imp::SpaceRing>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SpaceRing {
    /// A ring showing `free` of `total` bytes free, named `text`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a share of a ring needs no exact bytes"
    )]
    pub(crate) fn new(free: u64, total: u64, text: &str) -> Self {
        let ring: Self = glib::Object::new();
        let used = if total == 0 {
            0.0
        } else {
            total.saturating_sub(free) as f64 / total as f64
        };
        ring.imp().used.set(used);
        ring.update_property(&[gtk::accessible::Property::Label(text)]);
        ring
    }

    fn build(&self) {
        let imp = self.imp();
        self.set_halign(gtk::Align::Center);
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Fill)
            .can_shrink(true)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        picture.set_parent(self);
        let swatches = ["space-used", "space-free"].map(|class| {
            let swatch = gtk::Box::builder().visible(false).build();
            swatch.add_css_class(class);
            swatch.set_parent(self);
            swatch.upcast::<gtk::Widget>()
        });
        imp.picture.replace(Some(picture));
        imp.swatches.replace(swatches.to_vec());
    }

    /// Places the picture in `width` × `height`, and draws it again when
    /// its size, scale or colours changed.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "pixel sizes far inside every type's range"
    )]
    fn lay_out(&self, width: i32, height: i32) {
        let imp = self.imp();
        if let Some(picture) = imp.picture.borrow().as_ref() {
            picture.allocate(width, height, -1, None);
        }
        let side = (f64::from(width.min(height)) * self.device_scale()).ceil() as i32;
        let palette = self.palette();
        let used = imp.used.get();
        let key = (side, used.to_bits(), palette);
        if imp.drawn.get() == Some(key) || side <= 0 {
            return;
        }
        imp.drawn.set(Some(key));
        let size = side as usize;
        let texture = gdk::MemoryTexture::new(
            side,
            side,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &glib::Bytes::from_owned(render(used, size, palette)),
            size * 4,
        );
        if let Some(picture) = imp.picture.borrow().as_ref() {
            picture.set_paintable(Some(&texture));
        }
    }

    /// The colours of the skin, as last styled.
    fn palette(&self) -> Palette {
        let swatches = self.imp().swatches.borrow();
        let colour = |index: usize| {
            swatches
                .get(index)
                .map_or([0; 4], |swatch| bytes_of(&swatch.color()))
        };
        Palette {
            used: colour(0),
            free: colour(1),
        }
    }

    /// Draws the ring again soon after the theme changed its colours.
    fn follow_theme(&self) {
        let imp = self.imp();
        let changed = imp
            .drawn
            .get()
            .is_some_and(|(_, _, palette)| palette != self.palette());
        if !changed || imp.recolour.borrow().is_some() {
            return;
        }
        let source = glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = ring)]
            self,
            move || {
                ring.imp().recolour.take();
                ring.queue_allocate();
            }
        ));
        imp.recolour.replace(Some(source));
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

    const PALETTE: Palette = Palette {
        used: [0, 103, 192, 255],
        free: [200, 200, 200, 255],
    };

    fn pixel(image: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let at = (y * size + x) * 4;
        image[at..at + 4].try_into().expect("four channels")
    }

    #[test]
    fn the_used_share_runs_clockwise_from_the_top() {
        let size = 100;
        let image = render(0.25, size, PALETTE);
        let ring = 5;

        assert_eq!(
            pixel(&image, size, 60, ring),
            PALETTE.used,
            "just right of the top"
        );
        assert_eq!(
            pixel(&image, size, 40, ring),
            PALETTE.free,
            "just left of the top"
        );
        assert_eq!(
            pixel(&image, size, size - 1 - ring, 60),
            PALETTE.free,
            "below three o'clock"
        );
        assert_eq!(pixel(&image, size, 50, 50), [0; 4], "the middle is empty");
        assert_eq!(pixel(&image, size, 0, 0), [0; 4], "the corners are empty");
    }
}
