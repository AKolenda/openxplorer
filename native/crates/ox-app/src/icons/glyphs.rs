// SPDX-License-Identifier: AGPL-3.0-only
//! Stroke glyphs: the line icons of the command bar, sidebar and menus.
//!
//! The path data is the `paths` table at the top of `desktop/ui/app.js`
//! (24-unit viewBox, 1.35 stroke, round caps and joins; `more` uses a
//! 3-unit stroke so its dots are visible). From 16 pixels up the stroke is
//! never thinner than one pixel ([`Glyph::stroke_width`]).
//! [`GlyphPaintable`] draws a path with `gsk::Path` and implements
//! `GtkSymbolicPaintable`, so a `GtkImage` paints it in the widget's CSS
//! `color` and it follows hover, disabled and dark styles without any
//! image files.
//!
//! Glyphs are named by the [`Glyph`] enum, so a misspelt name does not
//! compile. (app.js looked names up at run time and silently drew the
//! documents glyph for an unknown one.)

use gtk::subclass::prelude::*;
use gtk::{gdk, glib, gsk};

/// The glyphs' coordinate space: a 24-unit square (the SVG viewBox).
const VIEWBOX_SIZE: f32 = 24.0;

/// The stroke of the web app's `paths` table, in viewBox units.
const WEB_STROKE_WIDTH: f32 = 1.35;

/// The heavier stroke of the `more` dots, so they are visible.
const MORE_DOTS_STROKE_WIDTH: f32 = 3.0;

/// The smallest glyph drawn with a stroke of at least one pixel.
const MONOLINE_FROM_SIZE: i32 = 16;

/// Ink for a glyph drawn outside a styled widget: the light theme's text.
const FALLBACK_INK: gdk::RGBA = gdk::RGBA::new(0.14, 0.14, 0.14, 1.0);

/// One line icon from the `paths` table in app.js, plus `restore` for the
/// maximised caption button.
///
/// The table is ported whole. Six glyphs are not drawn yet; each keeps an
/// `expect(dead_code)` naming the feature that draws it and the
/// `native/ROADMAP.md` milestone that ports it ("Then: recover the
/// remaining Python application services" or "Next: complete safe
/// file-operation workflows").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Glyph {
    /// A terminal window (Open in Terminal).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Open in Terminal (ROADMAP: application services)")
    )]
    Terminal,
    /// A gear.
    Settings,
    /// A plus sign.
    Plus,
    /// A cross.
    Close,
    /// A minus sign.
    Minus,
    /// A square (maximise caption button).
    Maximize,
    /// Two squares (restore caption button).
    Restore,
    /// An arrow to the left.
    Back,
    /// An arrow to the right.
    Forward,
    /// An arrow up.
    Up,
    /// A circular arrow.
    Refresh,
    /// A downward chevron.
    Down,
    /// A rightward chevron (the Settings search results).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Settings search (ROADMAP: application services)")
    )]
    Chevron,
    /// A magnifier.
    Search,
    /// A house.
    Home,
    /// A monitor.
    Desktop,
    /// An arrow into a tray.
    Downloads,
    /// A page with lines.
    Documents,
    /// A landscape picture.
    Pictures,
    /// Musical notes.
    Music,
    /// A film strip.
    Videos,
    /// A pin.
    Pin,
    /// Scissors.
    Cut,
    /// Two pages.
    Copy,
    /// A clipboard.
    Paste,
    /// A text cursor in a box.
    Rename,
    /// An arrow leaving a box.
    Share,
    /// A bin.
    Trash,
    /// A down arrow beside bars.
    Sort,
    /// Four squares.
    Grid,
    /// Bulleted lines.
    List,
    /// A window with a side pane.
    Details,
    /// Three dots.
    More,
    /// Connected boxes.
    Network,
    /// Two stacked server units.
    Server,
    /// A disk drive.
    Drive,
    /// A phone.
    Phone,
    /// A check mark.
    Check,
    /// A shield with a check mark (the sign-in dialog).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "sign-in dialog (ROADMAP: application services)")
    )]
    Shield,
    /// An "i" in a circle.
    Info,
    /// A sun.
    Sun,
    /// A crescent moon.
    Moon,
    /// An eject symbol (Sign out of server).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Sign out of server (ROADMAP: application services)")
    )]
    Eject,
    /// A clock (previous versions).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "previous versions (ROADMAP: application services)")
    )]
    Clock,
    /// A chain link (the context menu's Copy path).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "context-menu Copy path (ROADMAP: file operations)")
    )]
    Link,
    /// An eye.
    Eye,
    /// A folder outline.
    FolderLine,
    /// A diagonal cross.
    Cancel,
}

impl Glyph {
    /// The SVG path data, verbatim from app.js. An exhaustive table, hence
    /// its length.
    pub(super) const fn path_data(self) -> &'static str {
        match self {
            Glyph::Terminal => "M3 5h18v14H3zM6 9l3 3-3 3M12 15h5",
            Glyph::Settings => {
                "M10 2h4l1 3 3 1 3 2-2 3v2l2 3-3 2-3 1-1 3h-4l-1-3-3-1-3-2 2-3v-2L3 8l3-2 3-1zM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0"
            }
            Glyph::Plus => "M12 5v14M5 12h14",
            Glyph::Close => "m6 6 12 12M18 6 6 18",
            Glyph::Minus => "M5 12h14",
            Glyph::Maximize => "M5 5h14v14H5z",
            Glyph::Restore => "M8 8h11v11H8zM5 16V5h11",
            Glyph::Back => "m12 5-7 7 7 7M5 12h14",
            Glyph::Forward => "m12 5 7 7-7 7M5 12h14",
            Glyph::Up => "m5 12 7-7 7 7M12 5v14",
            Glyph::Refresh => "M19 10a7 7 0 1 0-1 7M19 4v6h-6",
            Glyph::Down => "m7 10 5 5 5-5",
            Glyph::Chevron => "m9 6 6 6-6 6",
            Glyph::Search => "M16 16l5 5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0",
            Glyph::Home => "m3 11 9-8 9 8M5 10v10h5v-6h4v6h5V10",
            Glyph::Desktop => "M3 4h18v13H3zM9 21h6M12 17v4",
            Glyph::Downloads => "M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5",
            Glyph::Documents => "M6 3h9l4 4v14H6zM14 3v5h5M9 12h7M9 16h7",
            Glyph::Pictures => "M3 4h18v16H3zM3 17l6-6 4 4 3-3 5 5M16 8h.01",
            Glyph::Music => "M9 18V5l11-2v13M9 7l11-2M9 18c0 2-6 3-6 0s6-3 6 0Zm11-2c0 2-6 3-6 0s6-3 6 0Z",
            Glyph::Videos => "M4 4h16v16H4zM4 8h16M4 16h16M8 4v4M16 4v4M8 16v4M16 16v4",
            Glyph::Pin => "m8 3 9 9M15 4l5 5-5 2-3 5-4-4-5-1 5-3 2-5M9 15l-6 6",
            Glyph::Cut => "m9 9 10 12M9 15 19 3M9 7a3 3 0 1 1-6 0 3 3 0 0 1 6 0Zm0 10a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z",
            Glyph::Copy => "M8 8h12v13H8zM16 8V3H3v13h5",
            Glyph::Paste => "M8 5H5v16h14V5h-3M9 3h6v4H9z",
            Glyph::Rename => "M3 7h7v10H3zM16 3v18M13 3h6M13 21h6M20 7h2v10h-2",
            Glyph::Share => "M14 4h7v7M21 4 10 15M10 5H4v16h16v-6",
            Glyph::Trash => "M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7M14 10v7",
            Glyph::Sort => "M7 3v18m-4-4 4 4 4-4M14 5h7M14 10h5M14 15h3",
            Glyph::Grid => "M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z",
            Glyph::List => "M3 5h2M9 5h12M3 12h2M9 12h12M3 19h2M9 19h12",
            Glyph::Details => "M3 4h18v16H3zM15 4v16M18 8h.01M18 12h.01M18 16h.01",
            Glyph::More => "M4 12h.01M12 12h.01M20 12h.01",
            Glyph::Network => "M8 3h8v6H8zM3 16h6v5H3zM15 16h6v5h-6zM12 9v4M6 16v-3h12v3",
            Glyph::Server => "M5 3h14v7H5zM5 14h14v7H5zM8 6.5h.01M8 17.5h.01M12 6.5h4M12 17.5h4",
            Glyph::Drive => "m5 5-3 11v5h20v-5L19 5zM2 16h20M17 18.5h.01M20 18.5h.01",
            Glyph::Phone => {
                "M8 2h8a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2zM10 5h4M11 19h2"
            }
            Glyph::Check => "m5 12 4 4L19 6",
            Glyph::Shield => "m12 2 8 3v6c0 5-8 11-8 11S4 16 4 11V5zM8 11l3 3 5-6",
            Glyph::Info => "M12 11v6M12 7h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0",
            Glyph::Sun => {
                "M12 2v2M12 20v2M2 12h2M20 12h2m-3-7 1.5-1.5M5 19l-1.5 1.5m0-17L5 5m14 14 1.5 1.5M17 12a5 5 0 1 1-10 0 5 5 0 0 1 10 0"
            }
            Glyph::Moon => "M20 15A9 9 0 0 1 9 4a9 9 0 1 0 11 11Z",
            Glyph::Eject => "m5 14 7-10 7 10zM5 20h14",
            Glyph::Clock => "M12 7v6l4 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0",
            Glyph::Link => {
                "m9 15 6-6M8 16l-1 1a4 4 0 0 1-6-6l5-5a4 4 0 0 1 6 0m0 2 1-1a4 4 0 0 1 6 6l-5 5a4 4 0 0 1-6 0"
            }
            Glyph::Eye => "M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
            Glyph::FolderLine => "M3 7V4h7l2 3h9v13H3z",
            Glyph::Cancel => "M5 5l14 14M5 19 19 5",
        }
    }

    /// Stroke width in viewBox units for the glyph drawn `size` pixels
    /// wide: 3 for the `more` dots, so they are visible, and 1.35 (the web
    /// app's) otherwise, but never under one pixel from 16 pixels up, the
    /// monoline stroke of Windows 11's icons (ui-spec.md I10). Smaller
    /// glyphs keep the web app's thinner line: a whole pixel there is
    /// visibly bolder, which awaits the owner's sign-off.
    fn stroke_width(self, size: i32) -> f32 {
        if self == Glyph::More {
            return MORE_DOTS_STROKE_WIDTH;
        }
        if size < MONOLINE_FROM_SIZE {
            return WEB_STROKE_WIDTH;
        }
        #[expect(clippy::cast_precision_loss, reason = "icon sizes fit an f32 exactly")]
        let one_pixel = VIEWBOX_SIZE / size as f32;
        WEB_STROKE_WIDTH.max(one_pixel)
    }

    /// The glyph for a known-folder icon name in ox-core's Quick access
    /// table (`desktop`, `downloads`, ...), or `None` for another name.
    pub(crate) fn for_known_folder(icon: &str) -> Option<Glyph> {
        match icon {
            "desktop" => Some(Glyph::Desktop),
            "downloads" => Some(Glyph::Downloads),
            "documents" => Some(Glyph::Documents),
            "pictures" => Some(Glyph::Pictures),
            "music" => Some(Glyph::Music),
            "videos" => Some(Glyph::Videos),
            _ => None,
        }
    }
}

mod imp {
    use std::cell::{Cell, OnceCell};

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gdk, glib, gsk};

    /// Private state of [`super::GlyphPaintable`].
    #[derive(Debug, Default)]
    pub(crate) struct GlyphPaintable {
        /// The glyph's stroke path in the 24-unit viewBox.
        pub(super) path: OnceCell<gsk::Path>,
        /// Stroke width in viewBox units.
        pub(super) stroke_width: Cell<f32>,
        /// Edge in logical pixels.
        pub(super) size: Cell<i32>,
        /// A fixed colour, or `None` to follow the CSS colour.
        pub(super) color: Cell<Option<gdk::RGBA>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GlyphPaintable {
        const NAME: &'static str = "OxGlyphPaintable";
        type Type = super::GlyphPaintable;
        type Interfaces = (gdk::Paintable, gtk::SymbolicPaintable);
    }

    impl ObjectImpl for GlyphPaintable {}

    impl PaintableImpl for GlyphPaintable {
        fn intrinsic_width(&self) -> i32 {
            self.size.get()
        }

        fn intrinsic_height(&self) -> i32 {
            self.size.get()
        }

        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE | gdk::PaintableFlags::STATIC_CONTENTS
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            self.draw(snapshot, width, height, super::FALLBACK_INK);
        }
    }

    impl SymbolicPaintableImpl for GlyphPaintable {
        fn snapshot_symbolic(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, colors: &[gdk::RGBA]) {
            let foreground = colors.first().copied().unwrap_or(super::FALLBACK_INK);
            self.draw(snapshot, width, height, foreground);
        }
    }

    impl GlyphPaintable {
        /// Strokes the path scaled from the 24-unit viewBox to the target
        /// size. A fixed colour (for Quick access glyphs) wins over CSS.
        fn draw(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, foreground: gdk::RGBA) {
            let (Some(path), Some(snapshot)) = (self.path.get(), snapshot.downcast_ref::<gtk::Snapshot>())
            else {
                return;
            };
            let color = self.color.get().unwrap_or(foreground);
            let stroke = gsk::Stroke::new(self.stroke_width.get());
            stroke.set_line_cap(gsk::LineCap::Round);
            stroke.set_line_join(gsk::LineJoin::Round);
            snapshot.save();
            #[expect(clippy::cast_possible_truncation, reason = "icon sizes fit an f32 exactly")]
            snapshot.scale(
                width as f32 / super::VIEWBOX_SIZE,
                height as f32 / super::VIEWBOX_SIZE,
            );
            snapshot.append_stroke(path, &stroke, &color);
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    /// A stroke glyph that paints in the current CSS colour.
    pub(crate) struct GlyphPaintable(ObjectSubclass<imp::GlyphPaintable>)
        @implements gdk::Paintable, gtk::SymbolicPaintable;
}

impl GlyphPaintable {
    /// `glyph` at `size` logical pixels. `color` fixes the colour instead of
    /// following CSS.
    ///
    /// # Panics
    ///
    /// Never for the glyphs of [`Glyph`]: every path in the table parses,
    /// which `every_glyph_parses_as_a_gsk_path` checks.
    pub(crate) fn new(glyph: Glyph, size: i32, color: Option<gdk::RGBA>) -> Self {
        let paintable: Self = glib::Object::new();
        let imp = paintable.imp();
        let path = gsk::Path::parse(glyph.path_data()).expect("glyph paths are valid SVG path data");
        imp.path.set(path).expect("a new paintable has no path yet");
        imp.stroke_width.set(glyph.stroke_width(size));
        imp.size.set(size);
        imp.color.set(color);
        paintable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph, in the order of the app.js table.
    const ALL_GLYPHS: [Glyph; 48] = [
        Glyph::Terminal,
        Glyph::Settings,
        Glyph::Plus,
        Glyph::Close,
        Glyph::Minus,
        Glyph::Maximize,
        Glyph::Restore,
        Glyph::Back,
        Glyph::Forward,
        Glyph::Up,
        Glyph::Refresh,
        Glyph::Down,
        Glyph::Chevron,
        Glyph::Search,
        Glyph::Home,
        Glyph::Desktop,
        Glyph::Downloads,
        Glyph::Documents,
        Glyph::Pictures,
        Glyph::Music,
        Glyph::Videos,
        Glyph::Pin,
        Glyph::Cut,
        Glyph::Copy,
        Glyph::Paste,
        Glyph::Rename,
        Glyph::Share,
        Glyph::Trash,
        Glyph::Sort,
        Glyph::Grid,
        Glyph::List,
        Glyph::Details,
        Glyph::More,
        Glyph::Network,
        Glyph::Server,
        Glyph::Drive,
        Glyph::Phone,
        Glyph::Check,
        Glyph::Shield,
        Glyph::Info,
        Glyph::Sun,
        Glyph::Moon,
        Glyph::Eject,
        Glyph::Clock,
        Glyph::Link,
        Glyph::Eye,
        Glyph::FolderLine,
        Glyph::Cancel,
    ];

    /// A stroke width in pixels for `glyph` drawn `size` pixels wide.
    fn stroke_pixels(glyph: Glyph, size: i32) -> f32 {
        let size_in_pixels = f32::from(u8::try_from(size).expect("a small icon"));
        glyph.stroke_width(size) * size_in_pixels / VIEWBOX_SIZE
    }

    /// parity: LOOK-015
    #[test]
    fn every_glyph_parses_as_a_gsk_path() {
        for glyph in ALL_GLYPHS {
            assert!(gsk::Path::parse(glyph.path_data()).is_ok(), "{glyph:?}");
        }
    }

    /// parity: LOOK-015
    #[test]
    fn the_more_dots_use_a_heavier_stroke() {
        assert!((Glyph::More.stroke_width(16) - 3.0).abs() < f32::EPSILON);
        assert!((Glyph::Copy.stroke_width(12) - 1.35).abs() < f32::EPSILON);
    }

    /// parity: LOOK-015
    #[test]
    fn glyphs_from_16_pixels_draw_at_least_a_one_pixel_line() {
        for size in [16, 17, 18, 24, 46] {
            assert!(
                stroke_pixels(Glyph::Copy, size) >= 1.0 - f32::EPSILON,
                "{size} pixels"
            );
        }
        assert!(
            (stroke_pixels(Glyph::Copy, 16) - 1.0).abs() < 1e-6,
            "16 pixels: exactly one"
        );
        assert!(
            stroke_pixels(Glyph::Copy, 12) < 0.7,
            "12 pixels keep the web app's line"
        );
    }

    /// parity: LOOK-015
    #[test]
    fn known_folder_icons_have_glyphs() {
        assert_eq!(Glyph::for_known_folder("downloads"), Some(Glyph::Downloads));
        assert_eq!(Glyph::for_known_folder("folder"), None);
    }
}
