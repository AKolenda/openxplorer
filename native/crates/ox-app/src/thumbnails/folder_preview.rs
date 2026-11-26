// SPDX-License-Identifier: AGPL-3.0-only
//! Previews of a folder's contents on its icon (VIEW-058), as Windows 11
//! draws a folder at Medium icons and larger and in Tiles: the thumbnail
//! of its first picture or video by name peeks out of the folder, straight,
//! between its back and its front; a folder without any shows the icon of
//! its first file as a page; an empty folder, or one holding only folders,
//! stays plain, as in Windows.
//!
//! What a folder holds is read off the main thread, a few folders at a
//! time, after the folder's own icon is drawn, and kept by the folder's
//! modification time, which changes whenever an item is added, removed or
//! renamed in it; the picture's thumbnail comes from the shared cache
//! through [`super::preview`], so a folder visited again draws its
//! previews on the frame its tile is bound ([`kept_folder_art`]).
//!
//! [`layout`] places the pieces as Windows 11 does, measured on its own
//! screenshots (`reference/ref-11-view-xlarge.png` and
//! `ref-12-view-large.png` of the status branch): every measure is a
//! fraction of the folder's width or height.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio};
use ox_core::entry::Entry;

use super::{OnScreen, PreviewPolicy, PreviewRequest, Slots};
use crate::folder_view::sorting::{compare_names, SortKey, SortName};

/// The most items of a folder looked at for its first picture: a folder
/// of more shows the first of those.
const MOST_READ: usize = 20_000;
/// The most folders whose contents are kept.
const MOST_FOLDERS_KEPT: usize = 4_096;
/// How many folders are read at a time.
const READING_AT_ONCE: usize = 2;

/// An item of a folder that peeks out of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Peeking {
    /// Its name.
    pub name: String,
    /// Its URI.
    pub uri: String,
    /// Its content type, guessed from its name.
    pub content_type: String,
    /// Its modification time, in seconds.
    pub modified: Option<u64>,
    /// Its size in bytes.
    pub size: Option<u64>,
}

/// The items of a folder that may peek out of it: its first picture and
/// its first video by name, and its first file of any kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Contents {
    picture: Option<Peeking>,
    video: Option<Peeking>,
    first_file: Option<Peeking>,
}

/// What a folder's icon shows of its contents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Shown {
    /// The picture or video whose thumbnail peeks out.
    pub picture: Option<Peeking>,
    /// With no picture to show, the file whose icon peeks out as a page.
    pub page: Option<Peeking>,
}

impl Contents {
    /// What the folder's icon shows of these contents when pictures' and
    /// videos' previews are on as `pictures` and `videos` say: its first
    /// picture or video by name, else its first file.
    pub(crate) fn shown(&self, pictures: bool, videos: bool) -> Shown {
        self.shown_within(pictures, videos, None)
    }

    /// [`Self::shown`] with "Skip very large files" as `size_limit` says:
    /// a picture larger than it keeps its icon wherever it is listed, so it
    /// does not peek out of its folder either; the video, or the first
    /// file's page, shows instead. A video has no limit, as its thumbnailer
    /// reads one frame ([`super::PreviewPolicy::allows`]).
    pub(crate) fn shown_within(&self, pictures: bool, videos: bool, size_limit: Option<u64>) -> Shown {
        let within_limit = |item: &&Peeking| match (size_limit, item.size) {
            (Some(limit), Some(size)) => size <= limit,
            _ => true,
        };
        let picture = self
            .picture
            .iter()
            .filter(|_| pictures)
            .filter(within_limit)
            .chain(self.video.iter().filter(|_| videos))
            .min_by_key(|item| NameOrder::of(&item.name))
            .cloned();
        let page = if picture.is_none() {
            self.first_file.clone()
        } else {
            None
        };
        Shown { picture, page }
    }
}

/// A name in the order Windows lists a folder by name: natural, folded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NameOrder(SortKey, String);

impl NameOrder {
    fn of(name: &str) -> Self {
        Self(SortKey::new(name), name.to_owned())
    }
}

impl PartialOrd for NameOrder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NameOrder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        compare_names(
            SortName {
                key: &self.0,
                name: &self.1,
            },
            SortName {
                key: &other.0,
                name: &other.1,
            },
        )
    }
}

/// The first name offered, by name, with its content type.
#[derive(Debug, Default)]
struct FirstName(Option<(NameOrder, String)>);

impl FirstName {
    /// Keeps `name` if it comes before the one kept so far.
    fn offer(&mut self, name: &NameOrder, content_type: &str) {
        if self.0.as_ref().is_none_or(|(kept, _)| name < kept) {
            self.0 = Some((name.clone(), content_type.to_owned()));
        }
    }
}

/// Reads which of the items of the local `folder` may peek out of it: its
/// first picture and video by name and its first file, leaving out
/// hidden items and folders. Blocks; an unreadable folder holds nothing.
pub(crate) fn read_contents(folder: &Path) -> Contents {
    let Ok(items) = std::fs::read_dir(folder) else {
        return Contents::default();
    };
    let mut picture = FirstName::default();
    let mut video = FirstName::default();
    let mut file = FirstName::default();
    for item in items.take(MOST_READ).flatten() {
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let is_file = match item.file_type() {
            Ok(kind) if kind.is_symlink() => item.path().metadata().is_ok_and(|target| target.is_file()),
            Ok(kind) => kind.is_file(),
            Err(_) => false,
        };
        if !is_file {
            continue;
        }
        let order = NameOrder::of(name);
        let (content_type, _) = gio::content_type_guess(Some(name), None);
        file.offer(&order, &content_type);
        match content_type.split_once('/').map(|(kind, _)| kind) {
            Some("image") => picture.offer(&order, &content_type),
            Some("video") => video.offer(&order, &content_type),
            _ => {}
        }
    }
    let peeking = |first: FirstName| {
        let (NameOrder(_, name), content_type) = first.0?;
        peeking_at(folder, name, content_type)
    };
    Contents {
        picture: peeking(picture),
        video: peeking(video),
        first_file: peeking(file),
    }
}

/// The item `name` of `folder`, of `content_type`, with its time and size
/// read now; `None` once it is gone.
fn peeking_at(folder: &Path, name: String, content_type: String) -> Option<Peeking> {
    let path = folder.join(&name);
    let metadata = std::fs::metadata(&path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs());
    Some(Peeking {
        uri: gio::File::for_path(&path).uri().to_string(),
        name,
        content_type,
        modified,
        size: Some(metadata.len()),
    })
}

/// The items `known` names, each with its time and size read again, as a
/// picture may change without its folder changing. An item that is gone
/// is left out (its folder's time changed too, so the folder is read anew
/// next); one that could not be read just now, as a drive busy for a
/// moment cannot, keeps what was read of it last, rather than the folder
/// being kept as having no such item for as long as its time stays.
fn read_again(folder: &Path, known: &Contents) -> Contents {
    let again = |item: &Option<Peeking>| {
        let item = item.as_ref()?;
        match std::fs::metadata(folder.join(&item.name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => Some(item.clone()),
            Ok(_) => peeking_at(folder, item.name.clone(), item.content_type.clone())
                .or_else(|| Some(item.clone())),
        }
    };
    Contents {
        picture: again(&known.picture),
        video: again(&known.video),
        first_file: again(&known.first_file),
    }
}

/// A folder whose contents' previews are wanted on its icon.
#[derive(Debug, Clone)]
pub(crate) struct FolderRequest {
    /// The folder's URI.
    pub uri: String,
    /// Its modification time, which tells its current contents from old.
    pub modified: Option<i64>,
    /// The edge of its icon, in device pixels.
    pub pixels: i32,
    /// Which previews the user wants.
    pub policy: PreviewPolicy,
    /// Whether the folder's tile is on the screen now.
    pub on_screen: OnScreen,
}

impl FolderRequest {
    /// What the folder's icon shows of `contents`, with the previews the
    /// user wants: pictures and videos as on, none larger than "Skip very
    /// large files" lets through ([`Contents::shown_within`]).
    fn shown_of(&self, contents: &Contents) -> Shown {
        contents.shown_within(self.policy.pictures, self.policy.videos, self.policy.size_limit)
    }
}

/// What a folder's icon draws of its contents.
#[derive(Debug, Clone)]
pub(crate) struct FolderArt {
    /// The thumbnail that peeks out.
    pub picture: Option<gdk::Texture>,
    /// The file whose icon peeks out as a page, with no thumbnail shown.
    pub page: Option<Peeking>,
}

/// What a folder's icon is known to draw of its contents.
#[derive(Debug, Clone)]
pub(crate) enum KnownArt {
    /// Nothing: the folder stays plain.
    Plain,
    /// Its contents.
    Drawn(FolderArt),
}

impl PreviewPolicy {
    /// The request for the previews of `entry`'s contents on its icon of
    /// `pixels` device pixels: only for a local folder, while previews
    /// are shown ("Always show icons, never thumbnails" is off), and with
    /// network previews off only for one on this computer.
    pub(crate) fn folder_request(self, entry: &Entry, pixels: i32) -> Option<FolderRequest> {
        let reachable = self.remote || !(self.folder_on_network || super::is_remote_item(entry));
        let wanted = self.shown && entry.is_dir && !entry.is_virtual && entry.uri.starts_with("file:");
        (wanted && reachable).then(|| FolderRequest {
            uri: entry.uri.clone(),
            modified: entry.modified,
            pixels,
            policy: self,
            on_screen: OnScreen::default(),
        })
    }
}

/// How long the times and sizes of a folder's items read last are taken
/// as they are: a picture overwritten in place leaves its folder's time
/// as it was, so a folder whose art is drawn from memory has its items
/// read again once this has passed ([`contents_need_checking`]).
const CONTENTS_CHECKED_FOR: Duration = Duration::from_secs(2);

/// The contents of the folders read, by URI and modification time, with
/// when their items' times and sizes were read.
type KnownContents = HashMap<(String, i64), (Rc<Contents>, Instant)>;

thread_local! {
    /// The contents of the folders read.
    static KNOWN: RefCell<KnownContents> = RefCell::new(HashMap::new());
    static READING: Rc<Slots> = Rc::new(Slots::new(READING_AT_ONCE));
}

/// The contents of `request`'s folder read for its current time, if any,
/// and when its items were read.
fn known(request: &FolderRequest) -> Option<(Rc<Contents>, Instant)> {
    let key = (request.uri.clone(), request.modified?);
    KNOWN.with_borrow(|known| known.get(&key).cloned())
}

/// Keeps `contents` for `request`'s folder at its current time, its items
/// read now.
fn keep(request: &FolderRequest, contents: Contents) -> Rc<Contents> {
    let contents = Rc::new(contents);
    if let Some(modified) = request.modified {
        KNOWN.with_borrow_mut(|known| {
            if known.len() >= MOST_FOLDERS_KEPT {
                known.clear();
            }
            known.insert(
                (request.uri.clone(), modified),
                (Rc::clone(&contents), Instant::now()),
            );
        });
    }
    contents
}

/// Whether the art of `request`'s folder, drawn at once from what is
/// known ([`kept_folder_art`]), should still be read again
/// ([`folder_art`]): its items were read longer ago than
/// [`CONTENTS_CHECKED_FOR`], and the picture peeking out may have been
/// overwritten in place since, which leaves its folder's time as it was.
/// Otherwise the folder's icon would keep the old picture for as long as
/// its thumbnail stayed in memory.
pub(crate) fn contents_need_checking(request: &FolderRequest) -> bool {
    known(request).is_some_and(|(_, read)| read.elapsed() >= CONTENTS_CHECKED_FOR)
}

/// The request for the thumbnail of `item` peeking out of `request`'s
/// folder.
fn picture_request(request: &FolderRequest, item: &Peeking) -> PreviewRequest {
    PreviewRequest {
        uri: item.uri.clone(),
        content_type: item.content_type.clone(),
        modified: item.modified,
        size: item.size,
        pixels: request.pixels,
        local_only: !request.policy.remote,
        target_uri: None,
        on_network: request.policy.folder_on_network,
        on_screen: request.on_screen.clone(),
    }
}

/// What the icon of `request`'s folder draws of its contents, known
/// without waiting: its contents were read at its current time and every
/// thumbnail is kept in memory. `None` when it must be read
/// ([`folder_art`]).
pub(crate) fn kept_folder_art(request: &FolderRequest) -> Option<KnownArt> {
    let (contents, _) = known(request)?;
    let shown = request.shown_of(&contents);
    let Some(item) = shown.picture else {
        return Some(shown.page.map_or(KnownArt::Plain, |page| {
            KnownArt::Drawn(FolderArt {
                picture: None,
                page: Some(page),
            })
        }));
    };
    let picture = super::kept_preview(&picture_request(request, &item))?;
    Some(KnownArt::Drawn(FolderArt {
        picture: Some(picture),
        page: None,
    }))
}

/// What the icon of `request`'s folder draws of its contents: the
/// thumbnail of its first picture, else its first file as a page, else
/// nothing. The folder is read off the main thread, after the tiles on
/// the screen; its contents are kept by its time.
pub(crate) async fn folder_art(request: FolderRequest) -> Option<FolderArt> {
    let folder = gio::File::for_uri(&request.uri).path()?;
    if !request.policy.remote && super::is_on_network(&request.uri, None).await {
        return None;
    }
    let known = known(&request).map(|(known, _)| Contents::clone(&known));
    let reading = READING.with(Rc::clone);
    let on_screen = request.on_screen.clone();
    let contents = reading
        .blocking_in_turn(&move || on_screen.get(), move |_| match known {
            // The picture's time is read again: it may have changed
            // without its folder changing.
            Some(known) => read_again(&folder, &known),
            None => read_contents(&folder),
        })
        .await?;
    // Kept as read now, so the art drawn at once next time is the
    // picture's current version.
    let contents = keep(&request, contents);
    let shown = request.shown_of(&contents);
    if let Some(item) = &shown.picture {
        if let Some(picture) = super::preview(picture_request(&request, item)).await {
            return Some(FolderArt {
                picture: Some(picture),
                page: None,
            });
        }
    }
    // A picture without a thumbnail leaves the first file's page.
    let page = shown.page.or_else(|| contents.shown(false, false).page)?;
    Some(FolderArt {
        picture: None,
        page: Some(page),
    })
}

/// The pieces a folder with contents is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pieces {
    /// The app's own folder, drawn in pieces.
    BuiltIn,
    /// The back and front from the user's Windows icon files, which draw
    /// the folder as Windows does in its icon's square.
    Windows,
}

/// A rectangle, in pixels of the icon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// The rectangle's right edge.
    pub(crate) fn right(&self) -> f32 {
        self.x + self.width
    }

    /// The rectangle's bottom edge.
    pub(crate) fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

/// Where the pieces of a folder with contents go on its icon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Layout {
    /// The folder, back and front together.
    pub folder: Rect,
    /// The sheet inside the back that the contents lie on, from its top
    /// down to the folder's bottom; the front hides its lower part.
    pub sheet: Rect,
    /// The part of the sheet the front leaves, which the picture covers.
    pub window: Rect,
    /// The front's top: lower on the left, up to `step_start`, then up a
    /// slope to `step_end` (under the back's tab's slope) and level to the
    /// right edge.
    pub front_left_top: f32,
    pub front_right_top: f32,
    pub step_start: f32,
    pub step_end: f32,
    /// The square the page's icon is drawn in.
    pub page: Rect,
}

/// The measures of Windows 11's folder with contents, each a fraction of
/// the folder's width (`x`) or height (`y`), measured on the reference
/// screenshots at Large and Extra large icons by
/// `native/tools/measure-folder-previews.py`.
mod measured {
    /// The folder's width and height, as fractions of the icon's edge.
    pub(super) const FOLDER_WIDTH: f32 = 0.875;
    pub(super) const FOLDER_HEIGHT: f32 = 0.6875;
    /// The sheet's sides and top.
    pub(super) const SHEET_LEFT_X: f32 = 0.03;
    pub(super) const SHEET_RIGHT_X: f32 = 0.96;
    pub(super) const SHEET_TOP_Y: f32 = 0.182;
    /// The front's top on the left and on the right, and the slope
    /// between, which lies under the tab's.
    pub(super) const FRONT_LEFT_TOP_Y: f32 = 0.602;
    pub(super) const FRONT_RIGHT_TOP_Y: f32 = 0.534;
    pub(super) const STEP_START_X: f32 = 0.357;
    pub(super) const STEP_END_X: f32 = 0.464;
    /// The page peeking out: its sides and top.
    pub(super) const PAGE_LEFT_X: f32 = 0.31;
    pub(super) const PAGE_RIGHT_X: f32 = 0.69;
    pub(super) const PAGE_TOP_Y: f32 = 0.25;
}

/// The app's own folder in its 32-unit square (`ox-file-folder-flat.svg`):
/// left, top, right and bottom.
const BUILT_IN_FOLDER: (f32, f32, f32, f32) = (2.0, 4.0, 30.0, 30.0);
/// The page's paper in the app's file icons, in a 32-unit square: its
/// sides and top, which the page's place is found from (the 48-unit
/// document's paper spans x 8 to 40 from y 4).
const PAGE_PAPER: (f32, f32, f32) = (16.0 / 3.0, 80.0 / 3.0, 8.0 / 3.0);
/// Where the pieces of a folder with contents go on an icon `edge` pixels
/// square drawn with `pieces`.
pub(crate) fn layout(edge: f32, pieces: Pieces) -> Layout {
    use measured::{
        FOLDER_HEIGHT, FOLDER_WIDTH, FRONT_LEFT_TOP_Y, FRONT_RIGHT_TOP_Y, PAGE_LEFT_X, PAGE_RIGHT_X,
        PAGE_TOP_Y, SHEET_LEFT_X, SHEET_RIGHT_X, SHEET_TOP_Y, STEP_END_X, STEP_START_X,
    };
    let unit = edge / 32.0;
    let folder = match pieces {
        Pieces::BuiltIn => {
            let (left, top, right, bottom) = BUILT_IN_FOLDER;
            Rect {
                x: left * unit,
                y: top * unit,
                width: (right - left) * unit,
                height: (bottom - top) * unit,
            }
        }
        Pieces::Windows => {
            let (width, height) = (edge * FOLDER_WIDTH, edge * FOLDER_HEIGHT);
            Rect {
                x: (edge - width) / 2.0,
                y: (edge - height) / 2.0,
                width,
                height,
            }
        }
    };
    let x = |fraction: f32| folder.x + folder.width * fraction;
    let y = |fraction: f32| folder.y + folder.height * fraction;
    let sheet = Rect {
        x: x(SHEET_LEFT_X),
        y: y(SHEET_TOP_Y),
        width: x(SHEET_RIGHT_X) - x(SHEET_LEFT_X),
        height: folder.bottom() - y(SHEET_TOP_Y),
    };
    // Down to the front's lower, left part, so the picture reaches it all
    // along.
    let window = Rect {
        height: y(FRONT_LEFT_TOP_Y) - sheet.y,
        ..sheet
    };
    // The page's paper spans the measured sides.
    let (paper_left, paper_right, paper_top) = PAGE_PAPER;
    let page_edge = (x(PAGE_RIGHT_X) - x(PAGE_LEFT_X)) * 32.0 / (paper_right - paper_left);
    let page = Rect {
        x: folder.x + folder.width / 2.0 - page_edge / 2.0,
        y: y(PAGE_TOP_Y) - paper_top * page_edge / 32.0,
        width: page_edge,
        height: page_edge,
    };
    Layout {
        folder,
        sheet,
        window,
        front_left_top: y(FRONT_LEFT_TOP_Y),
        front_right_top: y(FRONT_RIGHT_TOP_Y),
        step_start: x(STEP_START_X),
        step_end: x(STEP_END_X),
        page,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
    use gtk::glib;

    use super::*;
    use ox_core::settings::ViewOptions;

    /// A picture of 300 × 200 pixels of `color`, at `name` in `folder`.
    fn picture_at(folder: &Path, name: &str, color: u32) -> PathBuf {
        let path = folder.join(name);
        let pixels = Pixbuf::new(Colorspace::Rgb, false, 8, 300, 200).expect("a picture");
        pixels.fill(color);
        pixels.savev(&path, "png", &[]).expect("the picture");
        path
    }

    fn name(item: Option<&Peeking>) -> Option<&str> {
        item.map(|item| item.name.as_str())
    }

    fn policy() -> PreviewPolicy {
        PreviewPolicy::for_folder(&ViewOptions::default(), false)
    }

    /// A folder's first picture or video by name peeks out of it, in
    /// Windows' natural order (Harbor 2 before Harbor 10), leaving out
    /// hidden files, other files and folders; with video previews off,
    /// videos do not.
    ///
    /// parity: VIEW-058
    #[gtk::test]
    fn a_folders_first_picture_by_name_peeks_out() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        for name in ["Harbor 10.png", "Harbor 2.png", ".Cover.png", "Zoo.png"] {
            picture_at(folder.path(), name, 0x3366_99ff);
        }
        std::fs::write(folder.path().join("Agenda.txt"), "notes").expect("a file");
        std::fs::write(folder.path().join("Arrival.mp4"), b"not really a video").expect("a video");
        std::fs::create_dir(folder.path().join("A folder.png")).expect("a folder");

        let contents = read_contents(folder.path());

        let shown = contents.shown(true, true);
        assert_eq!(name(shown.picture.as_ref()), Some("Arrival.mp4"));
        assert_eq!(shown.page, None, "no page with a picture shown");
        let without_videos = contents.shown(true, false);
        assert_eq!(name(without_videos.picture.as_ref()), Some("Harbor 2.png"));
        let picture = without_videos.picture.expect("a picture");
        assert!(picture.uri.starts_with("file:///") && picture.uri.ends_with("Harbor%202.png"));
        assert_eq!(picture.content_type, "image/png");
        assert!(picture.modified.is_some() && picture.size.is_some_and(|size| size > 0));

        // "Skip very large files": a picture larger than the limit keeps its
        // icon, so it does not peek out either, and the video does.
        let video_size = shown.picture.and_then(|video| video.size).expect("a size");
        let small = contents.shown_within(true, true, Some(video_size));
        assert_eq!(
            name(small.picture.as_ref()),
            Some("Arrival.mp4"),
            "the small video still peeks out"
        );
        let large_video = contents.shown_within(true, true, Some(1));
        assert_eq!(
            name(large_video.picture.as_ref()),
            Some("Arrival.mp4"),
            "a video over the limit still peeks out"
        );
        let too_large = contents.shown_within(true, false, Some(video_size));
        assert_eq!(too_large.picture, None, "the larger pictures keep their icons");
        assert_eq!(
            too_large.page.map(|page| page.name).as_deref(),
            Some("Agenda.txt"),
            "the first file's page instead"
        );
    }

    /// Reading a folder's items again keeps one that could not be read for
    /// a moment as it was last read, so its art is not lost for as long as
    /// the folder's time stays; one that is gone is left out.
    ///
    /// parity: VIEW-058
    #[gtk::test]
    fn an_item_that_cannot_be_read_for_a_moment_keeps_its_place() {
        use std::os::unix::fs::PermissionsExt;

        let folder = tempfile::tempdir().expect("a temporary folder");
        picture_at(folder.path(), "Harbor.png", 0x3366_99ff);
        let known = read_contents(folder.path());
        assert_eq!(name(known.picture.as_ref()), Some("Harbor.png"));

        // A folder that cannot be searched just now: its items cannot be
        // stated, as a busy drive's cannot.
        std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o600)).expect("mode");
        let unreadable = std::fs::metadata(folder.path().join("Harbor.png")).is_err();
        let again = read_again(folder.path(), &known);
        std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o755)).expect("mode");
        if unreadable {
            assert_eq!(again.picture, known.picture, "kept as read last");
        }

        std::fs::remove_file(folder.path().join("Harbor.png")).expect("removed");
        assert_eq!(
            read_again(folder.path(), &known).picture,
            None,
            "a gone item goes"
        );
    }

    /// A folder without pictures shows its first file as a page; an empty
    /// folder, or one holding only folders and hidden files, stays plain.
    ///
    /// parity: VIEW-058
    #[gtk::test]
    fn a_folder_without_pictures_shows_its_first_file_and_an_empty_one_stays_plain() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        assert_eq!(read_contents(folder.path()).shown(true, true), Shown::default());
        std::fs::create_dir(folder.path().join("Drafts")).expect("a folder");
        std::fs::write(folder.path().join(".hidden"), "x").expect("a hidden file");
        assert_eq!(read_contents(folder.path()).shown(true, true), Shown::default());

        std::fs::write(folder.path().join("Report final.docx"), "x").expect("a file");
        std::fs::write(folder.path().join("Meeting notes.txt"), "x").expect("a file");
        let shown = read_contents(folder.path()).shown(true, true);

        assert!(shown.picture.is_none());
        assert_eq!(
            shown.page.map(|page| page.name).as_deref(),
            Some("Meeting notes.txt")
        );
        // With picture previews off, a folder of pictures shows a page.
        picture_at(folder.path(), "Beach.png", 0xff00_00ff);
        let shown = read_contents(folder.path()).shown(false, false);
        assert_eq!(shown.page.map(|page| page.name).as_deref(), Some("Beach.png"));
    }

    /// "Always show icons, never thumbnails" turns the folders' previews
    /// off, as in Windows; they are only for local folders and, with
    /// network thumbnails off, not for a folder on a network.
    ///
    /// parity: VIEW-058
    #[test]
    fn always_showing_icons_turns_folder_previews_off() {
        let mut folder = crate::test_support::folder_entry("Photos");
        folder.uri = "file:///home/demo/Photos".to_owned();
        assert!(policy().folder_request(&folder, 96).is_some());

        let icons_only = ViewOptions {
            show_previews: false,
            ..ViewOptions::default()
        };
        assert!(PreviewPolicy::for_folder(&icons_only, false)
            .folder_request(&folder, 96)
            .is_none());
        let network_icons = ViewOptions {
            remote_previews: false,
            ..ViewOptions::default()
        };
        assert!(PreviewPolicy::for_folder(&network_icons, true)
            .folder_request(&folder, 96)
            .is_none());
        let file = crate::test_support::file_entry("Photos.png");
        assert!(
            policy().folder_request(&file, 96).is_none(),
            "a file is no folder"
        );
        let mut share = folder.clone();
        share.uri = "smb://studio-nas/photos".to_owned();
        assert!(policy().folder_request(&share, 96).is_none());
    }

    /// The pieces sit where Windows 11 draws them at Large icons: the
    /// folder 84 × 66 pixels in the 96-pixel square, the pictures from 18 %
    /// of its height down to the front, which rises from 60 % on the left
    /// to 53 % on the right, as `ref-12-view-large.png` measures them.
    ///
    /// parity: VIEW-058
    #[test]
    fn the_pieces_are_laid_out_as_windows_draws_them() {
        let large = layout(96.0, Pieces::Windows);
        let near = |found: f32, measured: f32| (found - measured).abs() <= 1.0;
        // Measured on ref-12: the folder spans x 294..377 and y 149..214 of
        // its tile, whose 96-pixel square starts at 288 and 134.
        assert!(near(large.folder.width, 83.0), "{large:?}");
        assert!(near(large.folder.height, 65.0), "{large:?}");
        assert!(near(large.folder.x, 294.0 - 288.0) && near(large.folder.y, 149.0 - 134.0));
        assert!(near(large.sheet.y, 161.0 - 134.0), "the sheet's top: {large:?}");
        assert!(near(large.front_left_top, 189.0 - 134.0), "{large:?}");
        assert!(near(large.front_right_top, 184.0 - 134.0), "{large:?}");
        assert!(near(large.sheet.x, 296.0 - 288.0) && near(large.sheet.right(), 374.0 - 288.0));
        assert!((large.window.bottom() - large.front_left_top).abs() < 0.01);
        // The page's paper spans 31 % to 69 % of the folder.
        let paper = large.page.width * (PAGE_PAPER.1 - PAGE_PAPER.0) / 32.0;
        assert!(near(paper, 0.38 * large.folder.width), "{large:?}");

        // The app's own folder keeps its outline, with the same fractions.
        let own = layout(96.0, Pieces::BuiltIn);
        assert_eq!((own.folder.x, own.folder.y, own.folder.width), (6.0, 12.0, 84.0));
        let fraction = |at: f32| (at - own.folder.y) / own.folder.height;
        assert!((fraction(own.sheet.y) - measured::SHEET_TOP_Y).abs() < 0.001);
        assert!((fraction(own.front_right_top) - measured::FRONT_RIGHT_TOP_Y).abs() < 0.001);
    }

    /// A folder's previews are read once, kept by its time and drawn on the
    /// frame its tile is bound when it is shown again; once the folder
    /// changes, its contents are read again.
    ///
    /// parity: VIEW-058
    #[gtk::test]
    fn a_folders_previews_are_kept_until_it_changes() {
        let cache = glib::user_cache_dir();
        if !cache.starts_with(std::env::temp_dir()) {
            // Never writes into a real thumbnail cache.
            return;
        }
        let folder = tempfile::tempdir().expect("a temporary folder");
        picture_at(folder.path(), "Lake.png", 0xff00_00ff);
        let mut entry = crate::test_support::folder_entry("Trip");
        entry.uri = gio::File::for_path(folder.path()).uri().to_string();
        entry.modified = Some(1_700_000_000);
        let request = policy().folder_request(&entry, 96).expect("a request");
        let context = gtk::glib::MainContext::default();
        assert!(kept_folder_art(&request).is_none(), "not read yet");

        let art = context
            .block_on(folder_art(request.clone()))
            .expect("the folder's art");

        assert!(art.picture.is_some());
        assert!(art.page.is_none());
        let Some(KnownArt::Drawn(kept)) = kept_folder_art(&request) else {
            panic!("the folder's art is kept");
        };
        assert!(kept.picture.is_some(), "drawn at once on a revisit");
        let changed = FolderRequest {
            modified: Some(1_700_000_001),
            ..request.clone()
        };
        assert!(kept_folder_art(&changed).is_none(), "read again once it changed");
        // An empty folder is known to stay plain once read.
        let empty = tempfile::tempdir().expect("a temporary folder");
        let plain = FolderRequest {
            uri: gio::File::for_path(empty.path()).uri().to_string(),
            ..request
        };
        assert!(context.block_on(folder_art(plain.clone())).is_none());
        assert!(matches!(kept_folder_art(&plain), Some(KnownArt::Plain)));
    }

    /// The color at `x`, `y` of `texture`: red, green, blue and alpha.
    fn color_at(texture: &gdk::Texture, x: f32, y: f32) -> [u8; 4] {
        let mut downloader = gdk::TextureDownloader::new(texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "inside the icon"
        )]
        let at = y as usize * stride + x as usize * 4;
        [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
    }

    fn near(color: [u8; 4], expected: [u8; 4]) -> bool {
        color
            .iter()
            .zip(expected)
            .all(|(found, expected)| found.abs_diff(expected) <= 24)
    }

    /// The app's own folder with contents is drawn as its pieces are laid
    /// out: a picture between the folder's back and its lower front,
    /// filling the sheet, the back's tab above it, nothing outside the
    /// folder.
    ///
    /// parity: VIEW-058
    #[gtk::test]
    fn a_folders_picture_peeks_out_between_its_back_and_front() {
        use crate::folder_view::cells::FolderContents;
        use crate::test_support::harness::wait_for_frames;

        crate::icons::register(&gdk::Display::default().expect("a display"));
        let picture = Pixbuf::new(Colorspace::Rgb, false, 8, 120, 90).expect("a picture");
        picture.fill(0x2040_e0ff);
        let contents = FolderContents::default();
        contents.set_edge(96);
        contents.set_halign(gtk::Align::Start);
        contents.set_valign(gtk::Align::Start);
        contents.show_art(Some(&FolderArt {
            picture: Some(gdk::Texture::for_pixbuf(&picture)),
            page: None,
        }));
        let window = gtk::Window::new();
        window.set_child(Some(&contents));
        window.present();
        wait_for_frames(&window, 3);
        let paintable = gtk::WidgetPaintable::new(Some(&contents));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, 96.0, 96.0);
        let node = snapshot.to_node().expect("the folder is drawn");
        let renderer = window.renderer().expect("a renderer");
        let drawn = renderer.render_texture(&node, Some(&gtk::graphene::Rect::new(0.0, 0.0, 96.0, 96.0)));
        window.destroy();

        if let Some(saved) = std::env::var_os("OX_SAVE_FOLDER_PREVIEW") {
            drawn.save_to_png(saved).expect("the drawing is saved");
        }
        let at = layout(96.0, Pieces::BuiltIn);
        let middle = at.folder.x + at.folder.width / 2.0;
        let blue = [0x20, 0x40, 0xe0, 0xff];
        let checks = [
            ("the picture", middle, at.sheet.y + 2.0, blue),
            (
                "down to the front",
                at.sheet.x + 2.0,
                at.front_left_top - 2.0,
                blue,
            ),
            ("the front", middle, at.front_right_top + 4.0, [252, 213, 63, 255]),
            (
                "the front on the left",
                at.folder.x + 4.0,
                at.front_left_top + 4.0,
                [252, 213, 63, 255],
            ),
            (
                "the tab",
                at.folder.x + 6.0,
                at.folder.y + 2.0,
                [255, 176, 46, 255],
            ),
        ];
        for (what, x, y, expected) in checks {
            let found = color_at(&drawn, x, y);
            assert!(near(found, expected), "{what} at {x}, {y}: {found:?}");
        }
        assert_eq!(color_at(&drawn, 1.0, 1.0)[3], 0, "nothing outside the folder");
        assert_eq!(
            color_at(&drawn, at.folder.right() - 3.0, at.folder.y + 1.0)[3],
            0,
            "nothing right of the tab"
        );
    }

    /// With Windows icon files on, a folder with a picture is drawn with
    /// their back (imageres group 5) and front (6), and one with files but
    /// no picture as their folder with a page sticking out (162); files
    /// without those groups, or the files off, leave the app's own folder.
    /// The files are synthetic test patterns, never Windows icons.
    ///
    /// parity: VIEW-058, LOOK-030
    #[gtk::test]
    fn windows_icon_files_draw_the_folders_back_front_and_page() {
        use crate::folder_view::cells::FolderContents;
        use crate::icons::user_icons::{follow_preferences, wait_until_read};
        use ox_core::settings::Preferences;
        use ox_core::windows_icons::synthetic::SyntheticIconFile;

        /// Turns the icon files off again when the test ends.
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                follow_preferences(&Preferences::default());
            }
        }
        let _reset = Reset;
        let page = Peeking {
            name: "Meeting notes.txt".to_owned(),
            uri: "file:///home/demo/Projects/Meeting%20notes.txt".to_owned(),
            content_type: "text/plain".to_owned(),
            modified: Some(1),
            size: Some(7),
        };
        let picture = Pixbuf::new(Colorspace::Rgb, false, 8, 60, 40).expect("a picture");
        let with_picture = FolderArt {
            picture: Some(gdk::Texture::for_pixbuf(&picture)),
            page: None,
        };
        let with_page = FolderArt {
            picture: None,
            page: Some(page),
        };
        let drawn_with = |art: &FolderArt| {
            let contents = FolderContents::default();
            contents.set_edge(96);
            contents.show_art(Some(art));
            contents.uses_windows_pieces()
        };
        let choose = |groups: &[u16]| {
            let folder = tempfile::tempdir().expect("a temporary folder");
            let path = folder.path().join("imageres.dll.mun");
            std::fs::write(&path, SyntheticIconFile::with_groups(groups).build()).expect("written");
            follow_preferences(&Preferences {
                use_icon_files: true,
                icon_files: vec![path.display().to_string()],
                ..Preferences::default()
            });
            wait_until_read();
            folder
        };

        let _files = choose(&[3, 5, 6, 162]);
        assert_eq!(drawn_with(&with_picture), (true, false), "back and front");
        assert_eq!(drawn_with(&with_page), (true, true), "the folder with a page");

        let _files = choose(&[3]);
        assert_eq!(
            drawn_with(&with_picture),
            (false, false),
            "the folder cut instead"
        );
        assert_eq!(drawn_with(&with_page), (false, false));

        follow_preferences(&Preferences::default());
        assert_eq!(drawn_with(&with_page), (false, false), "files off");
    }

    /// Medium icons and larger and Tiles show a folder's contents, as
    /// Windows does; Details, List, Small icons and Content keep the plain
    /// folder, and so does every view with "Always show icons, never
    /// thumbnails" on.
    ///
    /// parity: VIEW-058
    #[gtk::test]
    fn only_medium_icons_and_larger_and_tiles_show_a_folders_contents() {
        use crate::folder_view::cells::{CellLayout, FileCell};
        use crate::folder_view::item::FileItem;
        use crate::test_support::harness::{wait_for, wait_until};

        if !glib::user_cache_dir().starts_with(std::env::temp_dir()) {
            return;
        }
        let folder = tempfile::tempdir().expect("a temporary folder");
        picture_at(folder.path(), "Harbor.png", 0x2040_e0ff);
        std::fs::write(folder.path().join("Notes.txt"), "notes").expect("a file");
        let mut entry = crate::test_support::folder_entry("Trip");
        entry.uri = gio::File::for_path(folder.path()).uri().to_string();
        entry.modified = Some(1_700_000_100);
        let item = FileItem::new(entry);
        let icons_only = PreviewPolicy::for_folder(
            &ViewOptions {
                show_previews: false,
                ..ViewOptions::default()
            },
            false,
        );

        for (layout, size) in [
            (CellLayout::IconTile, 96),
            (CellLayout::IconTile, 48),
            (CellLayout::Tile, 48),
        ] {
            let cell = FileCell::new(layout, size);
            cell.look_up_picture(&item, policy());
            wait_until("the folder's contents", || {
                cell.shown_folder_contents() == Some((true, false))
            });
            let plain = FileCell::new(layout, size);
            plain.look_up_picture(&item, icons_only);
            wait_for(std::time::Duration::from_millis(100));
            assert_eq!(plain.shown_folder_contents(), None, "icons only");
        }
        for (layout, size) in [
            (CellLayout::DetailsRow, 16),
            (CellLayout::CompactItem, 16),
            (CellLayout::SmallIconItem, 16),
            (CellLayout::ContentRow, 48),
            (CellLayout::IconTile, 32),
        ] {
            let cell = FileCell::new(layout, size);
            cell.look_up_picture(&item, policy());
            wait_for(std::time::Duration::from_millis(100));
            assert_eq!(cell.shown_folder_contents(), None, "{layout:?} at {size}");
        }
    }
}
