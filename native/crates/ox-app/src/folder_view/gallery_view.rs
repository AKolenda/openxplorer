// SPDX-License-Identifier: AGPL-3.0-only
//! Gallery's view (HOME-026 to HOME-028), as Windows 11 File Explorer's
//! Gallery shows photos: justified rows of thumbnails that keep each
//! photo's aspect ratio ([`super::justified`]), one heading over them with
//! the months of the photos on screen ("November 2026 – July 2025"), and
//! a date scrubber at the right edge in place of the scroll bar
//! ([`super::gallery_scrubber`]).
//!
//! GTK's grid view gives every tile one size, so Gallery lays its tiles
//! out itself: a [`GalleryCanvas`] as tall as every row, in a viewport,
//! makes widgets only for the rows near the part on screen and reuses
//! them as the view scrolls, so 10,000 photos cost no more than a
//! screenful. It shows the folder pane's own selection model, so
//! selecting, the menus, the commands and the status bar act on the same
//! items as in a folder; its tiles are registered with the pane's
//! [`CellOwners`] as the grid's cells are, so the window's pointer
//! handling (right-click, drag, rubber band) finds them.
//!
//! Keys: the arrows move among the tiles (Up and Down to the nearest
//! tile of the row above or below), Home and End to the first and last,
//! Page Up and Page Down a screen; Shift extends the selection from its
//! anchor, Ctrl moves the keyboard alone and Ctrl+Space selects or
//! deselects. Enter and a double-click open, as in a folder.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use super::cells::CellOwners;
use super::gallery_scrubber::GalleryScrubber;
use super::icon_size::IconSize;
use super::item::FileItem;
use super::justified::{justify, Layout};
use super::sort_roles::photo_date;

/// Pixels between tiles and between rows.
pub(crate) const GALLERY_GAP: i32 = 4;

/// How many screens of rows beyond the one shown keep their tiles, above
/// and below, so a scroll of a page never shows an empty row.
const ROWS_KEPT_BEYOND: f64 = 1.0;

/// The height of Gallery's rows at `size`: about 160 pixels at Medium
/// icons, as Windows shows them.
pub(crate) fn row_height(size: IconSize) -> i32 {
    match nearest_named(size) {
        IconSize::SMALL => 100,
        IconSize::LARGE => 240,
        IconSize::EXTRA_LARGE => 320,
        _ => 160,
    }
}

/// The named icon size nearest `size`: Gallery offers only those.
pub(crate) fn nearest_named(size: IconSize) -> IconSize {
    IconSize::NAMED
        .into_iter()
        .min_by_key(|named| (named.pixels() - size.pixels()).abs())
        .unwrap_or(IconSize::MEDIUM)
}

/// The edge of the square a photo of `aspect` (width over height, when
/// known) is decoded to fit, so it covers a tile `width` by `height`
/// pixels at `scale` device pixels each: the longest edge it needs.
pub(crate) fn sharp_edge(width: i32, height: i32, aspect: Option<f64>, scale: i32) -> i32 {
    let (width, height) = (f64::from(width * scale.max(1)), f64::from(height * scale.max(1)));
    let aspect = aspect
        .filter(|aspect| aspect.is_finite() && *aspect > 0.0)
        .unwrap_or(width / height.max(1.0));
    let needed = width.max(height * aspect).max(height).max(width / aspect);
    #[expect(clippy::cast_possible_truncation, reason = "edges are small")]
    let edge = needed.ceil() as i32;
    edge.max(1)
}

/// What a tile on screen shows, for tests.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct TileState {
    /// A picture is drawn.
    pub(crate) thumbnail: bool,
    /// The device pixels its sharp picture was asked at.
    pub(crate) requested: Option<(i32, i32)>,
    /// The size of the picture drawn, in pixels.
    pub(crate) drawn: Option<(i32, i32)>,
}

/// The aspect ratio (width over height) of `item`'s photo as shown, if
/// its size was read.
fn aspect_of(item: Option<&FileItem>) -> Option<f64> {
    let (width, height) = item?.entry().meta.pixel_size?;
    (height > 0).then(|| f64::from(width) / f64::from(height))
}

/// A month's name and year, as Gallery's heading and scrubber name it
/// ("November 2026").
pub(crate) fn month_text(seconds: Option<u64>) -> Option<String> {
    let seconds = i64::try_from(seconds?).ok()?;
    let date = glib::DateTime::from_unix_local(seconds).ok()?;
    date.format("%B %Y").ok().map(|text| text.to_string())
}

/// The heading for photos dated `newest` to `oldest`: one month, or the
/// newest month and the oldest with an en dash between them.
pub(crate) fn range_text(newest: Option<u64>, oldest: Option<u64>) -> Option<String> {
    let newest_text = month_text(newest);
    let oldest_text = month_text(oldest);
    match (newest_text, oldest_text) {
        (Some(newest), Some(oldest)) if newest != oldest => Some(format!("{newest} – {oldest}")),
        (Some(month), _) | (None, Some(month)) => Some(month),
        (None, None) => None,
    }
}

/// A photo's tooltip, as Windows 11's Gallery words it: "Path: <name>
/// (<folder>)", "Item type: PNG File", "Date: …" (taken, else modified)
/// and "Size: …".
fn photo_tooltip(owners: &CellOwners, item: &FileItem) -> String {
    use ox_core::format;
    use ox_core::i18n::format_message;
    let entry = item.entry();
    let name = owners.shown_name(item);
    let folder = gtk::gio::File::for_uri(&entry.uri)
        .parent()
        .and_then(|parent| parent.path())
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let mut lines = vec![format_message(
        "Path: {name} ({folder})",
        &[("name", &name), ("folder", &folder)],
    )];
    if !entry.type_label.is_empty() {
        lines.push(format_message(
            "Item type: {type}",
            &[("type", &entry.type_label)],
        ));
    }
    if let Some(date) = photo_date(entry) {
        let date = format::date_short_time_text(Some(date));
        lines.push(format_message("Date: {date}", &[("date", &date)]));
    }
    if let Some(size) = item.file_size() {
        let size = format::short_size(size);
        lines.push(format_message("Size: {size}", &[("size", &size)]));
    }
    lines.join("\n")
}

mod tile {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gdk, glib, graphene, gsk};

    use crate::folder_view::cells::TileSlot;

    mod imp {
        use super::{gdk, glib, graphene, gsk, Cell, Rc, RefCell, TileSlot};
        use gtk::prelude::*;
        use gtk::subclass::prelude::*;

        /// Private state of [`super::GalleryPicture`].
        #[derive(Debug, Default)]
        pub(crate) struct GalleryPicture {
            pub(super) texture: RefCell<Option<gdk::Texture>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for GalleryPicture {
            const NAME: &'static str = "OxGalleryPicture";
            type Type = super::GalleryPicture;
            type ParentType = gtk::Widget;

            fn class_init(klass: &mut Self::Class) {
                klass.set_css_name("picture");
            }
        }

        impl ObjectImpl for GalleryPicture {}

        impl WidgetImpl for GalleryPicture {
            fn snapshot(&self, snapshot: &gtk::Snapshot) {
                let widget = self.obj();
                #[expect(clippy::cast_precision_loss, reason = "tile sizes are small")]
                let (width, height) = (widget.width() as f32, widget.height() as f32);
                let Some(texture) = self.texture.borrow().clone() else {
                    return;
                };
                if width <= 0.0 || height <= 0.0 {
                    return;
                }
                // Covers the tile, cropping what overflows, as a photo
                // grid does; the layout keeps the aspect, so only a photo
                // narrower or wider than the layout allows is cropped.
                #[expect(clippy::cast_precision_loss, reason = "texture sizes are small")]
                let (texture_width, texture_height) = (texture.width() as f32, texture.height() as f32);
                #[expect(clippy::cast_precision_loss, reason = "scales are small")]
                let device = widget.scale_factor() as f32;
                if (texture_width - width * device).abs() < 0.5
                    && (texture_height - height * device).abs() < 0.5
                {
                    // The tile's own pixels, drawn one for one (HOME-033).
                    let bounds = graphene::Rect::new(0.0, 0.0, width, height);
                    snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, 4.0));
                    snapshot.append_texture(&texture, &bounds);
                    snapshot.pop();
                    return;
                }
                let scale = (width / texture_width).max(height / texture_height);
                let drawn = graphene::Rect::new(
                    (width - texture_width * scale) / 2.0,
                    (height - texture_height * scale) / 2.0,
                    texture_width * scale,
                    texture_height * scale,
                );
                let bounds = graphene::Rect::new(0.0, 0.0, width, height);
                snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, 4.0));
                // A plain texture node, drawn at the display's own pixels:
                // GTK's software renderer draws a scaled texture node
                // through a surface of the widget's logical size, which at
                // a scale of 2 halves the picture and blurs it (HOME-033).
                snapshot.append_texture(&texture, &drawn);
                snapshot.pop();
            }
        }

        /// Private state of [`super::GalleryTile`].
        #[derive(Debug, Default)]
        pub(crate) struct GalleryTile {
            pub(super) slot: Rc<TileSlot>,
            pub(super) picture: std::cell::OnceCell<super::GalleryPicture>,
            /// The preview being read for the item shown.
            pub(super) preview: RefCell<Option<glib::JoinHandle<()>>>,
            /// The item, version and size the preview was asked for.
            pub(super) asked: RefCell<Option<(String, Option<u64>, i32)>>,
            /// The sharp picture being decoded for the tile on screen.
            pub(super) sharp: RefCell<Option<glib::JoinHandle<()>>>,
            /// The item and edge the sharp picture was asked for, decoded
            /// or still being decoded.
            pub(super) sharp_asked: RefCell<Option<(String, (i32, i32))>>,
            pub(super) selected: Cell<bool>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for GalleryTile {
            const NAME: &'static str = "OxGalleryTile";
            type Type = super::GalleryTile;
            type ParentType = gtk::Widget;

            fn class_init(klass: &mut Self::Class) {
                // The name GTK's grid gives its tiles, which the cell
                // registry and the stylesheet know.
                klass.set_css_name("child");
                klass.set_layout_manager_type::<gtk::BinLayout>();
                klass.set_accessible_role(gtk::AccessibleRole::GridCell);
            }
        }

        impl ObjectImpl for GalleryTile {
            fn constructed(&self) {
                self.parent_constructed();
                let picture: super::GalleryPicture = glib::Object::new();
                picture.set_can_target(true);
                picture.set_parent(&*self.obj());
                self.picture.set(picture).expect("constructed runs once");
                self.obj().add_css_class("gallery-tile");
                self.obj().set_overflow(gtk::Overflow::Hidden);
            }

            fn dispose(&self) {
                if let Some(preview) = self.preview.take() {
                    preview.abort();
                }
                if let Some(sharp) = self.sharp.take() {
                    sharp.abort();
                }
                if let Some(picture) = self.picture.get() {
                    picture.unparent();
                }
            }
        }

        impl WidgetImpl for GalleryTile {}
    }

    glib::wrapper! {
        /// What draws a photo's thumbnail in its tile.
        pub(crate) struct GalleryPicture(ObjectSubclass<imp::GalleryPicture>)
            @extends gtk::Widget,
            @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
    }

    impl GalleryPicture {
        /// Draws `texture`, or nothing yet.
        pub(crate) fn show(&self, texture: Option<gdk::Texture>) {
            self.imp().texture.replace(texture);
            self.queue_draw();
        }

        /// Draws `texture` unless the picture drawn has more pixels: a
        /// cached thumbnail that comes late never replaces the sharp one.
        pub(crate) fn offer(&self, texture: gdk::Texture) {
            let area = |texture: &gdk::Texture| i64::from(texture.width()) * i64::from(texture.height());
            let sharper = self
                .imp()
                .texture
                .borrow()
                .as_ref()
                .is_some_and(|drawn| area(drawn) > area(&texture));
            if !sharper {
                self.show(Some(texture));
            }
        }

        /// Whether a thumbnail is drawn, for tests.
        #[cfg(test)]
        pub(crate) fn shows_thumbnail(&self) -> bool {
            self.imp().texture.borrow().is_some()
        }

        /// The size of the picture drawn, for tests.
        #[cfg(test)]
        pub(crate) fn texture_pixels(&self) -> Option<(i32, i32)> {
            let texture = self.imp().texture.borrow();
            texture
                .as_ref()
                .map(|texture| (texture.width(), texture.height()))
        }
    }

    glib::wrapper! {
        /// A photo's tile: the cell the registry knows, with its picture.
        pub(crate) struct GalleryTile(ObjectSubclass<imp::GalleryTile>)
            @extends gtk::Widget,
            @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
    }

    impl GalleryTile {
        pub(crate) fn new() -> Self {
            glib::Object::new()
        }

        pub(crate) fn slot(&self) -> &Rc<TileSlot> {
            &self.imp().slot
        }

        pub(crate) fn picture(&self) -> &GalleryPicture {
            self.imp().picture.get().expect("constructed builds the picture")
        }

        /// The preview task of the item shown, which a new item replaces.
        pub(crate) fn set_preview(&self, task: Option<glib::JoinHandle<()>>) {
            if let Some(previous) = self.imp().preview.replace(task) {
                previous.abort();
            }
        }

        /// What the preview was last asked for.
        pub(crate) fn asked(&self) -> &RefCell<Option<(String, Option<u64>, i32)>> {
            &self.imp().asked
        }

        /// The device pixels the sharp picture was asked at, for tests: the
        /// tile's own.
        #[cfg(test)]
        pub(crate) fn requested_pixels(&self) -> Option<(i32, i32)> {
            self.imp()
                .sharp_asked
                .borrow()
                .as_ref()
                .map(|(_, pixels)| *pixels)
        }

        /// The item and edge of the sharp picture asked for.
        pub(crate) fn sharp_asked(&self) -> &RefCell<Option<(String, (i32, i32))>> {
            &self.imp().sharp_asked
        }

        /// The sharp picture's decode, which a new one or `None` cancels.
        pub(crate) fn set_sharp(&self, task: Option<glib::JoinHandle<()>>) {
            if let Some(previous) = self.imp().sharp.replace(task) {
                previous.abort();
            }
        }

        /// Forgets the sharp picture's decode, which has finished.
        pub(crate) fn finish_sharp(&self) {
            drop(self.imp().sharp.take());
        }

        /// Cancels a sharp picture still being decoded; one decoded stays.
        pub(crate) fn cancel_pending_sharp(&self) {
            let pending = self.imp().sharp.take();
            if let Some(pending) = pending {
                // A finished task is gone already; an unfinished one is
                // asked again once the tile is back on screen.
                pending.abort();
                self.imp().sharp_asked.replace(None);
            }
        }

        /// Shows the tile selected or not.
        pub(crate) fn set_selected(&self, selected: bool) {
            if self.imp().selected.replace(selected) == selected {
                return;
            }
            if selected {
                self.set_state_flags(gtk::StateFlags::SELECTED, false);
            } else {
                self.unset_state_flags(gtk::StateFlags::SELECTED);
            }
            self.update_state(&[gtk::accessible::State::Selected(Some(selected))]);
        }
    }
}

pub(crate) use tile::GalleryTile;

/// Opens the item at a position: Enter or a double-click.
type Activate = Box<dyn Fn(u32)>;
/// What a tile's tooltip says of its item.
type TooltipText = Box<dyn Fn(&FileItem) -> Option<String>>;

mod imp {
    use super::{
        Activate, Cell, CellOwners, FileItem, GalleryTile, Layout, OnceCell, Rc, RefCell, TooltipText,
    };
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::GalleryCanvas`].
    #[derive(Default)]
    pub(crate) struct GalleryCanvas {
        pub(super) owners: OnceCell<Rc<CellOwners>>,
        pub(super) model: RefCell<Option<gtk::MultiSelection>>,
        pub(super) model_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        /// The aspect ratio of each item, by position.
        pub(super) aspects: RefCell<Vec<Option<f64>>>,
        pub(super) layout: RefCell<Layout>,
        /// The width and row height the layout was made for.
        pub(super) laid_for: Cell<(i32, i32)>,
        pub(super) row_height: Cell<i32>,
        pub(super) icon_size: Cell<Option<crate::folder_view::icon_size::IconSize>>,
        pub(super) tiles: RefCell<Vec<GalleryTile>>,
        pub(super) vadjustment: RefCell<Option<gtk::Adjustment>>,
        /// The item with the keyboard, and the one Shift extends from.
        pub(super) focus: Cell<Option<u32>>,
        pub(super) anchor: Cell<Option<u32>>,
        /// A press on a selected item, which selects it alone on release
        /// unless a drag began.
        pub(super) pending_click: Cell<Option<u32>>,
        /// An item to scroll to once the layout knows its width.
        pub(super) pending_reveal: Cell<Option<u32>>,
        pub(super) on_activate: RefCell<Option<Activate>>,
        pub(super) tooltip_text: RefCell<Option<TooltipText>>,
        /// Runs when what is on screen changed: the heading follows.
        pub(super) on_shown: RefCell<Vec<Box<dyn Fn()>>>,
        /// The view is told of a change once GTK is idle again.
        pub(super) shown_pending: Cell<bool>,
        pub(super) _item: std::marker::PhantomData<FileItem>,
    }

    impl std::fmt::Debug for GalleryCanvas {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("GalleryCanvas")
                .field("tiles", &self.tiles.borrow().len())
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GalleryCanvas {
        const NAME: &'static str = "OxGalleryCanvas";
        type Type = super::GalleryCanvas;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("gallery");
            klass.set_accessible_role(gtk::AccessibleRole::Grid);
        }
    }

    impl ObjectImpl for GalleryCanvas {
        fn constructed(&self) {
            self.parent_constructed();
            let canvas = self.obj();
            canvas.set_focusable(true);
            canvas.set_overflow(gtk::Overflow::Hidden);
            canvas.add_css_class("files");
            self.row_height
                .set(super::row_height(crate::folder_view::icon_size::IconSize::MEDIUM));
            canvas.install_input();
        }

        fn dispose(&self) {
            for tile in self.tiles.take() {
                tile.unparent();
            }
        }
    }

    impl WidgetImpl for GalleryCanvas {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                return (1, 1, -1, -1);
            }
            let width = if for_size > 0 { for_size } else { 800 };
            let height = self.obj().layout_for(width).height.max(1);
            (height, height, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().place_tiles(width);
            // A menu attached to the canvas (the right-click menu) is laid
            // out by its parent.
            for popover in self.obj().popovers() {
                popover.present();
            }
        }

        fn grab_focus(&self) -> bool {
            let grabbed = self.parent_grab_focus();
            self.obj().show_keyboard_focus();
            grabbed
        }
    }
}

glib::wrapper! {
    /// Gallery's tiles in justified rows, as tall as all of them.
    pub(crate) struct GalleryCanvas(ObjectSubclass<imp::GalleryCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl GalleryCanvas {
    fn new(owners: &Rc<CellOwners>) -> Self {
        let canvas: Self = glib::Object::new();
        canvas
            .imp()
            .owners
            .set(Rc::clone(owners))
            .expect("a canvas has one registry");
        canvas
    }

    fn owners(&self) -> &Rc<CellOwners> {
        self.imp().owners.get().expect("new sets the registry")
    }

    /// The items listed, with their selection.
    pub(crate) fn model(&self) -> Option<gtk::MultiSelection> {
        self.imp().model.borrow().clone()
    }

    fn n_items(&self) -> u32 {
        self.model().map_or(0, |model| model.n_items())
    }

    fn item(&self, position: u32) -> Option<FileItem> {
        self.model()?.item(position).and_downcast::<FileItem>()
    }

    /// Lists `model`, or nothing.
    pub(crate) fn set_model(&self, model: Option<&gtk::MultiSelection>) {
        let imp = self.imp();
        if imp.model.borrow().as_ref() == model {
            return;
        }
        if let Some(old) = imp.model.take() {
            for handler in imp.model_handlers.take() {
                old.disconnect(handler);
            }
        }
        if let Some(model) = model {
            let items = model.connect_items_changed(glib::clone!(
                #[weak(rename_to = canvas)]
                self,
                move |_, position, removed, added| canvas.items_changed(position, removed, added)
            ));
            let selection = model.connect_selection_changed(glib::clone!(
                #[weak(rename_to = canvas)]
                self,
                move |_, _, _| canvas.show_selection()
            ));
            imp.model_handlers.replace(vec![items, selection]);
        }
        imp.model.replace(model.cloned());
        imp.focus.set(None);
        imp.anchor.set(None);
        self.read_aspects();
    }

    /// Follows `adjustment`, the viewport's, to make tiles for the rows
    /// that come on screen.
    fn follow_adjustment(&self, adjustment: &gtk::Adjustment) {
        adjustment.connect_value_changed(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_| {
                canvas.queue_allocate();
                canvas.notify_shown();
            }
        ));
        adjustment.connect_page_size_notify(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_| {
                canvas.queue_allocate();
                canvas.notify_shown();
            }
        ));
        self.imp().vadjustment.replace(Some(adjustment.clone()));
    }

    pub(crate) fn vadjustment(&self) -> Option<gtk::Adjustment> {
        self.imp().vadjustment.borrow().clone()
    }

    fn items_changed(&self, position: u32, removed: u32, added: u32) {
        let imp = self.imp();
        let moved = |kept: Option<u32>| {
            kept.and_then(|kept| match kept {
                kept if kept < position => Some(kept),
                kept if kept < position + removed => None,
                kept => Some(kept - removed + added),
            })
        };
        imp.focus.set(moved(imp.focus.get()));
        imp.anchor.set(moved(imp.anchor.get()));
        // Only the items that changed are read: a listing arrives in many
        // small changes, and every one of them reading all 10,000 photos
        // would hold the window up.
        let Some(model) = self.model() else {
            return;
        };
        let new: Vec<Option<f64>> = (position..position + added)
            .map(|position| aspect_of(model.item(position).and_downcast::<FileItem>().as_ref()))
            .collect();
        {
            let mut aspects = imp.aspects.borrow_mut();
            let start = (position as usize).min(aspects.len());
            let end = (start + removed as usize).min(aspects.len());
            aspects.splice(start..end, new);
        }
        self.relayout();
    }

    /// Lays the tiles out again once GTK lays the canvas out, every tile
    /// bound anew, as positions moved.
    fn relayout(&self) {
        let imp = self.imp();
        imp.laid_for.set((0, 0));
        for tile in imp.tiles.borrow().iter() {
            tile.slot().show(None);
        }
        self.queue_resize();
        self.notify_shown();
    }

    /// Reads each item's aspect ratio and lays the tiles out again.
    fn read_aspects(&self) {
        let aspects: Vec<Option<f64>> = match self.model() {
            Some(model) => (0..model.n_items())
                .map(|position| aspect_of(model.item(position).and_downcast::<FileItem>().as_ref()))
                .collect(),
            None => Vec::new(),
        };
        self.imp().aspects.replace(aspects);
        self.relayout();
    }

    /// Sets the rows' height for `size`.
    pub(crate) fn set_icon_size(&self, size: IconSize) {
        let imp = self.imp();
        imp.icon_size.set(Some(nearest_named(size)));
        let height = row_height(size);
        if imp.row_height.replace(height) != height {
            self.relayout();
        }
    }

    pub(crate) fn icon_size(&self) -> IconSize {
        self.imp().icon_size.get().unwrap_or(IconSize::MEDIUM)
    }

    /// The layout for `width`, made again only when the width, the row
    /// height or the items changed.
    fn layout_for(&self, width: i32) -> std::cell::Ref<'_, Layout> {
        let imp = self.imp();
        let key = (width, imp.row_height.get());
        if imp.laid_for.get() != key {
            let layout = justify(&imp.aspects.borrow(), width, imp.row_height.get(), GALLERY_GAP);
            imp.layout.replace(layout);
            imp.laid_for.set(key);
        }
        imp.layout.borrow()
    }

    /// The layout at the canvas's width now.
    pub(crate) fn layout(&self) -> std::cell::Ref<'_, Layout> {
        self.layout_for(self.width().max(1))
    }

    /// The part of the canvas on screen: the viewport's scroll position
    /// and height.
    fn shown_span(&self) -> (f64, f64) {
        match self.vadjustment() {
            Some(adjustment) if adjustment.page_size() > 0.0 => (adjustment.value(), adjustment.page_size()),
            _ => (0.0, f64::from(self.height())),
        }
    }

    /// Binds tiles to the items of the rows near the part on screen and
    /// places them; tiles of rows further off are freed for others.
    fn place_tiles(&self, width: i32) {
        let (top, page) = self.shown_span();
        let beyond = page * ROWS_KEPT_BEYOND;
        let places: Vec<(usize, super::justified::Place)> = {
            let layout = self.layout_for(width);
            #[expect(clippy::cast_possible_truncation, reason = "layout coordinates are small")]
            let rows = layout.rows_between((top - beyond) as i32, (top + page + beyond) as i32);
            rows.flat_map(|row| layout.rows[row].start..layout.rows[row].end)
                .map(|item| (item, layout.places[item]))
                .collect()
        };
        let wanted: Vec<usize> = places.iter().map(|(item, _)| *item).collect();
        let imp = self.imp();
        let mut tiles = imp.tiles.borrow_mut();
        // Tiles already showing a wanted item keep it.
        let mut free: Vec<GalleryTile> = Vec::new();
        let mut bound: Vec<(usize, GalleryTile)> = Vec::new();
        for tile in tiles.drain(..) {
            match tile.slot().position().map(|position| position as usize) {
                Some(position) if wanted.binary_search(&position).is_ok() => bound.push((position, tile)),
                _ => free.push(tile),
            }
        }
        let rtl = self.direction() == gtk::TextDirection::Rtl;
        for (position, place) in &places {
            let tile = if let Some(index) = bound.iter().position(|(shown, _)| shown == position) {
                bound.swap_remove(index).1
            } else {
                let tile = free.pop().unwrap_or_else(|| self.new_tile());
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a listing has fewer than u32::MAX items"
                )]
                self.bind_tile(&tile, *position as u32);
                tile
            };
            let x = if rtl {
                width - place.x - place.width
            } else {
                place.x
            };
            tile.set_child_visible(true);
            #[expect(clippy::cast_precision_loss, reason = "layout coordinates are small")]
            let transform =
                gtk::gsk::Transform::new().translate(&graphene::Point::new(x as f32, place.y as f32));
            tile.allocate(place.width, place.height, -1, Some(transform));
            // Only tiles on screen get sharp pictures; those beside it
            // stop decoding (HOME-033).
            let on_screen = f64::from(place.y + place.height) > top && f64::from(place.y) < top + page;
            if on_screen {
                self.ask_sharp(&tile, place.width, place.height);
            } else {
                tile.cancel_pending_sharp();
            }
            tiles.push(tile);
        }
        for tile in free {
            if tile.slot().position().is_some() {
                Self::unbind_tile(&tile);
            }
            tile.set_child_visible(false);
            tiles.push(tile);
        }
        drop(tiles);
        if let Some(position) = imp.pending_reveal.take() {
            if width > 1 {
                glib::idle_add_local_once(glib::clone!(
                    #[weak(rename_to = canvas)]
                    self,
                    move || canvas.scroll_to_item(position)
                ));
            } else {
                imp.pending_reveal.set(Some(position));
            }
        }
    }

    fn new_tile(&self) -> GalleryTile {
        let tile = GalleryTile::new();
        tile.set_parent(self);
        self.owners().register_slot(tile.picture(), tile.slot());
        tile.set_has_tooltip(true);
        tile.connect_query_tooltip(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            #[upgrade_or]
            false,
            move |tile, _, _, _, tooltip| {
                // No tooltip over a menu, as in Windows.
                if canvas.popovers().iter().any(WidgetExt::is_visible) {
                    return false;
                }
                let Some(item) = tile.slot().item() else {
                    return false;
                };
                let text = canvas
                    .imp()
                    .tooltip_text
                    .borrow()
                    .as_ref()
                    .and_then(|text| text(&item));
                text.is_some_and(|text| {
                    tooltip.set_text(Some(&text));
                    true
                })
            }
        ));
        tile
    }

    /// Makes `tile` show the item at `position`.
    fn bind_tile(&self, tile: &GalleryTile, position: u32) {
        let Some(item) = self.item(position) else {
            Self::unbind_tile(tile);
            return;
        };
        tile.slot().show(Some((position, item.clone())));
        let owners = self.owners();
        owners.style_cell(tile.picture(), &item);
        tile.update_property(&[gtk::accessible::Property::Label(&owners.shown_name(&item))]);
        tile.set_selected(self.model().is_some_and(|model| model.is_selected(position)));
        self.show_focus_on(tile, position);
        self.ask_preview(tile, &item);
    }

    fn unbind_tile(tile: &GalleryTile) {
        tile.slot().show(None);
        tile.set_preview(None);
        tile.set_sharp(None);
        tile.sharp_asked().replace(None);
        tile.asked().replace(None);
        tile.picture().show(None);
        CellOwners::unstyle_cell(tile.picture());
    }

    /// Asks for `item`'s thumbnail at the size its row shows it.
    fn ask_preview(&self, tile: &GalleryTile, item: &FileItem) {
        let entry = item.entry();
        let edge = self.imp().row_height.get() * 2;
        let pixels = edge * self.scale_factor();
        let key = (entry.uri.clone(), entry.modified, pixels);
        if tile.asked().borrow().as_ref() == Some(&key) {
            return;
        }
        let same_photo = tile
            .asked()
            .borrow()
            .as_ref()
            .is_some_and(|(uri, modified, _)| *uri == entry.uri && *modified == entry.modified);
        tile.asked().replace(Some(key));
        if !same_photo {
            // Another photo: its own pictures, small and sharp, from the
            // start. The same photo at another size keeps what it draws,
            // so a View change never blanks or blurs it.
            tile.picture().show(None);
            tile.set_sharp(None);
            tile.sharp_asked().replace(None);
        }
        let Some(request) = self.owners().previews().request(entry, pixels) else {
            tile.set_preview(None);
            return;
        };
        let picture = tile.picture().downgrade();
        let task = glib::spawn_future_local(async move {
            let texture = crate::thumbnails::preview(request).await;
            if let (Some(picture), Some(texture)) = (picture.upgrade(), texture) {
                picture.offer(texture);
            }
        });
        tile.set_preview(Some(task));
    }

    /// Asks for `tile`'s photo decoded at least as large as the tile,
    /// `width` by `height` pixels, is drawn on this display (HOME-033):
    /// the cache's thumbnails are 128 or 256 pixels, which a tile of
    /// Medium icons and up draws blurred. A tile that grew asks again.
    fn ask_sharp(&self, tile: &GalleryTile, width: i32, height: i32) {
        let Some(item) = tile.slot().item() else {
            return;
        };
        let entry = item.entry();
        let scale = self.scale_factor();
        let pixels = (width * scale, height * scale);
        let asked_already = tile
            .sharp_asked()
            .borrow()
            .as_ref()
            .is_some_and(|(uri, asked)| *uri == entry.uri && *asked == pixels);
        if asked_already {
            return;
        }
        let edge = sharp_edge(width, height, aspect_of(Some(&item)), scale);
        let Some(request) = self.owners().previews().request(entry, edge) else {
            return;
        };
        tile.sharp_asked().replace(Some((entry.uri.clone(), pixels)));
        let weak = tile.downgrade();
        let task = glib::spawn_future_local(async move {
            let texture = crate::thumbnails::sharp_picture(request, pixels.0, pixels.1).await;
            let Some(tile) = weak.upgrade() else {
                return;
            };
            // Done: nothing is left to cancel.
            tile.finish_sharp();
            if let Some(texture) = texture {
                // The picture for the tile's size now, even when a larger
                // one, for a larger size before, was drawn.
                tile.picture().show(Some(texture));
            }
        });
        tile.set_sharp(Some(task));
    }

    /// Shows the selection on every tile on screen.
    fn show_selection(&self) {
        let Some(model) = self.model() else {
            return;
        };
        for tile in self.imp().tiles.borrow().iter() {
            if let Some(position) = tile.slot().position() {
                tile.set_selected(model.is_selected(position));
            }
        }
    }

    /// Rings the tile with the keyboard while the keyboard is on the
    /// canvas and the window shows where it is.
    fn show_keyboard_focus(&self) {
        for tile in self.imp().tiles.borrow().iter() {
            if let Some(position) = tile.slot().position() {
                self.show_focus_on(tile, position);
            }
        }
    }

    fn show_focus_on(&self, tile: &GalleryTile, position: u32) {
        let visible = self
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
            .is_some_and(|window| window.gets_focus_visible());
        let focused = self.has_focus() && visible && self.imp().focus.get() == Some(position);
        crate::window::widget_tree::toggle_class(tile, "keyboard-focus", focused);
    }

    /// The menus attached to the canvas.
    fn popovers(&self) -> Vec<gtk::Popover> {
        std::iter::successors(self.first_child(), WidgetExt::next_sibling)
            .filter_map(|child| child.downcast::<gtk::Popover>().ok())
            .collect()
    }

    /// The tiles bound to items now.
    #[cfg(test)]
    pub(crate) fn tiles(&self) -> Vec<GalleryTile> {
        self.imp()
            .tiles
            .borrow()
            .iter()
            .filter(|tile| tile.slot().position().is_some() && tile.is_child_visible())
            .cloned()
            .collect()
    }

    /// The item at (`x`, `y`) of the canvas.
    pub(crate) fn item_at(&self, x: f64, y: f64) -> Option<u32> {
        let width = self.width();
        let x = if self.direction() == gtk::TextDirection::Rtl {
            f64::from(width) - x
        } else {
            x
        };
        let item = self.layout().item_at(x, y)?;
        u32::try_from(item).ok()
    }

    /// The item with the keyboard.
    pub(crate) fn focused(&self) -> Option<u32> {
        self.imp().focus.get().filter(|focus| *focus < self.n_items())
    }

    /// Gives `position` the keyboard, scrolling to it unless `stay`.
    pub(crate) fn set_focused(&self, position: u32, stay: bool) {
        self.imp().focus.set(Some(position));
        if !stay {
            self.scroll_to_item(position);
        }
        self.show_keyboard_focus();
    }

    /// Makes `position` the item Shift extends a selection from.
    pub(crate) fn set_anchor(&self, position: u32) {
        self.imp().anchor.set(Some(position));
    }

    /// Scrolls so the row of `position` shows whole, if it does not.
    pub(crate) fn scroll_to_item(&self, position: u32) {
        let Some(adjustment) = self.vadjustment() else {
            return;
        };
        if self.width() <= 1 {
            self.imp().pending_reveal.set(Some(position));
            return;
        }
        let row = {
            let layout = self.layout();
            layout.row_of(position as usize).map(|index| layout.rows[index])
        };
        let Some(row) = row else {
            return;
        };
        let (top, page) = (adjustment.value(), adjustment.page_size());
        let (row_top, row_bottom) = (f64::from(row.y), f64::from(row.y + row.height));
        if row_top < top {
            adjustment.set_value(row_top);
        } else if row_bottom > top + page {
            adjustment.set_value((row_bottom - page).min(row_top));
        }
    }

    /// The positions whose tiles show at least half on screen.
    pub(crate) fn positions_shown(&self) -> Vec<u32> {
        let (top, page) = self.shown_span();
        let layout = self.layout();
        #[expect(clippy::cast_possible_truncation, reason = "layout coordinates are small")]
        let rows = layout.rows_between(top as i32, (top + page) as i32);
        rows.filter(|row| {
            let row = layout.rows[*row];
            let middle = f64::from(row.y) + f64::from(row.height) / 2.0;
            (top..top + page).contains(&middle)
        })
        .flat_map(|row| layout.rows[row].start..layout.rows[row].end)
        .filter_map(|item| u32::try_from(item).ok())
        .collect()
    }

    /// The newest and oldest photo dates on screen, for the heading.
    pub(crate) fn shown_dates(&self) -> (Option<u64>, Option<u64>) {
        let (top, page) = self.shown_span();
        let (first, last) = {
            let layout = self.layout();
            #[expect(clippy::cast_possible_truncation, reason = "layout coordinates are small")]
            let rows = layout.rows_between(top as i32, (top + page).ceil() as i32);
            if rows.is_empty() {
                return (None, None);
            }
            (layout.rows[rows.start].start, layout.rows[rows.end - 1].end - 1)
        };
        let date = |position: usize| {
            let item = self.item(u32::try_from(position).ok()?)?;
            photo_date(item.entry())
        };
        (date(first), date(last))
    }

    /// The months down the layout: each row's first photo's month, at
    /// the scroll position that brings the row to the top, as a fraction
    /// of the whole scroll; for the scrubber.
    pub(crate) fn timeline(&self) -> Vec<(f64, Option<u64>)> {
        let Some(adjustment) = self.vadjustment() else {
            return Vec::new();
        };
        let span = (adjustment.upper() - adjustment.page_size()).max(1.0);
        let starts: Vec<(i32, usize)> = {
            let layout = self.layout();
            layout.rows.iter().map(|row| (row.y, row.start)).collect()
        };
        starts
            .into_iter()
            .map(|(y, start)| {
                let date = u32::try_from(start)
                    .ok()
                    .and_then(|position| self.item(position))
                    .and_then(|item| photo_date(item.entry()));
                ((f64::from(y) / span).min(1.0), date)
            })
            .collect()
    }

    /// Runs `shown` whenever the part on screen or the items change.
    pub(crate) fn connect_shown(&self, shown: impl Fn() + 'static) {
        self.imp().on_shown.borrow_mut().push(Box::new(shown));
    }

    fn notify_shown(&self) {
        // Once GTK has laid the canvas out at its new size, and once for
        // many changes.
        if self.imp().shown_pending.replace(true) {
            return;
        }
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            move || {
                canvas.imp().shown_pending.set(false);
                for shown in canvas.imp().on_shown.borrow().iter() {
                    shown();
                }
            }
        ));
    }

    /// Runs `activate` with the position Enter or a double-click opens.
    pub(crate) fn connect_activate(&self, activate: impl Fn(u32) + 'static) {
        self.imp().on_activate.replace(Some(Box::new(activate)));
    }

    /// Makes the tiles' tooltips say `text` of their items.
    pub(crate) fn set_tooltip_text_for(&self, text: impl Fn(&FileItem) -> Option<String> + 'static) {
        self.imp().tooltip_text.replace(Some(Box::new(text)));
    }

    /// The tooltip of the item at `position`.
    #[cfg(test)]
    pub(crate) fn tooltip_for(&self, position: u32) -> Option<String> {
        let item = self.item(position)?;
        self.imp()
            .tooltip_text
            .borrow()
            .as_ref()
            .and_then(|text| text(&item))
    }

    fn activate_item(&self, position: u32) {
        if let Some(activate) = self.imp().on_activate.borrow().as_ref() {
            activate(position);
        }
    }

    /// Clicks and keys.
    fn install_input(&self) {
        let press = gtk::GestureClick::new();
        press.set_button(gdk::BUTTON_PRIMARY);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            move |press, presses, x, y| canvas.press(press, presses, x, y)
        ));
        press.connect_released(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_, _, _, _| {
                if let Some(position) = canvas.imp().pending_click.take() {
                    canvas.select_only(position);
                }
            }
        ));
        press.connect_cancel(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_, _| {
                canvas.imp().pending_click.take();
            }
        ));
        self.add_controller(press);
        let drag = gtk::GestureDrag::new();
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_, x, y| {
                if x.hypot(y) > 4.0 {
                    canvas.imp().pending_click.take();
                }
            }
        ));
        self.add_controller(drag);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| canvas.key(key, modifiers)
        ));
        self.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        focus.connect_contains_focus_notify(glib::clone!(
            #[weak(rename_to = canvas)]
            self,
            move |_| canvas.show_keyboard_focus()
        ));
        self.add_controller(focus);
    }

    fn press(&self, press: &gtk::GestureClick, presses: i32, x: f64, y: f64) {
        self.grab_focus();
        let Some(position) = self.item_at(x, y) else {
            return;
        };
        let Some(model) = self.model() else {
            return;
        };
        let modifiers = press.current_event_state();
        if presses == 2 {
            self.activate_item(position);
            return;
        }
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        self.imp().focus.set(Some(position));
        if shift {
            self.select_range_to(position, ctrl);
        } else if ctrl {
            if model.is_selected(position) {
                model.unselect_item(position);
            } else {
                model.select_item(position, false);
            }
            self.set_anchor(position);
        } else if model.is_selected(position) {
            // A drag of the selection may follow; a plain release selects
            // the item alone.
            self.imp().pending_click.set(Some(position));
            self.set_anchor(position);
        } else {
            self.select_only(position);
        }
        self.show_keyboard_focus();
    }

    /// Selects only `position` and makes it the anchor.
    pub(crate) fn select_only(&self, position: u32) {
        if let Some(model) = self.model() {
            model.select_item(position, true);
        }
        self.set_anchor(position);
    }

    /// Selects from the anchor to `position`, adding to the selection
    /// with `adding`.
    fn select_range_to(&self, position: u32, adding: bool) {
        let Some(model) = self.model() else {
            return;
        };
        let anchor = self.imp().anchor.get().unwrap_or(position);
        let (first, last) = (anchor.min(position), anchor.max(position));
        model.select_range(first, last - first + 1, !adding);
        self.imp().anchor.set(Some(anchor));
    }

    fn key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let count = self.n_items();
        if count == 0 {
            return glib::Propagation::Proceed;
        }
        let others =
            gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK | gdk::ModifierType::META_MASK;
        if modifiers.intersects(others) {
            return glib::Propagation::Proceed;
        }
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let current = self.focused();
        if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter) {
            if let Some(current) = current {
                self.activate_item(current);
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if key == gdk::Key::space && ctrl {
            if let (Some(current), Some(model)) = (current, self.model()) {
                if model.is_selected(current) {
                    model.unselect_item(current);
                } else {
                    model.select_item(current, false);
                }
                self.set_anchor(current);
            }
            return glib::Propagation::Stop;
        }
        let Some(target) = self.moved_to(key, current) else {
            return glib::Propagation::Proceed;
        };
        let target = target.min(count - 1);
        self.imp().focus.set(Some(target));
        if shift {
            self.select_range_to(target, ctrl);
        } else if !ctrl {
            self.select_only(target);
        }
        self.scroll_to_item(target);
        self.show_keyboard_focus();
        glib::Propagation::Stop
    }

    /// Where `key` moves the keyboard from `current`; `None` for a key
    /// that moves nothing.
    fn moved_to(&self, key: gdk::Key, current: Option<u32>) -> Option<u32> {
        let count = self.n_items();
        let rtl = self.direction() == gtk::TextDirection::Rtl;
        let Some(current) = current else {
            return matches!(
                key,
                gdk::Key::Left
                    | gdk::Key::Right
                    | gdk::Key::Up
                    | gdk::Key::Down
                    | gdk::Key::Home
                    | gdk::Key::End
                    | gdk::Key::Page_Up
                    | gdk::Key::Page_Down
                    | gdk::Key::KP_Left
                    | gdk::Key::KP_Right
                    | gdk::Key::KP_Up
                    | gdk::Key::KP_Down
            )
            .then(|| self.positions_shown().first().copied().unwrap_or(0));
        };
        let layout = self.layout();
        let index = current as usize;
        let forward = |forward: bool| {
            if forward == rtl {
                current.saturating_sub(1)
            } else {
                current + 1
            }
        };
        let moved = match key {
            gdk::Key::Right | gdk::Key::KP_Right => forward(true),
            gdk::Key::Left | gdk::Key::KP_Left => forward(false),
            gdk::Key::Up | gdk::Key::KP_Up => layout
                .item_beside_vertically(index, true)
                .and_then(|item| u32::try_from(item).ok())
                .unwrap_or(current),
            gdk::Key::Down | gdk::Key::KP_Down => layout
                .item_beside_vertically(index, false)
                .and_then(|item| u32::try_from(item).ok())
                .unwrap_or(current),
            gdk::Key::Home | gdk::Key::KP_Home => 0,
            gdk::Key::End | gdk::Key::KP_End => count - 1,
            gdk::Key::Page_Up | gdk::Key::Page_Down | gdk::Key::KP_Page_Up | gdk::Key::KP_Page_Down => {
                let down = matches!(key, gdk::Key::Page_Down | gdk::Key::KP_Page_Down);
                let (_, page) = self.shown_span();
                let place = layout.places.get(index).copied().unwrap_or_default();
                let y = f64::from(place.y) + if down { page } else { -page };
                let y = y.clamp(0.0, f64::from(layout.height.max(1) - 1));
                #[expect(clippy::cast_possible_truncation, reason = "layout coordinates are small")]
                let rows = layout.rows_between(y as i32, y as i32 + 1);
                let row = rows.start.min(layout.rows.len().saturating_sub(1));
                let row = layout.rows.get(row).copied();
                row.and_then(|row| {
                    (row.start..row.end).min_by(|a, b| {
                        let distance =
                            |item: &usize| (layout.places[*item].middle_x() - place.middle_x()).abs();
                        distance(a).total_cmp(&distance(b))
                    })
                })
                .and_then(|item| u32::try_from(item).ok())
                .unwrap_or(current)
            }
            _ => return None,
        };
        Some(moved.min(count - 1))
    }
}

mod view_imp {
    use super::{GalleryCanvas, GalleryScrubber, OnceCell};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::GalleryView`].
    #[derive(Debug, Default)]
    pub(crate) struct GalleryView {
        pub(super) canvas: OnceCell<GalleryCanvas>,
        pub(super) heading: OnceCell<gtk::Label>,
        pub(super) scroller: OnceCell<gtk::ScrolledWindow>,
        pub(super) scrubber: OnceCell<GalleryScrubber>,
        /// The month under the pointer on the scrubber.
        pub(super) hovered_month: OnceCell<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GalleryView {
        const NAME: &'static str = "OxGalleryView";
        type Type = super::GalleryView;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for GalleryView {}
    impl WidgetImpl for GalleryView {}
    impl BoxImpl for GalleryView {}
}

glib::wrapper! {
    /// Gallery's heading, tiles and scrubber.
    pub(crate) struct GalleryView(ObjectSubclass<view_imp::GalleryView>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl GalleryView {
    /// An empty view whose tiles `owners` registers.
    pub(crate) fn new(owners: &Rc<CellOwners>) -> Self {
        let view: Self = glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build();
        view.add_css_class("gallery-view");
        let heading = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["gallery-heading"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        // The heading names what shows, for a screen reader too.
        heading.set_accessible_role(gtk::AccessibleRole::Heading);
        view.append(&heading);
        let canvas = GalleryCanvas::new(owners);
        let tooltip_owners = Rc::downgrade(owners);
        canvas.set_tooltip_text_for(move |item| Some(photo_tooltip(&*tooltip_owners.upgrade()?, item)));
        let viewport = gtk::Viewport::builder()
            .child(&canvas)
            .scroll_to_focus(false)
            .vscroll_policy(gtk::ScrollablePolicy::Natural)
            .build();
        // The scrubber takes the scroll bar's place (HOME-028).
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&viewport)
            .hexpand(true)
            .vexpand(true)
            .build();
        let scrubber = GalleryScrubber::new();
        scrubber.set_halign(gtk::Align::End);
        scrubber.set_valign(gtk::Align::Fill);
        let overlay = gtk::Overlay::builder().child(&scroller).vexpand(true).build();
        overlay.add_overlay(&scrubber);
        let hovered_month = gtk::Label::builder()
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .can_target(false)
            .visible(false)
            .css_classes(["gallery-scrubber-month"])
            .build();
        overlay.add_overlay(&hovered_month);
        view.append(&overlay);
        canvas.follow_adjustment(&scroller.vadjustment());
        let imp = view.imp();
        imp.canvas.set(canvas.clone()).expect("new runs once");
        imp.heading.set(heading).expect("new runs once");
        imp.scroller.set(scroller.clone()).expect("new runs once");
        imp.scrubber.set(scrubber.clone()).expect("new runs once");
        imp.hovered_month
            .set(hovered_month.clone())
            .expect("new runs once");
        scrubber.connect_hover(glib::clone!(
            #[weak]
            hovered_month,
            #[weak]
            scrubber,
            move |shown| {
                let Some((month, y)) = shown else {
                    hovered_month.set_visible(false);
                    return;
                };
                hovered_month.set_text(&month);
                let (_, height) = hovered_month.preferred_size();
                // Beside the track, its middle at the pointer.
                #[expect(clippy::cast_possible_truncation, reason = "pixel positions")]
                let top = (y - f64::from(height.height()) / 2.0).round() as i32;
                hovered_month.set_margin_top(top.max(0));
                hovered_month.set_margin_end(scrubber.width() + 8);
                hovered_month.set_visible(true);
            }
        ));
        canvas.connect_shown(glib::clone!(
            #[weak]
            view,
            move || view.show_dates()
        ));
        scrubber.connect_jump(glib::clone!(
            #[weak]
            scroller,
            move |fraction| {
                let adjustment = scroller.vadjustment();
                let span = (adjustment.upper() - adjustment.page_size()).max(0.0);
                adjustment.set_value(fraction.clamp(0.0, 1.0) * span);
            }
        ));
        scrubber.connect_step(glib::clone!(
            #[weak]
            scroller,
            #[weak]
            canvas,
            move |up| {
                let adjustment = scroller.vadjustment();
                let step = f64::from(canvas.imp().row_height.get() + GALLERY_GAP);
                let value = adjustment.value() + if up { -step } else { step };
                adjustment
                    .set_value(value.clamp(adjustment.lower(), adjustment.upper() - adjustment.page_size()));
            }
        ));
        view
    }

    pub(crate) fn canvas(&self) -> &GalleryCanvas {
        self.imp().canvas.get().expect("new builds the canvas")
    }

    pub(crate) fn scrubber(&self) -> &GalleryScrubber {
        self.imp().scrubber.get().expect("new builds the scrubber")
    }

    /// The vertical scroll position's adjustment.
    pub(crate) fn vadjustment(&self) -> gtk::Adjustment {
        self.imp()
            .scroller
            .get()
            .expect("new builds the scroller")
            .vadjustment()
    }

    /// The month the scrubber shows under the pointer, while it shows.
    #[cfg(test)]
    pub(crate) fn hovered_month(&self) -> Option<String> {
        let label = self.imp().hovered_month.get()?;
        label.is_visible().then(|| label.text().to_string())
    }

    /// The heading over the photos, while it shows.
    #[cfg(test)]
    pub(crate) fn heading_text(&self) -> Option<String> {
        let heading = self.imp().heading.get()?;
        let text = heading.text();
        (heading.is_visible() && !text.is_empty()).then(|| text.to_string())
    }

    /// Shows the months on screen in the heading, and the timeline and
    /// position in the scrubber.
    fn show_dates(&self) {
        let canvas = self.canvas();
        let (newest, oldest) = canvas.shown_dates();
        let heading = self.imp().heading.get().expect("new builds the heading");
        let text = range_text(newest, oldest).unwrap_or_default();
        if heading.text() != text {
            heading.set_text(&text);
        }
        // The heading keeps its row while photos show, even when none on
        // screen has a date, so the photos never jump up and down.
        heading.set_visible(canvas.n_items() > 0);
        let timeline = canvas.timeline();
        let adjustment = self.vadjustment();
        let span = adjustment.upper() - adjustment.page_size();
        let position = if span > 0.0 {
            adjustment.value() / span
        } else {
            0.0
        };
        let scrubber = self.scrubber();
        scrubber.set_visible(timeline.len() > 1 && span > 0.0);
        scrubber.show_timeline(&timeline, position);
    }

    /// The pixels the tile of the photo called `name` draws, rendered by
    /// the window's renderer at the display's scale, as RGBA rows, with
    /// their width and height; for tests.
    #[cfg(test)]
    pub(crate) fn rendered_tile(&self, name: &str) -> Option<(i32, i32, Vec<u8>)> {
        let canvas = self.canvas();
        let tile = canvas
            .tiles()
            .into_iter()
            .find(|tile| tile.slot().item().is_some_and(|item| item.entry().name == name))?;
        let picture = tile.picture();
        let scale = picture.scale_factor();
        let (width, height) = (picture.width() * scale, picture.height() * scale);
        let paintable = gtk::WidgetPaintable::new(Some(picture));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
        let node = snapshot.to_node()?;
        let renderer = picture.native()?.renderer()?;
        #[expect(clippy::cast_precision_loss, reason = "tile sizes are small")]
        let bounds = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let texture = renderer.render_texture(node, Some(&bounds));
        let mut downloader = gdk::TextureDownloader::new(&texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let mut rows = Vec::with_capacity(usize::try_from(width * height * 4).ok()?);
        for row in 0..usize::try_from(height).ok()? {
            let start = row * stride;
            rows.extend_from_slice(&bytes[start..start + usize::try_from(width * 4).ok()?]);
        }
        Some((width, height, rows))
    }

    /// The tiles on screen with the names they show and their places in
    /// the canvas, for tests.
    #[cfg(test)]
    pub(crate) fn shown_tiles(&self) -> Vec<(String, graphene::Rect, TileState)> {
        let canvas = self.canvas();
        let (top, page) = canvas.shown_span();
        canvas
            .tiles()
            .into_iter()
            .filter_map(|tile| {
                let item = tile.slot().item()?;
                let bounds = tile.compute_bounds(canvas)?;
                let middle = f64::from(bounds.y() + bounds.height() / 2.0);
                (top..top + page).contains(&middle).then(|| {
                    (
                        item.entry().name.clone(),
                        bounds,
                        TileState {
                            thumbnail: tile.picture().shows_thumbnail(),
                            requested: tile.requested_pixels(),
                            drawn: tile.picture().texture_pixels(),
                        },
                    )
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The heading names one month, or the newest and oldest months with
    /// an en dash, as Windows' Gallery does.
    ///
    /// parity: HOME-027
    #[gtk::test]
    fn the_heading_names_the_newest_and_oldest_months() {
        let seconds = |year, month| {
            let date = glib::DateTime::from_local(year, month, 10, 12, 0, 0.0).expect("a date");
            u64::try_from(date.to_unix()).ok()
        };
        let november = month_text(seconds(2026, 11)).expect("a month");
        let july = month_text(seconds(2025, 7)).expect("a month");
        assert_eq!(
            range_text(seconds(2026, 11), seconds(2025, 7)),
            Some(format!("{november} – {july}"))
        );
        assert_eq!(range_text(seconds(2026, 11), seconds(2026, 11)), Some(november));
        assert_eq!(range_text(None, None), None);
    }

    /// Medium icons' rows are 160 pixels tall; the other named sizes are
    /// shorter or taller in order.
    ///
    /// parity: HOME-026
    #[test]
    fn rows_are_160_pixels_at_medium_icons() {
        assert_eq!(row_height(IconSize::MEDIUM), 160);
        let heights: Vec<i32> = IconSize::NAMED.into_iter().map(row_height).collect();
        assert!(heights.windows(2).all(|pair| pair[0] > pair[1]), "{heights:?}");
    }

    /// The decoded square covers the tile in device pixels, whatever the
    /// photo's shape, at 1x and 2x.
    ///
    /// parity: HOME-033
    #[test]
    fn the_sharp_picture_covers_its_tile_in_device_pixels() {
        // A 2:1 photo in a 320 by 160 tile: 320 across at 1x, 640 at 2x.
        assert_eq!(sharp_edge(320, 160, Some(2.0), 1), 320);
        assert_eq!(sharp_edge(320, 160, Some(2.0), 2), 640);
        // A phone screenshot, cropped to a wider tile: its height decides.
        let edge = sharp_edge(100, 160, Some(0.2), 1);
        assert!(f64::from(edge) * 0.2 >= 100.0, "{edge}");
        // A panorama cropped to a narrower tile: its width decides.
        let edge = sharp_edge(800, 160, Some(10.0), 2);
        assert!(f64::from(edge) / 10.0 >= 320.0, "{edge}");
        assert_eq!(sharp_edge(160, 160, None, 1), 160);
    }
}
