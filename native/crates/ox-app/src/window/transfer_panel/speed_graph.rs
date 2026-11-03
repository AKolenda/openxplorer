// SPDX-License-Identifier: AGPL-3.0-only
//! The speed graph of "More details" (OPS-021): how fast the action went
//! as it progressed, as the Windows progress window draws it: the speed
//! as a filled area up to the current percentage over faint gridlines,
//! the part done shaded, a line across at the current speed and "Speed:
//! 32.8 MB/s" at its top right. Windows' detailed view has "a real-time
//! throughput graph" (Building Windows 8, "Improving our file management
//! basics", 2011).
//!
//! [`render`] computes the picture's pixels at the device's scale, with
//! an anti-aliased top edge that runs straight from one report's speed to
//! the next; the widget shows them as a texture in a `gtk::Picture`,
//! drawn again only when the action reports, the size or the scale
//! changes, or the theme changes the colours. The colours come from the
//! skin (`dialogs.css`) through three hidden helper widgets. The label is
//! a real label over the picture, so it is text for the screen reader
//! and sharp at any scale. Nothing moves frame by frame, so reduced
//! motion has nothing to stop (ACC-010).
//!
//! For assistive technologies the graph is the action's progress bar
//! while the details show (the plain bar hides then), named "Operation
//! progress", with the speed as its description.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

/// The graph's height, in logical pixels.
const HEIGHT: i32 = 72;
/// Room above the highest speed, so the line and its label stay inside.
const HEADROOM: f64 = 1.3;
/// The gridlines: columns and rows of the grid.
const GRID_COLUMNS: usize = 10;
const GRID_ROWS: usize = 4;
/// Reports closer than this share one point of the graph.
const SAMPLE_STEP: f64 = 0.002;

/// An RGBA colour, not premultiplied.
type Rgba = [u8; 4];

/// The colours of the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Palette {
    /// The area under the speed.
    pub(super) area: Rgba,
    /// The part done, behind the area.
    pub(super) reached: Rgba,
    /// The gridlines.
    pub(super) grid: Rgba,
    /// The line at the current speed.
    pub(super) line: Rgba,
}

/// What the graph shows.
#[derive(Debug, Clone, Copy)]
pub(super) struct Plot<'a> {
    /// The speed (bytes per second) at each progress (0–1) reported,
    /// in order.
    pub(super) samples: &'a [(f64, u64)],
    /// How far the action is (0–1).
    pub(super) progress: f64,
    /// The current speed, when known.
    pub(super) speed: Option<u64>,
}

impl Plot<'_> {
    /// The speed the top of the graph stands for.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a pixel height needs far less precision"
    )]
    fn ceiling(&self) -> f64 {
        let highest = self
            .samples
            .iter()
            .map(|(_, speed)| *speed)
            .chain(self.speed)
            .max();
        highest.unwrap_or(0) as f64 * HEADROOM
    }

    /// The speed at `fraction`, running straight from one report to the
    /// next; the first report's speed before it.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a pixel height needs far less precision"
    )]
    fn speed_at(&self, fraction: f64) -> Option<f64> {
        let first = self.samples.first()?;
        if fraction <= first.0 {
            return Some(first.1 as f64);
        }
        let next = self.samples.iter().position(|(at, _)| *at >= fraction);
        let Some(next) = next else {
            return self.samples.last().map(|(_, speed)| *speed as f64);
        };
        let (from, to) = (self.samples[next - 1], self.samples[next]);
        let share = if to.0 > from.0 {
            (fraction - from.0) / (to.0 - from.0)
        } else {
            1.0
        };
        Some(from.1 as f64 + (to.1 as f64 - from.1 as f64) * share)
    }
}

/// The row of the line at the current speed in a picture `height` pixels
/// high, when the speed is known.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a row inside a picture far smaller than f64's precision"
)]
pub(super) fn line_row(plot: &Plot, height: usize) -> Option<usize> {
    let ceiling = plot.ceiling();
    let speed = plot.speed? as f64;
    if ceiling <= 0.0 || height == 0 {
        return None;
    }
    let top = height as f64 - speed / ceiling * height as f64;
    Some((top.round().max(0.0) as usize).min(height - 1))
}

/// The pixels of `plot`, `width` × `height`, as premultiplied RGBA rows.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel coordinates far inside f64's precision"
)]
pub(super) fn render(plot: &Plot, width: usize, height: usize, palette: &Palette) -> Vec<u8> {
    let mut image = vec![0_u8; width * height * 4];
    if width == 0 || height == 0 {
        return image;
    }
    let (w, h) = (width as f64, height as f64);
    let reach = plot.progress.clamp(0.0, 1.0) * w;
    let ceiling = plot.ceiling();
    let thickness = (height / HEIGHT as usize).max(1);
    let grid_x: Vec<usize> = (1..GRID_COLUMNS)
        .map(|i| (i as f64 * w / GRID_COLUMNS as f64).round() as usize)
        .collect();
    let grid_y: Vec<usize> = (1..GRID_ROWS)
        .map(|i| (i as f64 * h / GRID_ROWS as f64).round() as usize)
        .collect();
    for x in 0..width {
        let inside = (reach - x as f64).clamp(0.0, 1.0);
        let top = (ceiling > 0.0 && inside > 0.0)
            .then(|| plot.speed_at((x as f64 + 0.5) / w))
            .flatten()
            .map(|speed| h - speed / ceiling * h);
        for y in 0..height {
            let at = (y * width + x) * 4;
            let pixel = &mut image[at..at + 4];
            blend(pixel, palette.reached, inside);
            if grid_x.contains(&x) || grid_y.contains(&y) {
                blend(pixel, palette.grid, 1.0);
            }
            if let Some(top) = top {
                let covered = (y as f64 + 1.0 - top).clamp(0.0, 1.0);
                blend(pixel, palette.area, covered * inside);
            }
        }
    }
    if let Some(row) = line_row(plot, height) {
        for y in row..(row + thickness).min(height) {
            for x in 0..width {
                let at = (y * width + x) * 4;
                blend(&mut image[at..at + 4], palette.line, 1.0);
            }
        }
    }
    image
}

/// Paints `color` over the premultiplied `pixel`, covering `coverage` of
/// it.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channels stay between 0 and 255"
)]
fn blend(pixel: &mut [u8], color: Rgba, coverage: f64) {
    let alpha = f64::from(color[3]) / 255.0 * coverage;
    if alpha <= 0.0 {
        return;
    }
    for channel in 0..3 {
        let source = f64::from(color[channel]) * alpha;
        let below = f64::from(pixel[channel]) * (1.0 - alpha);
        pixel[channel] = (source + below).round().clamp(0.0, 255.0) as u8;
    }
    let below = f64::from(pixel[3]) * (1.0 - alpha);
    pixel[3] = (alpha * 255.0 + below).round().clamp(0.0, 255.0) as u8;
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

    /// Private state of [`super::SpeedGraph`].
    #[derive(Debug, Default)]
    pub(crate) struct SpeedGraph {
        /// Shows the rendered pixels.
        pub(super) picture: RefCell<Option<gtk::Picture>>,
        /// "Speed: 32.8 MB/s".
        pub(super) label: RefCell<Option<gtk::Label>>,
        /// Hidden widgets whose CSS colour is the area's, the part done's,
        /// the gridlines' and the speed line's.
        pub(super) swatches: RefCell<Vec<gtk::Widget>>,
        /// The speed at each progress reported.
        pub(super) samples: RefCell<Vec<(f64, u64)>>,
        /// How far the action is.
        pub(super) progress: Cell<f64>,
        /// The current speed.
        pub(super) speed: Cell<Option<u64>>,
        /// What the texture was drawn for: its size in pixels and colours.
        pub(super) drawn: Cell<Option<(i32, i32, Palette)>>,
        /// Whether the data changed since the texture was drawn.
        pub(super) stale: Cell<bool>,
        /// The redraw after a theme change, when one is due.
        pub(super) recolour: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpeedGraph {
        const NAME: &'static str = "OxSpeedGraph";
        type Type = super::SpeedGraph;
        type ParentType = gtk::Widget;

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

        fn dispose(&self) {
            if let Some(source) = self.recolour.take() {
                source.remove();
            }
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for SpeedGraph {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (0, 0, -1, -1),
                _ => (super::HEIGHT, super::HEIGHT, -1, -1),
            }
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
    /// The speed graph of an action's details.
    pub(crate) struct SpeedGraph(ObjectSubclass<imp::SpeedGraph>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SpeedGraph {
    fn build(&self) {
        let imp = self.imp();
        self.set_hexpand(true);
        self.set_overflow(gtk::Overflow::Hidden);
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Fill)
            .can_shrink(true)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        picture.set_parent(self);
        let label = gtk::Label::builder().visible(false).build();
        label.add_css_class("speed-label");
        label.set_parent(self);
        let swatches = ["speed-area", "speed-reached", "speed-grid", "speed-line"].map(|class| {
            let swatch = gtk::Box::builder().visible(false).build();
            swatch.add_css_class(class);
            swatch.set_parent(self);
            swatch.upcast::<gtk::Widget>()
        });
        imp.picture.replace(Some(picture));
        imp.label.replace(Some(label));
        imp.swatches.replace(swatches.to_vec());
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
        imp.samples.borrow_mut().clear();
        imp.progress.set(0.0);
        imp.speed.set(None);
        if let Some(label) = imp.label.borrow().as_ref() {
            label.set_visible(false);
        }
        self.update_property(&[gtk::accessible::Property::ValueNow(0.0)]);
        self.changed();
    }

    /// Shows the action at `fraction` (0–1), going at `speed` bytes per
    /// second when known, which the text says.
    pub(crate) fn show(&self, fraction: f64, speed: Option<(u64, String)>) {
        let imp = self.imp();
        let progress = fraction.clamp(0.0, 1.0).max(imp.progress.get());
        imp.progress.set(progress);
        self.update_property(&[gtk::accessible::Property::ValueNow((progress * 100.0).round())]);
        if let Some((speed, text)) = speed {
            imp.speed.set(Some(speed));
            let mut samples = imp.samples.borrow_mut();
            match samples.last_mut() {
                Some(last) if progress - last.0 < SAMPLE_STEP => last.1 = speed,
                _ => samples.push((progress, speed)),
            }
            drop(samples);
            if let Some(label) = imp.label.borrow().as_ref() {
                label.set_text(&text);
                label.set_visible(true);
            }
            self.update_property(&[gtk::accessible::Property::Description(&text)]);
        }
        self.changed();
    }

    /// Draws the graph again in the skin's colours, which a paused or
    /// resumed action has just changed: the area and the line turn the
    /// caution colour, or back, on the next frame.
    pub(crate) fn recolour(&self) {
        self.changed();
    }

    /// The data changed: draw again and place the label.
    fn changed(&self) {
        self.imp().stale.set(true);
        self.queue_allocate();
    }

    /// Places the picture and the label in `width` × `height`, and draws
    /// the picture again when anything it shows changed.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        reason = "pixel sizes far inside every type's range"
    )]
    fn lay_out(&self, width: i32, height: i32) {
        let imp = self.imp();
        if let Some(picture) = imp.picture.borrow().as_ref() {
            picture.allocate(width, height, -1, None);
        }
        let samples = imp.samples.borrow().clone();
        let plot = Plot {
            samples: &samples,
            progress: imp.progress.get(),
            speed: imp.speed.get(),
        };
        if let Some(label) = imp.label.borrow().as_ref().filter(|label| label.is_visible()) {
            let (_, natural) = label.preferred_size();
            let (label_width, label_height) = (natural.width().min(width), natural.height());
            let line = line_row(&plot, height.max(0) as usize).map_or(height, |row| row as i32);
            let x = width - label_width - 6;
            let y = (line - label_height - 1).clamp(0, (height - label_height).max(0));
            let at = gsk::Transform::new().translate(&graphene::Point::new(x.max(0) as f32, y as f32));
            label.allocate(label_width, label_height, -1, Some(at));
        }
        let scale = self.device_scale();
        let size = (
            (f64::from(width) * scale).ceil() as i32,
            (f64::from(height) * scale).ceil() as i32,
        );
        let palette = self.palette();
        let drawn = imp.drawn.get();
        if !imp.stale.get() && drawn == Some((size.0, size.1, palette)) {
            return;
        }
        imp.stale.set(false);
        imp.drawn.set(Some((size.0, size.1, palette)));
        if size.0 <= 0 || size.1 <= 0 {
            return;
        }
        let (columns, rows) = (size.0 as usize, size.1 as usize);
        let pixels = render(&plot, columns, rows, &palette);
        let texture = gdk::MemoryTexture::new(
            size.0,
            size.1,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &glib::Bytes::from_owned(pixels),
            columns * 4,
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
            area: colour(0),
            reached: colour(1),
            grid: colour(2),
            line: colour(3),
        }
    }

    /// Draws the graph again soon after the theme changed its colours.
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
            #[weak(rename_to = graph)]
            self,
            move || {
                graph.imp().recolour.take();
                graph.changed();
            }
        ));
        imp.recolour.replace(Some(source));
    }

    /// The pixels per logical pixel of the surface the graph shows on.
    fn device_scale(&self) -> f64 {
        self.native()
            .and_then(|native| native.surface())
            .map_or_else(|| f64::from(self.scale_factor()), |surface| surface.scale())
    }

    /// The size of the drawn texture in pixels, for tests.
    #[cfg(test)]
    pub(crate) fn texture_size(&self) -> Option<(i32, i32)> {
        let picture = self.imp().picture.borrow().clone()?;
        let texture = picture.paintable().and_downcast::<gdk::Texture>()?;
        Some((texture.width(), texture.height()))
    }

    /// The colour of the drawn texture at (`x`, `y`), fractions of its
    /// width and height, as straight RGBA, for tests.
    #[cfg(test)]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a pixel inside a small test texture"
    )]
    pub(crate) fn texture_pixel(&self, x: f64, y: f64) -> Option<Rgba> {
        let picture = self.imp().picture.borrow().clone()?;
        let texture = picture.paintable().and_downcast::<gdk::Texture>()?;
        let (width, height) = (texture.width() as usize, texture.height() as usize);
        let mut downloader = gdk::TextureDownloader::new(&texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let column = ((x * width as f64) as usize).min(width - 1);
        let row = ((y * height as f64) as usize).min(height - 1);
        let at = row * stride + column * 4;
        bytes.get(at..at + 4)?.try_into().ok()
    }

    /// The row of the speed line in the drawn texture, as a fraction of
    /// its height, for tests.
    #[cfg(test)]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "a row inside a small test texture"
    )]
    pub(crate) fn line_fraction(&self) -> Option<f64> {
        let (_, height) = self.texture_size()?;
        let samples = self.imp().samples.borrow().clone();
        let plot = Plot {
            samples: &samples,
            progress: self.imp().progress.get(),
            speed: self.imp().speed.get(),
        };
        let row = line_row(&plot, height as usize)?;
        Some((row as f64 + 0.5) / f64::from(height))
    }

    /// The colour of the caution swatch the skin gives a paused graph's
    /// area, or the accent one otherwise, as RGB, for tests.
    #[cfg(test)]
    pub(crate) fn area_rgb(&self) -> [u8; 3] {
        let area = self.palette().area;
        [area[0], area[1], area[2]]
    }

    /// How far the graph shows the action, and its speed, for tests.    /// How far the graph shows the action, and its speed, for tests.
    #[cfg(test)]
    pub(crate) fn shown(&self) -> (f64, Option<u64>) {
        (self.imp().progress.get(), self.imp().speed.get())
    }

    /// The speed label's text, for tests.
    #[cfg(test)]
    pub(crate) fn speed_text(&self) -> Option<String> {
        let label = self.imp().label.borrow().clone()?;
        label.is_visible().then(|| label.text().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The graph says the current speed and is the action's progress bar
    /// for assistive technologies; a new action empties it.
    ///
    /// parity: OPS-021
    #[gtk::test]
    fn the_graph_says_the_speed_and_is_a_progress_bar() {
        let graph: SpeedGraph = glib::Object::new();

        graph.show(0.2, Some((10, "Speed: 10 bytes/s".to_owned())));
        graph.show(0.44, Some((20, "Speed: 20 bytes/s".to_owned())));

        assert_eq!(graph.shown(), (0.44, Some(20)));
        assert_eq!(graph.speed_text().as_deref(), Some("Speed: 20 bytes/s"));
        assert_eq!(graph.accessible_role(), gtk::AccessibleRole::ProgressBar);
        graph.reset();
        assert_eq!((graph.shown(), graph.speed_text()), ((0.0, None), None));
    }
}

#[cfg(test)]
mod pixel_tests {
    use super::*;

    const AREA: [u8; 4] = [0, 0, 255, 255];
    const REACHED: [u8; 4] = [200, 200, 255, 255];
    const GRID: [u8; 4] = [10, 10, 10, 40];
    const LINE: [u8; 4] = [255, 255, 255, 255];
    const PALETTE: Palette = Palette {
        area: AREA,
        reached: REACHED,
        grid: GRID,
        line: LINE,
    };

    fn pixel(buffer: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let at = (y * width + x) * 4;
        buffer[at..at + 4].try_into().unwrap()
    }

    /// The area under the speed is filled up to the progress, the part
    /// done is shaded behind it, and nothing is drawn beyond it but the
    /// grid.
    ///
    /// parity: OPS-021
    #[test]
    fn the_area_fills_the_speed_up_to_the_progress_and_no_further() {
        let samples = [(0.1, 100), (0.4, 100)];
        let plot = Plot {
            samples: &samples,
            progress: 0.4,
            speed: Some(100),
        };
        let image = render(&plot, 200, 100, &PALETTE);

        assert_eq!(image.len(), 200 * 100 * 4);
        assert_eq!(pixel(&image, 200, 41, 98), AREA, "under the speed");
        assert_eq!(
            pixel(&image, 200, 41, 3),
            REACHED,
            "above the speed, inside the part done"
        );
        assert_ne!(pixel(&image, 200, 121, 98), AREA, "nothing beyond the progress");
        assert_ne!(pixel(&image, 200, 121, 98), REACHED);
    }

    /// The area's top edge follows the speed smoothly from one report to
    /// the next, with no columns or steps.
    ///
    /// parity: OPS-021
    #[test]
    fn the_area_rises_smoothly_with_the_speed() {
        let samples = [(0.0, 50), (0.5, 100)];
        let plot = Plot {
            samples: &samples,
            progress: 0.5,
            speed: Some(100),
        };
        let image = render(&plot, 200, 100, &PALETTE);
        let top = |x| (0..100).find(|&y| pixel(&image, 200, x, y) == AREA).unwrap();
        let tops: Vec<usize> = (2..98).map(top).collect();

        assert!(
            tops.windows(2).all(|pair| pair[1] <= pair[0]),
            "it only rises: {tops:?}"
        );
        assert!(
            tops.windows(2).all(|pair| pair[0] - pair[1] <= 1),
            "no steps: {tops:?}"
        );
        assert!(
            tops[0] > tops[95] + 20,
            "half the speed is clearly lower: {tops:?}"
        );
    }

    /// The line at the current speed crosses the whole graph.
    ///
    /// parity: OPS-021
    #[test]
    fn the_line_crosses_the_graph_at_the_current_speed() {
        let samples = [(0.2, 80)];
        let plot = Plot {
            samples: &samples,
            progress: 0.2,
            speed: Some(80),
        };
        let image = render(&plot, 200, 100, &PALETTE);
        let row = line_row(&plot, 100).expect("a speed is known");

        for x in [0, 100, 199] {
            assert_eq!(pixel(&image, 200, x, row), LINE, "at x {x}");
        }
        assert!(row > 10 && row < 90, "inside the graph: {row}");
    }

    /// A first report that is slower or faster than the rest makes no
    /// full-height column at the left edge.
    #[test]
    fn the_left_edge_has_no_full_height_column() {
        let samples = [(0.02, 30), (0.3, 100), (0.6, 100)];
        let plot = Plot {
            samples: &samples,
            progress: 0.6,
            speed: Some(100),
        };
        let image = render(&plot, 200, 100, &PALETTE);

        for y in 0..20 {
            assert_ne!(pixel(&image, 200, 0, y), AREA, "row {y} of the left edge");
        }
    }

    /// The graph is drawn at the device's pixels, so it stays sharp at a
    /// scale of 2.
    #[gtk::test]
    fn the_graph_is_drawn_at_the_devices_pixels() {
        let graph: SpeedGraph = glib::Object::new();
        let window = gtk::Window::builder().child(&graph).default_width(300).build();
        window.set_visible(true);
        graph.show(0.5, Some((100, "Speed: 100 bytes/s".to_owned())));
        crate::test_support::harness::wait_until("the graph to be drawn", || graph.texture_size().is_some());

        let scale = graph.scale_factor();
        assert_eq!(
            graph.texture_size(),
            Some((graph.width() * scale, graph.height() * scale))
        );
        window.destroy();
    }
}
