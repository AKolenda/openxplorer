// SPDX-License-Identifier: AGPL-3.0-only
//! Icons: the approved Fluent icons, bundled with the app and shown by name.
//!
//! Replaces the icon helpers of `v2.0.0:desktop/ui/app.js` (`icon()`, `folderIcon`,
//! `zipFolderIcon`, `fileIcon` and `networkIcon`), which drew their own SVG.
//! The native app draws no icon: every picture is one of the upstream files
//! in `resources/icons/` (`SOURCES.md` says where each comes from), which
//! `build.rs` compiles into the binary as a `GResource` and [`register`] adds
//! to the display's icon theme. [`Icon`] names every file.
//!
//! A single icon is a `gtk::Image` showing it by name ([`image`],
//! [`set_icon`]), so GTK renders it sharp at any scale and paints a glyph
//! in the CSS `color` of its image. Pictures made of several icons (the zip
//! badge, the green network bar, the red cross of a disconnected share) are
//! an [`ArtImage`], which lays real icons and CSS-styled boxes over each
//! other.
//!
//! Every bundled icon a [`gtk::Image`] shows stays loaded for the life of
//! the process ([`keep_loaded`]). GTK 4.14's icon cache holds icons
//! without a reference and forgets one only when it is finalized, while
//! the thread that loads an icon's picture holds a reference of its own:
//! when that thread drops the last one, the main thread can find the
//! dying icon in the cache and take it again, which `GLib` reports as
//! `object_ref` on a finalized object. Keeping one reference to each icon
//! the app shows means its last reference is never dropped, on any
//! thread. The bundle is about a hundred small files, at the few sizes
//! the skin uses. The icons are kept at every scale the display's
//! monitors use, and looked up again when the icon theme changes or a
//! monitor is added or rescaled, since images then look their icons up
//! afresh without the app setting them again.
//!
//! Icons are hidden from assistive technology, as app.js marks every icon
//! `aria-hidden`: the button, row, tab or card around an icon names it.

mod art;
mod art_image;
mod composition;
mod emblem;
mod file_type;
mod icon;
mod tint;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Once;

use gtk::prelude::*;
use gtk::{gdk, glib};

pub(crate) use art::{Art, Connection, Storage};
pub(crate) use art_image::ArtImage;
pub(crate) use emblem::Emblems;
pub(crate) use file_type::is_zip;
pub(crate) use file_type::FileType;
pub(crate) use icon::Icon;
pub(crate) use tint::{stylesheet as tint_stylesheet, Tint};

/// Where the bundled icons live in the application's resources: the
/// prefix of `resources/icons/icons.gresource.xml`. Under it, the files
/// follow the `scalable/<context>/` layout that GTK's icon theme reads
/// from a resource path.
const RESOURCE_PATH: &str = "/io/winspace/Development/Native/icons";

/// The CSS class of every single-icon image, which tests use to find the
/// frame's glyphs.
const GLYPH_CLASS: &str = "glyph";

/// Registers the bundled icons once per process and adds them to the icon
/// theme of `display`, so every [`Icon`] resolves there. The desktop's icon
/// theme cannot replace one: every name starts with `ox-`, which no theme
/// uses.
///
/// # Panics
///
/// If the bundle compiled into the binary cannot be registered, which only
/// a corrupt build causes.
pub(crate) fn register(display: &gdk::Display) {
    static REGISTER_BUNDLE: Once = Once::new();
    REGISTER_BUNDLE.call_once(|| {
        gio::resources_register_include!("icons.gresource")
            .expect("build.rs compiles a valid icon bundle into the binary");
    });
    let theme = gtk::IconTheme::for_display(display);
    let is_added = theme.resource_path().iter().any(|path| path == RESOURCE_PATH);
    if !is_added {
        theme.add_resource_path(RESOURCE_PATH);
    }
}

/// A new image of `icon` at `size` logical pixels, centred in its
/// allocation so a larger one never stretches it.
pub(crate) fn image(icon: Icon, size: i32) -> gtk::Image {
    let image = decorative_image();
    image.add_css_class(GLYPH_CLASS);
    set_icon(&image, icon, size);
    image
}

/// An empty image, centred in its allocation and hidden from assistive
/// technology (see the module documentation).
fn decorative_image() -> gtk::Image {
    gtk::Image::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build()
}

/// Shows `icon` in `image` at `size` logical pixels.
pub(crate) fn set_icon(image: &gtk::Image, icon: Icon, size: i32) {
    image.set_icon_name(Some(icon.name()));
    image.set_pixel_size(size);
    keep_loaded(image, icon, size);
}

/// The scale a window can draw at before it knows its monitor's, on X11
/// and on Wayland compositors without buffer scales, so icons are kept at
/// it too. Every other scale is a monitor's: GTK rounds a fractional
/// scale up, as the monitor reports it. Keeping 2 and 3 everywhere would
/// load four and nine times the pixels of each icon on screens that never
/// use them.
const FIRST_SCALE: i32 = 1;

/// An icon as an image shows it, apart from the scale: its name, edge and
/// whether the text runs right to left.
type Shown = (&'static str, i32, bool);

/// What the icon theme caches an icon under: [`Shown`] and the scale.
type CacheKey = (&'static str, i32, bool, i32);

/// One reference to every icon shown on one display, at every scale its
/// monitors use and the [`FIRST_SCALE`].
#[derive(Default)]
struct KeptIcons {
    /// What the display's images show.
    shown: HashSet<Shown>,
    /// The scales the icons are kept at.
    scales: Vec<i32>,
    /// The icon theme's cached icon for each key.
    icons: HashMap<CacheKey, gtk::IconPaintable>,
}

impl KeptIcons {
    /// Looks `shown` up at `scale` as a `gtk::Image` does: a themed icon
    /// of its name, with [`gtk::IconLookupFlags::PRELOAD`]. A hit in the
    /// theme's cache, which a change of icon theme empties: the icon kept
    /// before is then replaced by the new theme's.
    fn keep(&mut self, theme: &gtk::IconTheme, shown: Shown, scale: i32) {
        let (name, size, is_rtl) = shown;
        let direction = if is_rtl {
            gtk::TextDirection::Rtl
        } else {
            gtk::TextDirection::Ltr
        };
        let themed = gio::ThemedIcon::new(name);
        let icon = theme.lookup_by_gicon(&themed, size, scale, direction, gtk::IconLookupFlags::PRELOAD);
        self.icons.insert((name, size, is_rtl, scale), icon);
    }

    /// Looks every shown icon up again at the display's current scales.
    fn refresh(&mut self, display: &gdk::Display) {
        let theme = gtk::IconTheme::for_display(display);
        self.scales = scales_of(display);
        let shown: Vec<Shown> = self.shown.iter().copied().collect();
        for shown in shown {
            for scale in self.scales.clone() {
                self.keep(&theme, shown, scale);
            }
        }
    }

    /// Keeps the icons of `display` up to date: after a change of icon
    /// theme, which empties GTK's cache before the images look their
    /// icons up again (the theme's `changed` signal comes first), and
    /// when a monitor is added or changes its scale.
    fn follow(display: &gdk::Display) {
        let theme = gtk::IconTheme::for_display(display);
        theme.connect_changed(glib::clone!(
            #[weak]
            display,
            move |_| refresh_kept(&display)
        ));
        let monitors = display.monitors();
        for monitor in monitors.iter::<gdk::Monitor>().filter_map(Result::ok) {
            follow_scale(display, &monitor);
        }
        monitors.connect_items_changed(glib::clone!(
            #[weak]
            display,
            move |monitors, position, _removed, added| {
                for index in position..position + added {
                    if let Some(monitor) = monitors.item(index).and_downcast::<gdk::Monitor>() {
                        follow_scale(&display, &monitor);
                    }
                }
                refresh_kept(&display);
            }
        ));
    }
}

/// Keeps the icons of `display` at `monitor`'s scale when it changes.
fn follow_scale(display: &gdk::Display, monitor: &gdk::Monitor) {
    monitor.connect_scale_factor_notify(glib::clone!(
        #[weak]
        display,
        move |_| refresh_kept(&display)
    ));
}

/// The scales to keep icons at on `display`: the [`FIRST_SCALE`] and
/// every monitor's.
fn scales_of(display: &gdk::Display) -> Vec<i32> {
    let mut scales: Vec<i32> = display
        .monitors()
        .iter::<gdk::Monitor>()
        .filter_map(Result::ok)
        .map(|monitor| monitor.scale_factor())
        .chain([FIRST_SCALE])
        .collect();
    scales.sort_unstable();
    scales.dedup();
    scales
}

thread_local! {
    /// The kept icons of each display, by its name.
    static KEPT: RefCell<HashMap<String, KeptIcons>> = RefCell::new(HashMap::new());
}

/// Looks the kept icons of `display` up again.
fn refresh_kept(display: &gdk::Display) {
    KEPT.with_borrow_mut(|kept| {
        if let Some(icons) = kept.get_mut(display.name().as_str()) {
            icons.refresh(display);
        }
    });
}

/// Keeps `icon` at `size` loaded for good, at every scale `image`'s
/// display may use, as `image` will look it up, so the icon theme's
/// cached icon is never finalized (see the module documentation). The
/// icon stays kept after a change of icon theme and when a monitor is
/// added or rescaled. Only the main thread calls it, as GTK does.
pub(crate) fn keep_loaded(image: &impl IsA<gtk::Widget>, icon: Icon, size: i32) {
    let display = image.as_ref().display();
    let is_rtl = image.as_ref().direction() == gtk::TextDirection::Rtl;
    let shown: Shown = (icon.name(), size, is_rtl);
    let theme = gtk::IconTheme::for_display(&display);
    let is_new_display = KEPT.with_borrow_mut(|kept| {
        let name = display.name().to_string();
        let is_new = !kept.contains_key(&name);
        let icons = kept.entry(name).or_insert_with(|| KeptIcons {
            scales: scales_of(&display),
            ..KeptIcons::default()
        });
        icons.shown.insert(shown);
        // Looked up again on every call, cheap hits in the theme's cache:
        // a lookup can empty the cache itself, when it finds the theme's
        // folders changed, before the theme's `changed` signal comes.
        for scale in icons.scales.clone() {
            icons.keep(&theme, shown, scale);
        }
        is_new
    });
    if is_new_display {
        KeptIcons::follow(&display);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gtk::gsk;

    use super::*;
    use crate::icons::icon::ALL_ICONS;

    /// The edge every icon is painted at to check that it loads.
    const PAINTED_SIZE: i32 = 32;

    /// The tests' private display.
    fn display() -> gdk::Display {
        gdk::Display::default().expect("GTK tests run on a private display")
    }

    /// The icon theme of the tests' private display, with the bundle added.
    fn registered_theme() -> gtk::IconTheme {
        let display = display();
        register(&display);
        gtk::IconTheme::for_display(&display)
    }

    /// `icon` as the icon theme finds it for `size` pixels.
    fn look_up(theme: &gtk::IconTheme, icon: Icon, size: i32) -> gtk::IconPaintable {
        theme.lookup_icon(
            icon.name(),
            &[],
            size,
            1,
            gtk::TextDirection::Ltr,
            gtk::IconLookupFlags::empty(),
        )
    }

    /// The resource an icon name resolves to at 16 pixels, as a URI.
    fn resolved_uri(theme: &gtk::IconTheme, icon: Icon) -> Option<String> {
        let file = look_up(theme, icon, 16).file()?;
        Some(file.uri().to_string())
    }

    /// A renderer that paints off screen. GSK requires a renderer to be
    /// unrealized before it is dropped, so the guard does that, also when
    /// an assertion fails.
    struct OffscreenRenderer(gsk::CairoRenderer);

    impl OffscreenRenderer {
        fn new() -> Self {
            let renderer = gsk::CairoRenderer::new();
            renderer
                .realize_for_display(&display())
                .expect("the Cairo renderer needs no GPU");
            Self(renderer)
        }

        /// What `paintable` looks like `size` pixels square.
        fn paint(&self, paintable: &impl IsA<gdk::Paintable>, size: i32) -> Option<gdk::Texture> {
            let snapshot = gtk::Snapshot::new();
            let edge = f64::from(size);
            paintable.snapshot(&snapshot, edge, edge);
            let node = snapshot.to_node()?;
            Some(self.0.render_texture(node, None))
        }
    }

    impl Drop for OffscreenRenderer {
        fn drop(&mut self) {
            self.0.unrealize();
        }
    }

    /// How many pixels of `texture` any ink covers.
    fn inked_pixels(texture: &gdk::Texture) -> usize {
        let downloader = gdk::TextureDownloader::new(texture);
        let (bytes, _stride) = downloader.download_bytes();
        let (pixels, _) = bytes.as_chunks::<4>();
        // GDK's default memory format keeps alpha in the fourth byte.
        pixels.iter().filter(|pixel| pixel[3] > 0).count()
    }

    /// An icon an image shows is never finalized, however many other
    /// icons pass through the icon theme's cache of recent ones (100 in
    /// GTK 4.14): its cached icon stays the one the theme hands out, so
    /// GTK's loading thread can never drop its last reference.
    #[gtk::test]
    fn a_shown_icon_stays_loaded_after_many_others_pass_through_the_cache() {
        let theme = registered_theme();
        let image = image(Icon::FileFolder, 16);
        let direction = image.direction();
        let looked_up = || {
            theme.lookup_by_gicon(
                &gio::ThemedIcon::new(Icon::FileFolder.name()),
                16,
                1,
                direction,
                gtk::IconLookupFlags::PRELOAD,
            )
        };
        let first = looked_up();
        let cached = first.downgrade();
        drop(first);
        drop(image);

        // Far more distinct lookups than the cache of recent icons holds,
        // and time for GTK's loading threads to finish and let go.
        for icon in ALL_ICONS {
            for size in [10, 11, 12, 13] {
                drop(look_up(&theme, icon, size));
            }
        }
        for _ in 0..20 {
            crate::test_support::harness::wait_for(std::time::Duration::from_millis(10));
        }

        let kept = cached.upgrade().expect("the icon is still loaded");
        assert_eq!(looked_up(), kept, "the theme hands out the kept icon");
    }

    /// Pushes far more distinct icons through the theme's cache of
    /// recent ones than it holds, and gives GTK's loading threads time to
    /// finish and let go.
    fn flood_the_cache(theme: &gtk::IconTheme) {
        for icon in ALL_ICONS {
            for size in [10, 11, 12, 13] {
                drop(look_up(theme, icon, size));
            }
        }
        for _ in 0..20 {
            crate::test_support::harness::wait_for(std::time::Duration::from_millis(10));
        }
    }

    /// The folder icon at 16 pixels and `scale`, as a `gtk::Image` showing
    /// it looks it up.
    fn shown_folder(theme: &gtk::IconTheme, scale: i32) -> gtk::IconPaintable {
        theme.lookup_by_gicon(
            &gio::ThemedIcon::new(Icon::FileFolder.name()),
            16,
            scale,
            gtk::Widget::default_direction(),
            gtk::IconLookupFlags::PRELOAD,
        )
    }

    /// A change of icon theme empties GTK's cache, and the images on
    /// screen then look their icons up afresh, without the app setting
    /// them again: the fresh icons are kept too.
    #[gtk::test]
    fn a_shown_icon_stays_loaded_after_the_icon_theme_changes() {
        let theme = registered_theme();
        let image = image(Icon::FileFolder, 16);
        // The same folders again: GTK treats it as a change of theme and
        // empties its cache, and nothing else changes for later tests.
        let search_path = theme.search_path();
        theme.set_search_path(&search_path.iter().map(PathBuf::as_path).collect::<Vec<_>>());
        crate::test_support::harness::settle();

        let fresh = shown_folder(&theme, 1);
        let cached = fresh.downgrade();
        drop(fresh);
        flood_the_cache(&theme);

        let kept = cached.upgrade().expect("the fresh icon is still loaded");
        assert_eq!(shown_folder(&theme, 1), kept, "the theme hands out the kept icon");
        drop(image);
    }

    #[gtk::test]
    fn every_icon_resolves_to_its_bundled_file() {
        let theme = registered_theme();
        for icon in ALL_ICONS {
            assert!(theme.has_icon(icon.name()), "{icon:?}");
            let uri = resolved_uri(&theme, icon).unwrap_or_default();
            let expected_prefix = format!("resource://{RESOURCE_PATH}/scalable/");
            assert!(uri.starts_with(&expected_prefix), "{icon:?} resolves to {uri:?}");
            assert!(uri.ends_with(&format!("/{}.svg", icon.name())), "{icon:?}: {uri}");
        }
    }

    /// Every file loads and paints: GTK would show its own image-missing
    /// picture instead of a file it cannot read (a broken gradient, say),
    /// and names the paintable after that picture.
    ///
    /// parity: LOOK-015
    #[gtk::test]
    fn every_icon_loads_and_paints_something() {
        let theme = registered_theme();
        let renderer = OffscreenRenderer::new();
        for icon in ALL_ICONS {
            let paintable = look_up(&theme, icon, PAINTED_SIZE);
            let texture = renderer.paint(&paintable, PAINTED_SIZE);
            let inked = texture.as_ref().map_or(0, inked_pixels);
            assert!(inked > 0, "{icon:?} paints nothing");
            let loaded_name = paintable.icon_name();
            assert_eq!(
                loaded_name,
                Some(PathBuf::from(icon.name())),
                "{icon:?} did not load"
            );
            let is_symbolic = icon.name().ends_with("-symbolic");
            assert_eq!(paintable.is_symbolic(), is_symbolic, "{icon:?}");
        }
    }

    #[gtk::test]
    fn registering_twice_adds_the_icons_once() {
        let theme = registered_theme();
        register(&display());
        let bundles = theme
            .resource_path()
            .iter()
            .filter(|path| *path == RESOURCE_PATH)
            .count();
        assert_eq!(bundles, 1);
    }

    #[gtk::test]
    fn an_image_shows_its_icon_by_name_at_its_size() {
        let image = image(Icon::ArrowLeft, 16);
        assert_eq!(image.icon_name().as_deref(), Some("ox-arrow-left-20-symbolic"));
        assert_eq!(image.pixel_size(), 16);
        assert!(image.has_css_class(GLYPH_CLASS));
        assert_eq!(image.accessible_role(), gtk::AccessibleRole::Presentation);
        set_icon(&image, Icon::ArrowRight, 12);
        assert_eq!(image.icon_name().as_deref(), Some("ox-arrow-right-20-symbolic"));
        assert_eq!(image.pixel_size(), 12);
    }
}
