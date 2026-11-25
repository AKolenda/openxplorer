// SPDX-License-Identifier: AGPL-3.0-only
//! Glyphs from the Windows symbol font the user chose (LOOK-031).
//!
//! Windows 11 File Explorer draws its command bar, menus, navigation
//! buttons and chevrons with the Segoe Fluent Icons font. When Windows
//! icon files are on and the user chose `SegoeIcons.ttf` (or Segoe MDL2
//! Assets, `segmdl2.ttf`), [`follow_preferences`] has the glyphs the app
//! uses read off the main thread ([`ox_core::windows_glyphs`]) and written
//! as symbolic icons into the user's cache, one per bundled icon
//! ([`glyph_of`] names its codepoint, from Microsoft's published table of
//! Segoe Fluent Icons). That folder joins the icon theme's search path,
//! and [`drawn_name`] makes every [`super::set_icon`] show the glyph's
//! icon instead of the bundled one, so GTK draws it at the same size and
//! in the same CSS colour, following the theme. The chevrons the skin
//! draws in CSS follow through an override stylesheet. A glyph the font
//! lacks keeps the bundled icon, and turning the font off brings every
//! bundled icon back at once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::settings::Preferences;
use ox_core::windows_glyphs::{GlyphRequest, LoadedGlyphs};

use super::icon::{ALL_ICONS, COMMAND_ROLES};
use super::Icon;

/// A glyph of Segoe Fluent Icons: its codepoint and the name Microsoft's
/// table gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowsGlyph {
    /// Its codepoint, in the Private Use Area.
    pub(crate) codepoint: u32,
    /// Its name in Microsoft's table.
    pub(crate) name: &'static str,
}

/// Where a bundled icon's Windows counterpart comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsSource {
    /// A glyph of the symbol font.
    Glyph(WindowsGlyph),
    /// Not a glyph: a colour picture the Windows icon files draw by role
    /// (LOOK-030), or that only the app draws.
    Picture,
    /// No glyph of Windows matches; the reason.
    None(&'static str),
}

/// A glyph named `name` at `codepoint`.
const fn glyph(codepoint: u32, name: &'static str) -> WindowsSource {
    WindowsSource::Glyph(WindowsGlyph { codepoint, name })
}

/// Why an icon kept from an earlier design has no glyph.
const UNUSED: &str = "kept from an earlier design; not drawn";

/// Where `icon`'s Windows counterpart comes from: the Segoe Fluent Icons
/// glyph Windows 11 draws in the same place, by Microsoft's published
/// codepoints. An exhaustive table, hence its length.
#[expect(clippy::too_many_lines, reason = "one line per bundled icon")]
pub(crate) const fn windows_source(icon: Icon) -> WindowsSource {
    match icon {
        Icon::Add => glyph(0xE710, "Add"),
        Icon::Apps => glyph(0xE71D, "AllApps"),
        Icon::AppsListDetail => glyph(0xECA5, "Tiles"),
        Icon::ArrowClockwise => glyph(0xE72C, "Refresh"),
        Icon::ArrowCounterclockwise => glyph(0xE777, "UpdateRestore"),
        Icon::ArrowDown => glyph(0xE74B, "Down"),
        Icon::ArrowEject => glyph(0xF847, "Eject"),
        Icon::ArrowDownload => glyph(0xE896, "Download"),
        Icon::ArrowLeft => glyph(0xE72B, "Back"),
        Icon::ArrowRedo => glyph(0xE7A6, "Redo"),
        Icon::ArrowRight => glyph(0xE72A, "Forward"),
        Icon::ArrowSort => glyph(0xE8CB, "Sort"),
        // The command bar's rotate commands (Set as background is with
        // Image). Windows 11 draws them as two-tone pictures, not font
        // glyphs; these are the nearest glyphs of Microsoft's table, drawn
        // in the bar's colour as Cut's is.
        Icon::RotateLeft => glyph(0xE80D, "RotateMapLeft"),
        Icon::RotateRight => glyph(0xE80C, "RotateMapRight"),
        Icon::ArrowUndo => glyph(0xE7A7, "Undo"),
        Icon::ArrowUp => glyph(0xE74A, "Up"),
        Icon::Bluetooth => glyph(0xE702, "Bluetooth"),
        Icon::CheckboxChecked => glyph(0xE73A, "CheckboxComposite"),
        Icon::Checkmark => glyph(0xE73E, "CheckMark"),
        // The chevrons are `WinUI`'s, drawn at its 8-pixel font size
        // ([`em_of`]), as a drop-down button, a breadcrumb or a submenu
        // draws them.
        Icon::ChevronDown | Icon::ChevronDown16 => glyph(0xE70D, "ChevronDown"),
        Icon::ChevronLeft16 => glyph(0xE76B, "ChevronLeft"),
        Icon::ChevronRight16 => glyph(0xE76C, "ChevronRight"),
        Icon::ChevronUp16 => glyph(0xE70E, "ChevronUp"),
        Icon::CircleSmallFilled => glyph(0xE915, "RadioBullet"),
        Icon::ClipboardPaste => glyph(0xE77F, "Paste"),
        Icon::Clock => glyph(0xE917, "Clock"),
        Icon::Cloud => glyph(0xE753, "Cloud"),
        Icon::Copy => glyph(0xE8C8, "Copy"),
        Icon::Cut => glyph(0xE8C6, "Cut"),
        Icon::Delete => glyph(0xE74D, "Delete"),
        Icon::Desktop => glyph(0xE7F4, "TVMonitor"),
        // The thin X of a tab's Close and a menu's "Close tab"; a caption
        // or dialog button draws `ChromeClose` instead ([`glyph_at`]).
        Icon::Dismiss | Icon::Dismiss16 => glyph(0xE711, "Cancel"),
        Icon::Document => glyph(0xE8A5, "Document"),
        Icon::Eye => glyph(0xE890, "View"),
        Icon::Folder => glyph(0xE8B7, "Folder"),
        Icon::FolderAdd => glyph(0xE8F4, "NewFolder"),
        Icon::FolderZip => glyph(0xF012, "ZipFolder"),
        Icon::Grid => glyph(0xE8A9, "ViewAll"),
        Icon::HardDrive => glyph(0xEDA2, "HardDrive"),
        Icon::History => glyph(0xE81C, "History"),
        Icon::Home => glyph(0xE80F, "Home"),
        // Set as background takes the same Picture glyph.
        Icon::Image | Icon::SetAsBackground => glyph(0xE8B9, "Picture"),
        Icon::Info => glyph(0xE946, "Info"),
        Icon::Laptop => glyph(0xE7F8, "DeviceLaptopNoPic"),
        Icon::Link => glyph(0xE71B, "Link"),
        Icon::MapNetworkDrive24 => glyph(0xE8CE, "MapDrive"),
        Icon::Maximize => glyph(0xE922, "ChromeMaximize"),
        Icon::MoreHorizontal => glyph(0xE712, "More"),
        Icon::MusicNote => glyph(0xE90B, "MusicInfo"),
        Icon::Open => glyph(0xE8E5, "OpenFile"),
        Icon::Organization => glyph(0xE968, "Network"),
        Icon::PaintBrush => glyph(0xE771, "Personalize"),
        Icon::PanelLeft => glyph(0xE90C, "DockLeft"),
        Icon::PanelRight => glyph(0xE90D, "DockRight"),
        Icon::PanelRightGallery => glyph(0xE8A1, "PreviewLink"),
        Icon::Pause16 => glyph(0xE769, "Pause"),
        Icon::Phone => glyph(0xE8EA, "CellPhone"),
        Icon::Play16 => glyph(0xE768, "Play"),
        Icon::Pin => glyph(0xE718, "Pin"),
        Icon::Rename => glyph(0xE8AC, "Rename"),
        Icon::Search => glyph(0xE721, "Search"),
        Icon::SelectAll24 => glyph(0xE8B3, "SelectAll"),
        Icon::SelectNone24 => glyph(0xE8E6, "ClearSelection"),
        Icon::Settings => glyph(0xE713, "Settings"),
        Icon::Share => glyph(0xE72D, "Share"),
        Icon::ShieldLock => glyph(0xEA18, "Shield"),
        Icon::SquareMultiple => glyph(0xE923, "ChromeRestore"),
        Icon::Star => glyph(0xE734, "FavoriteStar"),
        Icon::StarFilled => glyph(0xE735, "FavoriteStarFill"),
        Icon::Subtract => glyph(0xE921, "ChromeMinimize"),
        Icon::TextBulletList => glyph(0xE8FD, "BulletedList"),
        Icon::TextColumnTwo => glyph(0xEA37, "List"),
        Icon::Video => glyph(0xE714, "Video"),
        Icon::WeatherMoon => glyph(0xE708, "QuietHours"),
        Icon::WeatherSunny => glyph(0xE706, "Brightness"),
        Icon::WindowConsole => glyph(0xE756, "CommandPrompt"),
        Icon::WindowMultiple => glyph(0xE7C4, "TaskView"),
        Icon::WindowNew => glyph(0xE78B, "NewWindow"),
        Icon::Wrench => glyph(0xE90F, "Repair"),
        Icon::AddNetworkLocation24
        | Icon::DisconnectNetworkDrive24
        | Icon::InvertSelection24
        | Icon::Options24 => WindowsSource::None(
            "an original glyph matching Windows 11's; Microsoft's table names none for it",
        ),
        Icon::ArrowAutofitHeight | Icon::ContentView | Icon::RectangleLandscape | Icon::SearchSettings => {
            WindowsSource::None("Microsoft's table names no glyph for this command")
        }
        Icon::ArrowReset
        | Icon::DeleteDismiss
        | Icon::Server
        | Icon::ChevronDoubleLeft16
        | Icon::ChevronDoubleRight16 => WindowsSource::None(
            "no glyph of this name in Microsoft's table (Reset, EmptyRecycleBin, Server, double chevrons)",
        ),
        Icon::DocumentAdd | Icon::DocumentCopy | Icon::DocumentText => {
            WindowsSource::None("Windows shows the file type's picture here, not a glyph")
        }
        Icon::DismissCircleFilled => WindowsSource::None("a coloured status badge, not a command glyph"),
        Icon::ArrowSwap
        | Icon::Braces
        | Icon::Markdown
        | Icon::SelectAllOff
        | Icon::SelectAllOn
        | Icon::Table => WindowsSource::None(UNUSED),
        Icon::CodeColor20
        | Icon::CodeColor24
        | Icon::DismissCircleColor32
        | Icon::DocumentColor20
        | Icon::DocumentColor32
        | Icon::DocumentColor48
        | Icon::DocumentTextColor20
        | Icon::DocumentTextColor32
        | Icon::DocumentTextColor48
        | Icon::FileFolder
        | Icon::HeadphonesColor20
        | Icon::HeadphonesColor32
        | Icon::HeadphonesColor48
        | Icon::ImageColor20
        | Icon::ImageColor32
        | Icon::ImageColor48
        | Icon::MailColor20
        | Icon::TableColor20
        | Icon::TableColor32
        | Icon::TableColor48
        | Icon::VideoColor20
        | Icon::VideoColor32
        | Icon::VideoColor48
        | Icon::WarningColor32 => WindowsSource::Picture,
    }
}

/// The glyph of `icon`, when Windows draws one in its place.
pub(crate) const fn glyph_of(icon: Icon) -> Option<WindowsGlyph> {
    match windows_source(icon) {
        WindowsSource::Glyph(glyph) => Some(glyph),
        WindowsSource::Picture | WindowsSource::None(_) => None,
    }
}

/// The em Windows 11 draws a glyph at in a square of `size` pixels, as
/// its controls set the font size: the square itself up to 16 pixels
/// (the 16-pixel command bar, navigation buttons, address bar and menu
/// rows, the 12-pixel tab close button), 16 pixels in the app's 17 to 24
/// pixel squares, and the square itself above that, where the user chose
/// larger icons.
pub(crate) const fn windows_em(size: i32) -> i32 {
    if size >= 17 && size <= LARGEST_PADDED {
        16
    } else {
        size
    }
}

/// The em Windows 11 draws `icon`'s glyph at in a square of `size`
/// pixels, as measured on Windows 11's own screens at 1x: chevrons at
/// 12 pixels in a 16-pixel square (a submenu's arrow, a heading's or a
/// tree's expander), 10 in a 12-pixel square (a breadcrumb separator)
/// and `WinUI`'s 8 elsewhere (a drop-down button's caret, a column's sort
/// arrow); the caption buttons' glyphs at 10 in their 15-pixel squares;
/// the search box's magnifier and the new-tab button's plus at 12;
/// others as [`windows_em`] has it. Never larger than the square.
pub(crate) const fn em_of(icon: Icon, size: i32) -> i32 {
    // Each em gives the ink Windows 11 draws there, from the real Segoe
    // Fluent Icons' ink at every em (measured on the user's PC), whose
    // glyphs ink less than their em, or all of it.
    let em = match icon {
        // The command bar's drop-down caret, 6 by 4, whatever its square.
        Icon::ChevronDown => CHEVRON_EM,
        // A drop-down list's, the address bar's history and a sort
        // indicator's chevron, as WinUI's ComboBox draws its 12-pixel one.
        Icon::ChevronDown16 | Icon::ChevronUp16 if size == 12 => 12,
        Icon::ChevronDown16
        | Icon::ChevronLeft16
        | Icon::ChevronRight16
        | Icon::ChevronUp16
        | Icon::ChevronDoubleLeft16
        | Icon::ChevronDoubleRight16 => match size {
            // A submenu's arrow, 5 by 10.
            16 => 13,
            // A breadcrumb separator, 5 by 8, and Windows' column sort
            // chevron, 8 by 5 (7 by 5 in Windows).
            12 | 13 => 11,
            _ => CHEVRON_EM,
        },
        // Caption buttons: the Chrome glyphs ink their em, 10.
        Icon::Subtract | Icon::Maximize | Icon::SquareMultiple | Icon::Dismiss if size <= 15 => CAPTION_EM,
        // A Quick access row's pin inks its em, Windows' 10 by 10.
        Icon::Pin if size == 14 => 10,
        // The search box's magnifier, 11 by 11.
        Icon::Search if size == 15 => 13,
        // Back, Forward, Up and Refresh beside the address bar: the
        // arrows ink their whole em, Windows' 12 by 12.
        Icon::ArrowLeft | Icon::ArrowRight | Icon::ArrowUp | Icon::ArrowClockwise if size == 16 => 12,
        // The new-tab plus (11 by 11 at 14, its square) and the tab's
        // close X (9 by 9 at 13, its square) fill their squares.
        _ => windows_em(size),
    };
    if em < size {
        em
    } else {
        size
    }
}

/// `WinUI`'s font size for the chevrons of its drop-down buttons.
const CHEVRON_EM: i32 = 8;

/// The font size of the caption buttons' glyphs.
const CAPTION_EM: i32 = 10;

/// The glyph of `icon` in a square of `size` pixels: a dismiss X in a
/// square of 15 pixels or less is a caption or dialog button, which
/// Windows draws as `ChromeClose`; elsewhere it is the thin Cancel, as a
/// menu's "Close tab" is. The command bar's New, in its 18-pixel square,
/// is Windows' circled plus, `AddTo`; the new-tab button's plus stays
/// plain.
const fn glyph_at(icon: Icon, size: i32) -> Option<WindowsGlyph> {
    match icon {
        Icon::Add if size >= 17 => Some(WindowsGlyph {
            codepoint: 0xECC8,
            name: "AddTo",
        }),
        Icon::Dismiss if size <= 15 => Some(WindowsGlyph {
            codepoint: 0xE8BB,
            name: "ChromeClose",
        }),
        _ => glyph_of(icon),
    }
}

/// The largest square a glyph is drawn into at a smaller em.
const LARGEST_PADDED: i32 = 24;

/// The squares the font's glyphs are made for at an em smaller than the
/// square: those the app's glyphs are drawn in.
const PADDED_SQUARES: [i32; 16] = [9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24];

/// The name of the icon the cache draws `icon`'s glyph as in a square of
/// `size` pixels: `ox-win-add-20-symbolic` for `ox-add-20-symbolic` where
/// the em fills the square, and `ox-win-add-20-box20-symbolic` where the
/// em is smaller ([`em_of`]).
fn glyph_icon_name(icon: Icon, size: i32) -> String {
    if em_of(icon, size) == size {
        return full_glyph_name(icon);
    }
    let name = glyph_key(icon);
    let stem = name.strip_prefix("ox-").unwrap_or(name);
    let stem = stem.strip_suffix("-symbolic").unwrap_or(stem);
    format!("ox-win-{stem}-box{size}-symbolic")
}

/// The name of `icon`'s glyph drawn at an em filling its square, which
/// any square can show: `ox-win-add-20-symbolic` for `ox-add-20-symbolic`.
fn full_glyph_name(icon: Icon) -> String {
    let name = glyph_key(icon);
    let stem = name.strip_prefix("ox-").unwrap_or(name);
    format!("ox-win-{stem}")
}

/// The name `icon`'s glyph is kept under: its own file's name, or for a
/// command role that draws another icon's file ([`COMMAND_ROLES`]), a
/// name of its own, so its glyph never replaces that icon's.
const fn glyph_key(icon: Icon) -> &'static str {
    match icon {
        Icon::RotateLeft => "ox-rotate-left-20-symbolic",
        Icon::RotateRight => "ox-rotate-right-20-symbolic",
        Icon::SetAsBackground => "ox-set-as-background-20-symbolic",
        _ => icon.name(),
    }
}

/// Every icon a glyph may stand for: the bundled icons and the command
/// roles.
fn glyph_icons() -> impl Iterator<Item = Icon> {
    ALL_ICONS.into_iter().chain(COMMAND_ROLES)
}

/// The bundled icon a drawn icon name stands for: itself, or the icon a
/// glyph's name was made from, in any square.
fn icon_named(name: &str) -> Option<Icon> {
    let bundled = name.strip_prefix("ox-win-").map(|rest| {
        let rest = match rest.rsplit_once("-box") {
            Some((stem, square))
                if square
                    .trim_end_matches("-symbolic")
                    .bytes()
                    .all(|byte| byte.is_ascii_digit()) =>
            {
                format!("{stem}-symbolic")
            }
            _ => rest.to_owned(),
        };
        format!("ox-{rest}")
    });
    let name = bundled.as_deref().unwrap_or(name);
    glyph_icons().find(|icon| glyph_key(*icon) == name)
}

/// What the font is asked for: each glyph at an em filling its square,
/// and in each of [`PADDED_SQUARES`] where its em is smaller.
fn glyph_requests() -> Vec<GlyphRequest> {
    let mut requests = Vec::new();
    for icon in glyph_icons() {
        let Some(glyph) = glyph_of(icon) else {
            continue;
        };
        // The glyph filling its em, for squares where the em fills the
        // square, even when Windows' 16-pixel square holds a smaller em.
        requests.push(GlyphRequest {
            name: full_glyph_name(icon),
            codepoint: glyph.codepoint,
            em: 16,
            size: 16,
        });
        let padded = PADDED_SQUARES
            .into_iter()
            .filter(|size| em_of(icon, *size) < *size);
        for size in padded {
            let codepoint = match glyph_at(icon, size) {
                Some(glyph) => glyph.codepoint,
                None => glyph.codepoint,
            };
            requests.push(GlyphRequest {
                name: glyph_icon_name(icon, size),
                codepoint,
                em: em_of(icon, size).unsigned_abs(),
                size: size.unsigned_abs(),
            });
        }
    }
    requests.dedup_by(|a, b| a.name == b.name);
    requests
}

/// Where the glyphs of the chosen font stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    /// Off, or no font chosen: the app's own glyphs.
    Off,
    /// The chosen font is being read.
    Reading,
    /// The font was read and draws `found` of `wanted` glyphs.
    Ready {
        /// How many glyphs it draws.
        found: usize,
        /// How many the app asked for.
        wanted: usize,
    },
}

/// The glyphs of the chosen font, on the main thread.
struct State {
    /// The font to read while the setting is on; `None` while off.
    wanted: Option<PathBuf>,
    /// Counts every change of [`Self::wanted`].
    generation: u64,
    /// The glyphs the font gave.
    loaded: Option<LoadedGlyphs>,
    /// The cache folder added to the icon theme's search path.
    searched: Option<PathBuf>,
    /// The stylesheet that draws the skin's chevrons from the font.
    stylesheet: Option<gtk::CssProvider>,
    /// The glyph icon names handed out, made once per icon and square.
    names: HashMap<(&'static str, i32), &'static str>,
    status: Status,
    /// Called with the status whenever it changes, until they return false.
    followers: Vec<Box<dyn Fn(Status) -> bool>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        wanted: None,
        generation: 0,
        loaded: None,
        searched: None,
        stylesheet: None,
        names: HashMap::new(),
        status: Status::Off,
        followers: Vec::new(),
    });
}

/// The CSS priority of the chevrons' stylesheet: above every layer of
/// the skin.
const STYLESHEET_PRIORITY: u32 = gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 5;

/// The icon name `icon` is drawn by in a square of `size` pixels: its
/// glyph from the chosen font, at the em Windows draws it at there
/// ([`windows_em`]), when the font has one, else its own.
pub(crate) fn drawn_name(icon: Icon, size: i32) -> &'static str {
    let own = icon.name();
    if glyph_of(icon).is_none() {
        return own;
    }
    // A square with a padded icon draws it; any other draws the glyph at
    // an em filling it, as an icon the user enlarged.
    // 0 stands for the glyph filling its em.
    let square = if PADDED_SQUARES.contains(&size) && em_of(icon, size) < size {
        size
    } else {
        0
    };
    STATE.with_borrow_mut(|state| {
        let name = if square == 0 {
            full_glyph_name(icon)
        } else {
            glyph_icon_name(icon, square)
        };
        let has_glyph = state.loaded.as_ref().is_some_and(|loaded| loaded.contains(&name));
        if !has_glyph {
            return own;
        }
        // At most one name per bundled icon and square is ever made, so
        // keeping them for the life of the process is bounded.
        state
            .names
            .entry((glyph_key(icon), square))
            .or_insert_with(|| Box::leak(name.into_boxed_str()))
    })
}

/// Reads the chosen font when Windows icon files are on and a font is
/// chosen, or goes back to the app's own glyphs. Called at startup and
/// whenever the settings change; nothing happens when the choice is the
/// same.
pub(crate) fn follow_preferences(preferences: &Preferences) {
    let wanted = preferences
        .use_icon_files
        .then(|| preferences.symbol_font.as_ref().map(PathBuf::from))
        .flatten();
    let generation = STATE.with_borrow_mut(|state| {
        if state.wanted == wanted {
            return None;
        }
        state.wanted.clone_from(&wanted);
        state.generation += 1;
        state.loaded = None;
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
    apply();
    let Some(font) = wanted else {
        return;
    };
    let glyphs = glyph_requests();
    let cache = ox_core::windows_glyphs::glyph_cache_folder();
    glib::spawn_future_local(async move {
        let read =
            gio::spawn_blocking(move || ox_core::windows_glyphs::load_glyphs(&font, &glyphs, &cache)).await;
        let loaded = read.unwrap_or_default();
        let current = STATE.with_borrow_mut(|state| {
            if state.generation != generation {
                return false;
            }
            let found = glyph_icons()
                .filter(|icon| glyph_of(*icon).is_some() && loaded.contains(&glyph_icon_name(*icon, 16)))
                .count();
            state.status = Status::Ready {
                found,
                wanted: glyph_count(),
            };
            state.loaded = Some(loaded);
            true
        });
        if current {
            apply();
        }
    });
}

/// How many bundled icons have a glyph of the font.
pub(crate) fn glyph_count() -> usize {
    glyph_icons().filter(|icon| glyph_of(*icon).is_some()).count()
}

/// Where the glyphs of the chosen font stand now.
pub(crate) fn status() -> Status {
    STATE.with_borrow(|state| state.status)
}

/// Calls `follower` with the status now and whenever it changes, until it
/// returns false.
pub(crate) fn follow_status(follower: impl Fn(Status) -> bool + 'static) {
    if follower(status()) {
        STATE.with_borrow_mut(|state| state.followers.push(Box::new(follower)));
    }
}

/// Tells the status followers, dropping those that are gone.
fn tell_followers() {
    let (followers, status) =
        STATE.with_borrow_mut(|state| (std::mem::take(&mut state.followers), state.status));
    let kept: Vec<Box<dyn Fn(Status) -> bool>> = followers
        .into_iter()
        .filter(|follower| follower(status))
        .collect();
    STATE.with_borrow_mut(|state| {
        let added = std::mem::replace(&mut state.followers, kept);
        state.followers.extend(added);
    });
}

/// Makes the display draw what the state holds: the icon theme searches
/// the glyphs' folder, the chevrons' stylesheet draws theirs, and every
/// image on screen shows its icon again.
fn apply() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&display);
    let (old, new, stylesheet) = STATE.with_borrow_mut(|state| {
        let new = state
            .loaded
            .as_ref()
            .and_then(LoadedGlyphs::folder)
            .map(std::path::Path::to_path_buf);
        (state.searched.take(), new, state.stylesheet.take())
    });
    let mut paths = theme.search_path();
    if let Some(old) = &old {
        paths.retain(|path| path != old);
    }
    if let Some(new) = &new {
        paths.push(new.clone());
    }
    if old != new {
        theme.set_search_path(&paths.iter().map(PathBuf::as_path).collect::<Vec<_>>());
    }
    if let Some(stylesheet) = stylesheet {
        gtk::style_context_remove_provider_for_display(&display, &stylesheet);
    }
    let css = chevron_stylesheet(crate::theme::skin_rules(), drawn_name);
    let stylesheet = (!css.is_empty()).then(|| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&css);
        gtk::style_context_add_provider_for_display(&display, &provider, STYLESHEET_PRIORITY);
        provider
    });
    STATE.with_borrow_mut(|state| {
        state.searched = new;
        state.stylesheet = stylesheet;
    });
    redraw_images();
    tell_followers();
}

/// The rules of `rules` that draw a bundled icon through
/// `-gtk-icontheme`, with only those declarations, the icon names
/// replaced as `drawn` draws them in the rule's square: its
/// `background-size`, else its `min-width`, else the square of the
/// longest earlier selector its own starts with (a `:dir(rtl)` or
/// `:checked` variant of a rule), else 16 pixels. Empty when `drawn`
/// replaces none.
pub(crate) fn chevron_stylesheet(rules: &str, drawn: impl Fn(Icon, i32) -> &'static str) -> String {
    let mut text = String::with_capacity(rules.len());
    let mut rest = rules;
    // Comments out, so a brace inside one cannot split a rule.
    while let Some(start) = rest.find("/*") {
        text.push_str(&rest[..start]);
        rest = rest[start..]
            .find("*/")
            .map_or("", |end| &rest[start + end + 2..]);
    }
    text.push_str(rest);
    let mut stylesheet = String::new();
    let mut squares: Vec<(String, i32)> = Vec::new();
    for rule in text.split('}') {
        let Some((selector, body)) = rule.split_once('{') else {
            continue;
        };
        let mut replaced = false;
        let own = css_pixels(body, "background-size").or_else(|| css_pixels(body, "min-width"));
        let base = selector
            .trim()
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let square = own.unwrap_or_else(|| {
            squares
                .iter()
                .filter(|(known, _)| base.starts_with(known.as_str()))
                .max_by_key(|(known, _)| known.len())
                .map_or(16, |(_, square)| *square)
        });
        if let Some(own) = own {
            for selector in selector.split(',') {
                let base = selector.trim().split(':').next().unwrap_or_default().trim();
                squares.push((base.to_owned(), own));
            }
        }
        let declarations: Vec<String> = body
            .split(';')
            .filter(|declaration| declaration.contains("-gtk-icontheme(\"ox-"))
            .map(|declaration| {
                let mut declaration = declaration.trim().to_owned();
                for icon in ALL_ICONS {
                    let quoted = format!("\"{}\"", icon.name());
                    let drawn_as = drawn(icon, square);
                    if drawn_as != icon.name() && declaration.contains(&quoted) {
                        declaration = declaration.replace(&quoted, &format!("\"{drawn_as}\""));
                        replaced = true;
                    }
                }
                declaration
            })
            .collect();
        if replaced {
            stylesheet.push_str(selector.trim());
            stylesheet.push_str(" { ");
            stylesheet.push_str(&declarations.join("; "));
            stylesheet.push_str("; }\n");
        }
    }
    stylesheet
}

/// The first pixel length the declaration `property` of `body` gives.
fn css_pixels(body: &str, property: &str) -> Option<i32> {
    let value = body.split(';').find_map(|declaration| {
        let (name, value) = declaration.split_once(':')?;
        (name.trim() == property).then_some(value)
    })?;
    value.split_whitespace().next()?.strip_suffix("px")?.parse().ok()
}

/// Shows every bundled glyph on screen again as [`drawn_name`] draws it
/// now: each image of every window whose icon is a bundled icon or a
/// glyph's.
fn redraw_images() {
    for window in gtk::Window::list_toplevels() {
        let mut pending = vec![window];
        while let Some(widget) = pending.pop() {
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                pending.push(current);
            }
            let Some(image) = widget.downcast_ref::<gtk::Image>() else {
                continue;
            };
            let Some(icon) = image.icon_name().as_deref().and_then(icon_named) else {
                continue;
            };
            if glyph_of(icon).is_some() {
                super::set_icon(image, icon, image.pixel_size());
            }
        }
    }
}

/// Waits until the chosen font was read, for tests.
#[cfg(test)]
pub(crate) fn wait_until_read() {
    crate::test_support::harness::wait_until("the symbol font to be read", || {
        !matches!(status(), Status::Reading)
    });
}

#[cfg(test)]
mod tests;
