// SPDX-License-Identifier: AGPL-3.0-only
//! Stroke glyphs: the line icons of the command bar, sidebar and menus.
//!
//! The path data is the `paths` table at the top of `desktop/ui/app.js`
//! (24-unit viewBox, 1.35 stroke, round caps and joins; `more` uses a
//! 3-unit stroke so its dots are visible). [`GlyphPaintable`] draws a path
//! with `gsk::Path` and implements `GtkSymbolicPaintable`, so a `GtkImage`
//! paints it in the widget's CSS `color` and it follows hover, disabled and
//! dark styles without any image files.

use std::cell::{Cell, OnceCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, gsk};

/// Glyph names and their SVG path data, from `paths` in app.js, plus
/// `restore` for the maximised caption button.
pub const PATHS: &[(&str, &str)] = &[
    ("terminal", "M3 5h18v14H3zM6 9l3 3-3 3M12 15h5"),
    (
        "settings",
        "M10 2h4l1 3 3 1 3 2-2 3v2l2 3-3 2-3 1-1 3h-4l-1-3-3-1-3-2 2-3v-2L3 8l3-2 3-1zM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
    ),
    ("plus", "M12 5v14M5 12h14"),
    ("close", "m6 6 12 12M18 6 6 18"),
    ("minus", "M5 12h14"),
    ("maximize", "M5 5h14v14H5z"),
    ("restore", "M8 8h11v11H8zM5 16V5h11"),
    ("back", "m12 5-7 7 7 7M5 12h14"),
    ("forward", "m12 5 7 7-7 7M5 12h14"),
    ("up", "m5 12 7-7 7 7M12 5v14"),
    ("refresh", "M19 10a7 7 0 1 0-1 7M19 4v6h-6"),
    ("down", "m7 10 5 5 5-5"),
    ("chevron", "m9 6 6 6-6 6"),
    ("search", "M16 16l5 5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0"),
    ("home", "m3 11 9-8 9 8M5 10v10h5v-6h4v6h5V10"),
    ("desktop", "M3 4h18v13H3zM9 21h6M12 17v4"),
    ("downloads", "M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"),
    ("documents", "M6 3h9l4 4v14H6zM14 3v5h5M9 12h7M9 16h7"),
    ("pictures", "M3 4h18v16H3zM3 17l6-6 4 4 3-3 5 5M16 8h.01"),
    (
        "music",
        "M9 18V5l11-2v13M9 7l11-2M9 18c0 2-6 3-6 0s6-3 6 0Zm11-2c0 2-6 3-6 0s6-3 6 0Z",
    ),
    ("videos", "M4 4h16v16H4zM4 8h16M4 16h16M8 4v4M16 4v4M8 16v4M16 16v4"),
    ("pin", "m8 3 9 9M15 4l5 5-5 2-3 5-4-4-5-1 5-3 2-5M9 15l-6 6"),
    (
        "cut",
        "m9 9 10 12M9 15 19 3M9 7a3 3 0 1 1-6 0 3 3 0 0 1 6 0Zm0 10a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z",
    ),
    ("copy", "M8 8h12v13H8zM16 8V3H3v13h5"),
    ("paste", "M8 5H5v16h14V5h-3M9 3h6v4H9z"),
    ("rename", "M3 7h7v10H3zM16 3v18M13 3h6M13 21h6M20 7h2v10h-2"),
    ("share", "M14 4h7v7M21 4 10 15M10 5H4v16h16v-6"),
    ("trash", "M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7M14 10v7"),
    ("sort", "M7 3v18m-4-4 4 4 4-4M14 5h7M14 10h5M14 15h3"),
    ("grid", "M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z"),
    ("list", "M3 5h2M9 5h12M3 12h2M9 12h12M3 19h2M9 19h12"),
    ("details", "M3 4h18v16H3zM15 4v16M18 8h.01M18 12h.01M18 16h.01"),
    ("more", "M4 12h.01M12 12h.01M20 12h.01"),
    ("network", "M8 3h8v6H8zM3 16h6v5H3zM15 16h6v5h-6zM12 9v4M6 16v-3h12v3"),
    (
        "server",
        "M5 3h14v7H5zM5 14h14v7H5zM8 6.5h.01M8 17.5h.01M12 6.5h4M12 17.5h4",
    ),
    ("drive", "m5 5-3 11v5h20v-5L19 5zM2 16h20M17 18.5h.01M20 18.5h.01"),
    (
        "phone",
        "M8 2h8a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2zM10 5h4M11 19h2",
    ),
    ("check", "m5 12 4 4L19 6"),
    ("shield", "m12 2 8 3v6c0 5-8 11-8 11S4 16 4 11V5zM8 11l3 3 5-6"),
    ("info", "M12 11v6M12 7h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0"),
    (
        "sun",
        "M12 2v2M12 20v2M2 12h2M20 12h2m-3-7 1.5-1.5M5 19l-1.5 1.5m0-17L5 5m14 14 1.5 1.5M17 12a5 5 0 1 1-10 0 5 5 0 0 1 10 0",
    ),
    ("moon", "M20 15A9 9 0 0 1 9 4a9 9 0 1 0 11 11Z"),
    ("eject", "m5 14 7-10 7 10zM5 20h14"),
    ("clock", "M12 7v6l4 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0"),
    (
        "link",
        "m9 15 6-6M8 16l-1 1a4 4 0 0 1-6-6l5-5a4 4 0 0 1 6 0m0 2 1-1a4 4 0 0 1 6 6l-5 5a4 4 0 0 1-6 0",
    ),
    (
        "eye",
        "M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
    ),
    ("folderline", "M3 7V4h7l2 3h9v13H3z"),
    ("cancel", "M5 5l14 14M5 19 19 5"),
];

/// The glyph drawn for an unknown name, as `icon()` in app.js does.
const FALLBACK: &str = "documents";

/// Path data for `name`, falling back to the documents glyph.
pub fn path_data(name: &str) -> &'static str {
    let lookup = |wanted: &str| {
        PATHS
            .iter()
            .find(|(key, _)| *key == wanted)
            .map(|(_, data)| *data)
    };
    lookup(name)
        .or_else(|| lookup(FALLBACK))
        .expect("the fallback glyph is in the table")
}

/// Stroke width in viewBox units: 3 for the `more` dots, 1.35 otherwise.
pub fn stroke_width(name: &str) -> f32 {
    if name == "more" {
        3.0
    } else {
        1.35
    }
}

mod imp {
    use super::*;

    /// Private state of [`super::GlyphPaintable`].
    #[derive(Default)]
    pub struct GlyphPaintable {
        pub path: OnceCell<gsk::Path>,
        pub stroke_width: Cell<f32>,
        pub size: Cell<i32>,
        pub color: Cell<Option<gdk::RGBA>>,
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
            let fallback = gdk::RGBA::new(0.14, 0.14, 0.14, 1.0);
            self.draw(snapshot, width, height, fallback);
        }
    }

    impl SymbolicPaintableImpl for GlyphPaintable {
        fn snapshot_symbolic(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, colors: &[gdk::RGBA]) {
            let foreground = colors
                .first()
                .copied()
                .unwrap_or_else(|| gdk::RGBA::new(0.14, 0.14, 0.14, 1.0));
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
            snapshot.scale((width / 24.0) as f32, (height / 24.0) as f32);
            snapshot.append_stroke(path, &stroke, &color);
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    /// A stroke glyph that paints in the current CSS colour.
    pub struct GlyphPaintable(ObjectSubclass<imp::GlyphPaintable>)
        @implements gdk::Paintable, gtk::SymbolicPaintable;
}

impl GlyphPaintable {
    /// The glyph `name` at `size` logical pixels. `color` (CSS hex) fixes the
    /// colour instead of following CSS.
    pub fn new(name: &str, size: i32, color: Option<&str>) -> Self {
        let paintable: Self = glib::Object::new();
        let imp = paintable.imp();
        let path = gsk::Path::parse(path_data(name)).expect("glyph paths are valid SVG path data");
        imp.path.set(path).expect("a new paintable has no path yet");
        imp.stroke_width.set(stroke_width(name));
        imp.size.set(size);
        imp.color.set(color.and_then(|hex| gdk::RGBA::parse(hex).ok()));
        paintable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_parses_as_a_gsk_path() {
        for (name, data) in PATHS {
            assert!(gsk::Path::parse(data).is_ok(), "{name}");
        }
    }

    #[test]
    fn glyph_names_are_unique() {
        let mut names: Vec<&str> = PATHS.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PATHS.len());
    }

    #[test]
    fn unknown_names_draw_the_documents_glyph() {
        assert_eq!(path_data("no-such-glyph"), path_data("documents"));
        assert_eq!(stroke_width("more"), 3.0);
        assert_eq!(stroke_width("copy"), 1.35);
    }
}
