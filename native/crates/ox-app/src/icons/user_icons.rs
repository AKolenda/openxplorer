// SPDX-License-Identifier: AGPL-3.0-only
//! Icons from the Windows icon files the user chose (LOOK-030).
//!
//! Off by default. Once "Icons from Windows icon files" is on in Settings
//! and files are chosen, [`follow_preferences`] has them read off the main
//! thread by [`ox_core::windows_icons::load_icons`], which keeps their
//! images as PNG files in the user's cache folder. Until that finishes,
//! and for every role the files do not draw, the app's own icons stay.
//!
//! An [`ArtImage`] asks [`texture_for`] before it composes its own icon:
//! the [`Art`]s that stand for a place or item a Windows file draws
//! ([`role_of`]) get the image whose edge best fits the icon's size at the
//! widget's scale, so the navigation pane draws the 16 or 20 pixel image,
//! the lists the 32 or 48 pixel one and the large views the 256 pixel
//! one, the smallest at least as large as the device pixels to fill. An
//! image that cannot be loaded is remembered as missing and the app's own
//! icon shown in its place. Every `ArtImage` and role image is redrawn
//! when the files are read, turned off or changed, and when its scale
//! changes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::places::KnownFolder;
use ox_core::settings::Preferences;
use ox_core::windows_icons::{IconRole, LoadedIcons};

use super::art::{NetworkArt, NetworkPlace};
use super::{Art, ArtImage, Connection, FileType, Icon, Medium, Tint};

/// Where the icons from the chosen files stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    /// Off, or on without any file: the app's own icons.
    Off,
    /// The chosen files are being read.
    Reading,
    /// The chosen files were read and draw `found` of the roles.
    Ready {
        /// How many of [`IconRole::ALL`] they draw.
        found: usize,
    },
}

/// A plain image showing a role's icon, or `fallback` when the files do
/// not draw it, such as a drive's tile in This PC or the address bar's
/// icon; no role shows `fallback` alone.
struct RoleImage {
    image: glib::WeakRef<gtk::Image>,
    role: Option<IconRole>,
    fallback: Icon,
    size: i32,
}

/// A follower of the [`Status`]; it returns false once it is gone.
type StatusFollower = Box<dyn Fn(Status) -> bool>;

/// The icons of the chosen files, on the main thread.
struct State {
    /// The files to read while the setting is on; `None` while off.
    wanted: Option<Vec<PathBuf>>,
    /// Counts every change of [`Self::wanted`], so a read that finishes
    /// after another change is dropped.
    generation: u64,
    /// The images the files gave.
    loaded: Option<Rc<LoadedIcons>>,
    /// The textures made so far, by cached file and edge in device
    /// pixels; `None` for one that could not be loaded.
    textures: HashMap<(PathBuf, u32), Option<gdk::Texture>>,
    /// Every art image made, to redraw.
    art_images: Vec<glib::WeakRef<ArtImage>>,
    /// Art images made since the dead ones were last dropped.
    added_since_pruning: usize,
    /// Every plain role image, to redraw.
    role_images: Vec<RoleImage>,
    status: Status,
    status_followers: Vec<StatusFollower>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            wanted: None,
            generation: 0,
            loaded: None,
            textures: HashMap::new(),
            art_images: Vec::new(),
            added_since_pruning: 0,
            role_images: Vec::new(),
            status: Status::Off,
            status_followers: Vec::new(),
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// Where the standard folders are, by location.
    static STANDARD_FOLDERS: RefCell<Vec<(String, KnownFolder)>> = const { RefCell::new(Vec::new()) };
}

/// The role `art` stands for, when a Windows icon file can draw it: the
/// folders, standard folders (as places, as listed items and on the
/// network), drives by their medium, the Recycle Bin empty (its plain
/// glyph) and full (its tinted one), Home, Gallery, This PC, Network and
/// the places in it, the Settings tab, and files by kind. A disconnected
/// share and every other glyph keep the app's own icons.
pub(crate) fn role_of(art: Art) -> Option<IconRole> {
    let role = match art {
        Art::Folder => IconRole::Folder,
        Art::StandardFolder(folder) => known_folder_role(folder).unwrap_or(IconRole::Folder),
        Art::ZipFolder | Art::File(FileType::Archive) => IconRole::ZipFolder,
        Art::File(file_type) => file_role(file_type),
        // Every fixed drive, Local Disk too, is the plain drive: no Windows
        // logo on a disk that is not Windows'.
        Art::Glyph(Icon::HardDrive) | Art::SystemDrive => IconRole::Drive,
        Art::RemovableDrive(Medium::Disk) => IconRole::RemovableDrive,
        Art::RemovableDrive(Medium::Optical) => IconRole::OpticalDrive,
        Art::Glyph(Icon::Delete) => IconRole::RecycleBinEmpty,
        Art::TintedGlyph(Icon::Delete, _) => IconRole::RecycleBinFull,
        Art::Glyph(Icon::Home) | Art::TintedGlyph(Icon::Home, _) => IconRole::Home,
        Art::TintedGlyph(Icon::Image, Tint::Home) => IconRole::Gallery,
        Art::Glyph(Icon::Laptop) | Art::TintedGlyph(Icon::Laptop, Tint::ThisPc) => IconRole::ThisPc,
        Art::Glyph(Icon::Organization) | Art::TintedGlyph(Icon::Organization, _) => IconRole::Network,
        Art::Glyph(Icon::Settings) => IconRole::Settings,
        Art::TintedGlyph(_, Tint::KnownFolder(folder)) => known_folder_role(folder)?,
        Art::Network(network) => network_role(network)?,
        // File icons' document with a zipper has no Windows picture.
        Art::ZipDocument | Art::Glyph(_) | Art::TintedGlyph(..) => return None,
    };
    Some(role)
}

/// The role of a file of `file_type`: its own for text, pictures, audio,
/// video, programs, scripts and shortcuts; the blank page Windows shows
/// for a file no installed app claims for every other kind.
const fn file_role(file_type: FileType) -> IconRole {
    match file_type {
        FileType::Text => IconRole::TextFile,
        FileType::Image => IconRole::ImageFile,
        FileType::Audio => IconRole::AudioFile,
        FileType::Video => IconRole::VideoFile,
        FileType::Executable => IconRole::Executable,
        FileType::Script => IconRole::Script,
        FileType::Shortcut => IconRole::Shortcut,
        FileType::Archive => IconRole::ZipFolder,
        FileType::Document | FileType::Pdf | FileType::Spreadsheet | FileType::Code => IconRole::GenericFile,
    }
}

/// The role of a network location: a server's computer, a share, a mapped
/// drive connected or not, and a standard folder moved onto the network
/// as that folder. A disconnected share keeps the app's red cross.
fn network_role(network: NetworkArt) -> Option<IconRole> {
    let connected = network.connection == Connection::Connected;
    match network.place {
        NetworkPlace::Server | NetworkPlace::Computer => Some(IconRole::NetworkComputer),
        NetworkPlace::Share => connected.then_some(IconRole::NetworkShare),
        NetworkPlace::MappedDrive if connected => Some(IconRole::NetworkDrive),
        NetworkPlace::MappedDrive => Some(IconRole::NetworkDriveDisconnected),
        NetworkPlace::KnownFolder(folder) => known_folder_role(folder),
    }
}

/// The role of a standard folder with an icon of its own.
const fn known_folder_role(folder: KnownFolder) -> Option<IconRole> {
    match folder {
        KnownFolder::Desktop => Some(IconRole::Desktop),
        KnownFolder::Downloads => Some(IconRole::Downloads),
        KnownFolder::Documents => Some(IconRole::Documents),
        KnownFolder::Pictures => Some(IconRole::Pictures),
        KnownFolder::Music => Some(IconRole::Music),
        KnownFolder::Videos => Some(IconRole::Videos),
        KnownFolder::Templates | KnownFolder::Public => None,
    }
}

/// Reads the chosen files when the setting is on and the files changed,
/// or goes back to the app's own icons when it is turned off. Called at
/// startup and whenever the settings change; nothing happens when the
/// choice is the same.
pub(crate) fn follow_preferences(preferences: &Preferences) {
    let wanted = (preferences.use_icon_files && !preferences.icon_files.is_empty()).then(|| {
        preferences
            .icon_files
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    });
    let generation = STATE.with_borrow_mut(|state| {
        if state.wanted == wanted {
            return None;
        }
        state.wanted.clone_from(&wanted);
        state.generation += 1;
        state.loaded = None;
        state.textures.clear();
        state.status = if wanted.is_some() {
            Status::Reading
        } else {
            Status::Off
        };
        Some(state.generation)
    });
    let Some(generation) = generation else {
        return;
    };
    redraw_all();
    let Some(files) = wanted else {
        return;
    };
    let cache = ox_core::windows_icons::cache_folder();
    glib::spawn_future_local(async move {
        let read = gio::spawn_blocking(move || ox_core::windows_icons::load_icons(&files, &cache)).await;
        let loaded = read.unwrap_or_default();
        use_loaded(generation, loaded);
    });
}

/// Takes the icons a read gave, unless the choice changed since.
fn use_loaded(generation: u64, loaded: LoadedIcons) {
    let current = STATE.with_borrow_mut(|state| {
        if state.generation != generation {
            return false;
        }
        state.status = Status::Ready {
            found: loaded.found(),
        };
        state.loaded = Some(Rc::new(loaded));
        state.textures.clear();
        true
    });
    if current {
        redraw_all();
    }
}

/// Where the icons from the chosen files stand now.
pub(crate) fn status() -> Status {
    STATE.with_borrow(|state| state.status)
}

/// Calls `follower` with the status now and whenever it changes, until it
/// returns false.
pub(crate) fn follow_status(follower: impl Fn(Status) -> bool + 'static) {
    if follower(status()) {
        STATE.with_borrow_mut(|state| state.status_followers.push(Box::new(follower)));
    }
}

/// The texture of `role`'s image that best fills `size` logical pixels at
/// `scale`: the smallest of the file's images at least that large,
/// resampled once to exactly `size` × `scale` device pixels, so it is
/// drawn pixel for pixel. A renderer scaling a larger bitmap itself (the
/// GPU renderers' linear filter) leaves a zoomed view's icons jagged and
/// soft. `None` when the files do not draw the role, while they are
/// read, or when the setting is off.
pub(crate) fn texture_of(role: IconRole, size: i32, scale: i32) -> Option<gdk::Texture> {
    let pixels = u32::try_from(size.max(1).saturating_mul(scale.max(1))).unwrap_or(1);
    STATE.with_borrow_mut(|state| {
        let image = state.loaded.as_ref()?.best(role, pixels)?;
        // A file without a large enough image keeps its largest as it is.
        let edge = pixels.min(image.edge);
        let path = image.path.clone();
        state
            .textures
            .entry((path, edge))
            .or_insert_with_key(|(path, edge)| match resampled(path, *edge) {
                Ok(texture) => Some(texture),
                Err(error) => {
                    glib::g_warning!(
                        ox_core::LOG_DOMAIN,
                        "A cached Windows icon could not be loaded, so the built-in one is shown: {error}"
                    );
                    None
                }
            })
            .clone()
    })
}

/// The PNG at `path`, resampled with gdk-pixbuf's area filter to `edge`
/// pixels square, or as it is when it already is that size.
fn resampled(path: &std::path::Path, edge: u32) -> Result<gdk::Texture, glib::Error> {
    let edge = i32::try_from(edge).unwrap_or(i32::MAX);
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_file(path)?;
    if pixbuf.width() == edge && pixbuf.height() == edge {
        return Ok(gdk::Texture::for_pixbuf(&pixbuf));
    }
    let scaled = pixbuf
        .scale_simple(edge, edge, gtk::gdk_pixbuf::InterpType::Hyper)
        .ok_or_else(|| glib::Error::new(glib::FileError::Nomem, "the icon could not be resampled"))?;
    Ok(gdk::Texture::for_pixbuf(&scaled))
}

/// The texture to show for `art` at `size` logical pixels and `scale`, as
/// [`texture_of`] gives it for the art's role.
pub(crate) fn texture_for(art: Art, size: i32, scale: i32) -> Option<gdk::Texture> {
    texture_of(role_of(art)?, size, scale)
}

/// Remembers `image`, so it is redrawn when the icons change.
pub(super) fn register(image: &ArtImage) {
    STATE.with_borrow_mut(|state| {
        state.added_since_pruning += 1;
        if state.added_since_pruning > state.art_images.len().max(64) {
            state.art_images.retain(|image| image.upgrade().is_some());
            state.added_since_pruning = 0;
        }
        state.art_images.push(image.downgrade());
    });
}

/// A new image showing `role`'s icon from the chosen files at `size`
/// logical pixels, or the app's `fallback` glyph, and following the
/// choice as an [`ArtImage`] does.
pub(crate) fn role_image(role: IconRole, fallback: Icon, size: i32) -> gtk::Image {
    let image = super::image(fallback, size);
    show_icon(&image, Some(role), fallback, size);
    image
}

/// Shows `role`'s icon from the chosen files in `image` at `size` logical
/// pixels, or `fallback` when there is no role or no picture for it, and
/// keeps doing so when the files or the image's scale change. Showing
/// another role in the same image replaces the first.
pub(crate) fn show_icon(image: &gtk::Image, role: Option<IconRole>, fallback: Icon, size: i32) {
    let is_new = STATE.with_borrow_mut(|state| {
        state.role_images.retain(|kept| kept.image.upgrade().is_some());
        let kept = state
            .role_images
            .iter_mut()
            .find(|kept| kept.image.upgrade().as_ref() == Some(image));
        if let Some(kept) = kept {
            kept.role = role;
            kept.fallback = fallback;
            kept.size = size;
            return false;
        }
        state.role_images.push(RoleImage {
            image: image.downgrade(),
            role,
            fallback,
            size,
        });
        true
    });
    if is_new {
        image.connect_scale_factor_notify(redraw_role_image);
    }
    show_role(image, role, fallback, size);
}

/// Draws `image` again with the role it was last given.
fn redraw_role_image(image: &gtk::Image) {
    let shown = STATE.with_borrow(|state| {
        state
            .role_images
            .iter()
            .find(|kept| kept.image.upgrade().as_ref() == Some(image))
            .map(|kept| (kept.role, kept.fallback, kept.size))
    });
    if let Some((role, fallback, size)) = shown {
        show_role(image, role, fallback, size);
    }
}

/// Shows `role` in `image`, or `fallback` when there is no texture for it.
fn show_role(image: &gtk::Image, role: Option<IconRole>, fallback: Icon, size: i32) {
    match role.and_then(|role| texture_of(role, size, image.scale_factor())) {
        Some(texture) => {
            image.set_paintable(Some(&texture));
            image.set_pixel_size(size);
        }
        None => super::set_icon(image, fallback, size),
    }
}

/// Remembers where the standard folders are, so a listed folder there is
/// [`Art::StandardFolder`]; the app context calls it whenever they move.
pub(crate) fn remember_standard_folders(places: &[ox_core::places::Place]) {
    let folders: Vec<(String, KnownFolder)> = places
        .iter()
        .filter_map(|place| Some((place.uri.clone(), place.known_folder?)))
        .filter(|(_, folder)| KnownFolder::QUICK_ACCESS.contains(folder))
        .collect();
    STANDARD_FOLDERS.with_borrow_mut(|known| *known = folders);
}

/// The standard folder at `uri`, if one is there.
pub(crate) fn standard_folder_at(uri: &str) -> Option<KnownFolder> {
    STANDARD_FOLDERS.with_borrow(|known| {
        known
            .iter()
            .find(|(place, _)| ox_core::location::same_location(place, uri))
            .map(|(_, folder)| *folder)
    })
}

/// Redraws every image after a change, and tells the status followers.
fn redraw_all() {
    // The same icons' other source, File icons (LOOK-029), counts the
    // change too, so folder views that follow its generation draw again.
    super::file_icons::count_new_generation();
    let (art_images, role_images, status) = STATE.with_borrow(|state| {
        let art_images: Vec<ArtImage> = state
            .art_images
            .iter()
            .filter_map(glib::WeakRef::upgrade)
            .collect();
        let role_images: Vec<(gtk::Image, Option<IconRole>, Icon, i32)> = state
            .role_images
            .iter()
            .filter_map(|kept| Some((kept.image.upgrade()?, kept.role, kept.fallback, kept.size)))
            .collect();
        (art_images, role_images, state.status)
    });
    for image in art_images {
        image.redraw();
    }
    for (image, role, fallback, size) in role_images {
        show_role(&image, role, fallback, size);
    }
    let followers = STATE.with_borrow_mut(|state| std::mem::take(&mut state.status_followers));
    let kept: Vec<StatusFollower> = followers
        .into_iter()
        .filter(|follower| follower(status))
        .collect();
    STATE.with_borrow_mut(|state| {
        let added = std::mem::replace(&mut state.status_followers, kept);
        state.status_followers.extend(added);
    });
}

/// Waits until the chosen files were read, for tests.
#[cfg(test)]
pub(crate) fn wait_until_read() {
    crate::test_support::harness::wait_until("the Windows icon files to be read", || {
        !matches!(status(), Status::Reading)
    });
}

#[cfg(test)]
mod tests;
