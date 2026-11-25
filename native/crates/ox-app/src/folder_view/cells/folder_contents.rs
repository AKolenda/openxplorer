// SPDX-License-Identifier: AGPL-3.0-only
//! A folder's contents peeking out of its icon (VIEW-058), as Windows 11
//! draws a folder with contents at Medium icons and larger and in Tiles:
//! the thumbnail of the folder's first picture, straight, or the icon of
//! its first file as a page, in front of the folder's back and behind its
//! front, which is lower than a plain folder's. The pieces go where
//! [`layout`] puts them.
//!
//! The app draws no art of its own (`SOURCES.md`): the folder is its usual
//! folder icon, drawn whole behind the contents and again in front of
//! them below the front's edge only, cut along it by clips. With the
//! user's Windows icon files on, that icon is Windows' folder, the back
//! and front come from the files instead when they hold both
//! ([`IconRole::FolderBack`], [`IconRole::FolderFront`]), and a folder
//! with files but no picture is Windows' own folder with a page sticking
//! out ([`IconRole::FolderWithFiles`]) when they hold it.
//!
//! The widget lies over the cell's art, which the cell hides while it
//! shows contents; a cell with nothing to show hides the widget.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use ox_core::windows_icons::IconRole;

use crate::icons::{Art, ArtImage, FileType};
use crate::thumbnails::folder_preview::{layout, FolderArt, Layout, Pieces, Rect};

/// How far the clips that cut the front overlap, in pixels, so no seam
/// shows where they meet.
const CLIP_OVERLAP: f32 = 1.0;

fn rect(rect: Rect) -> graphene::Rect {
    graphene::Rect::new(rect.x, rect.y, rect.width, rect.height)
}

mod imp {
    use super::{ArtImage, Cell, RefCell};
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gdk, glib};

    /// Private state of [`super::FolderContents`].
    #[derive(Debug, Default)]
    pub(crate) struct FolderContents {
        /// The folder's own icon, drawn as its back and its front.
        pub(super) folder: ArtImage,
        /// The thumbnail that peeks out.
        pub(super) picture: RefCell<Option<gdk::Texture>>,
        /// The page's icon, made the first time a page peeks out.
        pub(super) page: RefCell<Option<ArtImage>>,
        /// The icon's edge, in logical pixels.
        pub(super) edge: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FolderContents {
        const NAME: &'static str = "OxFolderContents";
        type Type = super::FolderContents;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("foldercontents");
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for FolderContents {
        fn constructed(&self) {
            self.parent_constructed();
            self.folder.set_parent(&*self.obj());
            // The pieces of the Windows icon files come and go with them.
            let contents = self.obj().downgrade();
            crate::icons::user_icons::follow_status(move |_| {
                contents.upgrade().is_some_and(|contents| {
                    contents.queue_resize();
                    true
                })
            });
        }

        fn dispose(&self) {
            self.folder.unparent();
            if let Some(page) = self.page.take() {
                page.unparent();
            }
        }
    }

    impl WidgetImpl for FolderContents {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let edge = self.edge.get();
            (edge, edge, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            self.folder
                .size_allocate(&gtk::Allocation::new(0, 0, width, height), -1);
            let Some(page) = self.page.borrow().clone() else {
                return;
            };
            let place = self.obj().layout().page;
            #[expect(clippy::cast_possible_truncation, reason = "icon sizes are small")]
            let allocation = gtk::Allocation::new(
                place.x.round() as i32,
                place.y.round() as i32,
                place.width.round() as i32,
                place.height.round() as i32,
            );
            page.size_allocate(&allocation, -1);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    /// A folder's contents peeking out of its icon.
    pub(crate) struct FolderContents(ObjectSubclass<imp::FolderContents>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for FolderContents {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl FolderContents {
    /// Draws the folder on an icon `edge` logical pixels square.
    pub(crate) fn set_edge(&self, edge: i32) {
        let imp = self.imp();
        imp.edge.set(edge);
        imp.folder.set_art(Art::Folder, edge);
        if let Some(page) = imp.page.borrow().as_ref() {
            if let Some(Art::File(file_type)) = page.art() {
                page.set_art(Art::File(file_type), self.page_edge());
            }
        }
        self.queue_resize();
    }

    /// Shows `art` of a folder's contents, or hides the widget for `None`.
    pub(crate) fn show_art(&self, art: Option<&FolderArt>) {
        let imp = self.imp();
        imp.picture.replace(art.and_then(|art| art.picture.clone()));
        let page = art.and_then(|art| art.page.as_ref());
        if let Some(page) = page {
            let file_type = FileType::of(&page.name, Some(&page.content_type));
            let image = imp
                .page
                .borrow_mut()
                .get_or_insert_with(|| {
                    let image = ArtImage::default();
                    image.set_parent(self);
                    image
                })
                .clone();
            image.set_art(Art::File(file_type), self.page_edge());
            image.set_visible(true);
        } else if let Some(image) = imp.page.borrow().as_ref() {
            image.set_visible(false);
        }
        self.set_visible(art.is_some());
        self.queue_resize();
        self.queue_draw();
    }

    /// Whether a picture peeks out, and whether a page does, for tests.
    #[cfg(test)]
    pub(crate) fn shown(&self) -> Option<(bool, bool)> {
        let imp = self.imp();
        let page = imp.page.borrow().as_ref().is_some_and(WidgetExt::is_visible);
        self.is_visible().then(|| (imp.picture.borrow().is_some(), page))
    }

    /// Which pictures of the Windows icon files the folder is drawn with
    /// now: its back and front, and its folder with a page sticking out,
    /// for tests.
    #[cfg(test)]
    pub(crate) fn uses_windows_pieces(&self) -> (bool, bool) {
        (
            self.windows_pieces().is_some(),
            self.windows_folder_with_files().is_some(),
        )
    }

    /// The page's icon's edge, in logical pixels.
    fn page_edge(&self) -> i32 {
        #[expect(clippy::cast_possible_truncation, reason = "icon sizes are small")]
        let edge = self.layout().page.width.round() as i32;
        edge.max(1)
    }

    /// The back and front from the Windows icon files, when they hold both.
    fn windows_pieces(&self) -> Option<(gdk::Texture, gdk::Texture)> {
        let (edge, scale) = (self.imp().edge.get(), self.scale_factor());
        let back = crate::icons::user_icons::texture_of(IconRole::FolderBack, edge, scale)?;
        let front = crate::icons::user_icons::texture_of(IconRole::FolderFront, edge, scale)?;
        Some((back, front))
    }

    /// Windows' folder with a page sticking out, from the Windows icon
    /// files, for a folder whose page peeks out, when they hold it.
    fn windows_folder_with_files(&self) -> Option<gdk::Texture> {
        let imp = self.imp();
        if !imp.page.borrow().as_ref().is_some_and(WidgetExt::is_visible) {
            return None;
        }
        crate::icons::user_icons::texture_of(IconRole::FolderWithFiles, imp.edge.get(), self.scale_factor())
    }

    /// Where the pieces go on this icon: on Windows' folder, as Windows
    /// places them, while the icon files draw it.
    fn layout(&self) -> Layout {
        let (edge, scale) = (self.imp().edge.get(), self.scale_factor());
        let windows = crate::icons::user_icons::texture_of(IconRole::Folder, edge, scale).is_some()
            || self.windows_pieces().is_some();
        let pieces = if windows { Pieces::Windows } else { Pieces::BuiltIn };
        #[expect(clippy::cast_precision_loss, reason = "icon sizes are small")]
        layout(edge as f32, pieces)
    }

    /// Draws the back, the pictures, the page and the front.
    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        #[expect(clippy::cast_precision_loss, reason = "icon sizes are small")]
        let edge = imp.edge.get() as f32;
        let whole = graphene::Rect::new(0.0, 0.0, edge, edge);
        if let Some(folder) = self.windows_folder_with_files() {
            snapshot.append_scaled_texture(&folder, gsk::ScalingFilter::Trilinear, &whole);
            return;
        }
        let at = self.layout();
        let windows = self.windows_pieces();
        let folder = if windows.is_none() {
            let drawn = gtk::Snapshot::new();
            self.snapshot_child(&imp.folder, &drawn);
            drawn.to_node()
        } else {
            None
        };
        match (&windows, &folder) {
            (Some((back, _)), _) => {
                snapshot.append_scaled_texture(back, gsk::ScalingFilter::Trilinear, &whole);
            }
            (None, Some(folder)) => snapshot.append_node(folder),
            (None, None) => {}
        }
        if let Some(picture) = imp.picture.borrow().as_ref() {
            draw_covering(snapshot, picture, at.window);
        }
        if let Some(page) = imp.page.borrow().as_ref() {
            self.snapshot_child(page, snapshot);
        }
        match (&windows, &folder) {
            (Some((_, front)), _) => {
                snapshot.append_scaled_texture(front, gsk::ScalingFilter::Trilinear, &whole);
            }
            (None, Some(folder)) => draw_front(snapshot, folder, &at),
            (None, None) => {}
        }
    }
}

/// The folder's icon `folder` again, below the front's edge only: level
/// on the left, a slope up under the tab's slope, and level to the right.
fn draw_front(snapshot: &gtk::Snapshot, folder: &gsk::RenderNode, at: &Layout) {
    let (left, right, bottom) = (at.folder.x, at.folder.right(), at.folder.bottom());
    let level = |x: f32, top: f32, width: f32| {
        snapshot.push_clip(&graphene::Rect::new(x, top, width, bottom - top));
        snapshot.append_node(folder);
        snapshot.pop();
    };
    level(left, at.front_left_top, at.step_start - left + CLIP_OVERLAP);
    level(
        at.step_end - CLIP_OVERLAP,
        at.front_right_top,
        right - at.step_end + CLIP_OVERLAP,
    );
    // The slope: between its ends, below the line that joins them.
    let (run, rise) = (
        at.step_end - at.step_start,
        at.front_right_top - at.front_left_top,
    );
    let degrees = rise.atan2(run).to_degrees();
    let length = run.hypot(rise);
    snapshot.push_clip(&graphene::Rect::new(
        at.step_start,
        at.front_right_top,
        run,
        bottom - at.front_right_top,
    ));
    snapshot.save();
    let corner = graphene::Point::new(at.step_start, at.front_left_top);
    snapshot.translate(&corner);
    snapshot.rotate(degrees);
    snapshot.push_clip(&graphene::Rect::new(
        -length,
        0.0,
        3.0 * length,
        2.0 * at.folder.height,
    ));
    snapshot.rotate(-degrees);
    snapshot.translate(&graphene::Point::new(-corner.x(), -corner.y()));
    snapshot.append_node(folder);
    snapshot.pop();
    snapshot.restore();
    snapshot.pop();
}

/// `texture` scaled to cover `place`, centered and cut to it.
fn draw_covering(snapshot: &gtk::Snapshot, texture: &gdk::Texture, place: Rect) {
    #[expect(clippy::cast_precision_loss, reason = "picture sizes are small")]
    let (width, height) = (texture.width() as f32, texture.height() as f32);
    if width <= 0.0 || height <= 0.0 || place.width <= 0.0 || place.height <= 0.0 {
        return;
    }
    let scale = (place.width / width).max(place.height / height);
    let (drawn_width, drawn_height) = (width * scale, height * scale);
    snapshot.push_clip(&rect(place));
    snapshot.append_scaled_texture(
        texture,
        gsk::ScalingFilter::Trilinear,
        &graphene::Rect::new(
            place.x + (place.width - drawn_width) / 2.0,
            place.y + (place.height - drawn_height) / 2.0,
            drawn_width,
            drawn_height,
        ),
    );
    snapshot.pop();
}
