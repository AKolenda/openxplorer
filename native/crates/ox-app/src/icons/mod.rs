// SPDX-License-Identifier: AGPL-3.0-only
//! Icons: the approved Fluent icons, bundled with the app and shown by name.
//!
//! Replaces the icon helpers of `desktop/ui/app.js` (`icon()`, `folderIcon`,
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

mod art;
mod art_image;
mod composition;
mod file_type;
mod icon;
mod tint;

use std::sync::Once;

use gtk::gdk;

pub(crate) use art::{Art, Connection, Storage};
pub(crate) use art_image::ArtImage;
#[cfg(test)]
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
    let image = gtk::Image::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes([GLYPH_CLASS])
        .build();
    set_icon(&image, icon, size);
    image
}

/// Shows `icon` in `image` at `size` logical pixels.
pub(crate) fn set_icon(image: &gtk::Image, icon: Icon, size: i32) {
    image.set_icon_name(Some(icon.name()));
    image.set_pixel_size(size);
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;

    use super::*;
    use crate::icons::icon::ALL_ICONS;

    /// The icon theme of the tests' private display, with the bundle added.
    fn registered_theme() -> gtk::IconTheme {
        let display = gdk::Display::default().expect("GTK tests run on a private display");
        register(&display);
        gtk::IconTheme::for_display(&display)
    }

    /// The resource an icon name resolves to at 16 pixels, as a URI.
    fn resolved_uri(theme: &gtk::IconTheme, icon: Icon) -> Option<String> {
        let paintable = theme.lookup_icon(
            icon.name(),
            &[],
            16,
            1,
            gtk::TextDirection::Ltr,
            gtk::IconLookupFlags::empty(),
        );
        let file = paintable.file()?;
        Some(file.uri().to_string())
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

    #[gtk::test]
    fn registering_twice_adds_the_icons_once() {
        let theme = registered_theme();
        let display = gdk::Display::default().expect("GTK tests run on a private display");
        register(&display);
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
        set_icon(&image, Icon::ArrowRight, 12);
        assert_eq!(image.icon_name().as_deref(), Some("ox-arrow-right-20-symbolic"));
        assert_eq!(image.pixel_size(), 12);
    }
}
