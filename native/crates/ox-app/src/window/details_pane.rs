// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane: the selection's properties.
//!
//! Laid out as Windows 11 File Explorer's Details pane and §4.8 of
//! `native/docs/ui-spec.md`: a header with a close button, then a large
//! picture without a frame, the name in large text, the folder's item
//! count, and rows of names and values. It has no buttons: Open and Pin to
//! Quick access are in the item's and the folder's menus, as in Windows.
//! What the pane says is computed by [`pane_content`] ([`content`]); this
//! module draws it.
//!
//! [`DetailsPane`] is a widget subclass whose static tree is the template
//! `resources/ui/details-pane.ui`. This module adds what a template cannot
//! express: the close glyph and action, the stacked picture of several
//! items and the rows, one per property shown.

mod content;
mod media;
mod options_menu;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use ox_core::settings::DetailsPaneOptions;

use crate::folder_view::item::FileItem;
use crate::icons::{self, Art, ArtImage, Icon};

use super::window_action::WindowAction;

pub(super) use content::{pane_content, PaneFacts};

/// What the pane at the right shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum PaneMode {
    /// The Details pane: the preview, name and properties.
    #[default]
    Details,
    /// The Preview pane: the preview alone.
    Preview,
}
use content::{Field, PaneContent, Preview, Property};

/// Width of the pane (`.details` in `v2.0.0:desktop/ui/style.css`).
pub(super) const PANE_WIDTH: i32 = 262;

/// The size of the item's picture in the pane, Windows' large icon.
const PREVIEW_SIZE: i32 = 96;

/// The size of each picture stacked for several selected items.
const STACKED_SIZE: i32 = 80;

/// How far each picture of the stack sits from the one in front of it.
const STACK_STEP: i32 = 10;

/// The glyph of the header's close button.
const CLOSE_GLYPH: i32 = 12;

/// The width of the rows' names column, wide enough for "Date modified".
const PROPERTY_NAME_WIDTH: i32 = 92;
/// The Properties column of the names.
const NAME_COLUMN: i32 = 0;
/// The Properties column of the values.
const VALUE_COLUMN: i32 = 1;

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use ox_core::settings::DetailsPaneOptions;

    use crate::folder_view::item::FileItem;
    use crate::icons::ArtImage;

    /// Private state of [`super::DetailsPane`]: the template's widgets
    /// that change with the selection.
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/details-pane.ui")]
    pub(crate) struct DetailsPane {
        /// The pane's one child, around everything else. Bound so that
        /// `dispose_template` unparents it with the pane.
        #[template_child]
        pub(super) scroller: TemplateChild<gtk::ScrolledWindow>,
        /// The header's close button.
        #[template_child]
        pub(super) close_button: TemplateChild<gtk::Button>,
        /// The art or glyph at the top.
        #[template_child]
        pub(super) preview: TemplateChild<ArtImage>,
        /// The area around the art, which a content preview takes over
        /// (PROP-011).
        #[template_child]
        pub(super) preview_area: TemplateChild<gtk::CenterBox>,
        /// The item's or folder's name.
        #[template_child]
        pub(super) name: TemplateChild<gtk::Label>,
        /// The folder's item count under its name, shown only for the
        /// folder.
        #[template_child]
        pub(super) summary: TemplateChild<gtk::Label>,
        /// The rows of names and values.
        #[template_child]
        pub(super) properties: TemplateChild<gtk::Grid>,
        /// Counts the content shown, so a preview read for an earlier
        /// selection is dropped.
        pub(super) media_generation: Cell<u64>,
        /// The content preview shown, which a redraw for the same item
        /// keeps (PROP-011).
        pub(super) shown_media: RefCell<Option<super::content::MediaPreview>>,
        /// The Preview pane shows the item's icon in place of its
        /// contents (Folder Options' "Show preview handlers in preview
        /// pane" off).
        pub(super) hides_media: Cell<bool>,
        /// The rows the preview added: Dimensions, Length.
        pub(super) media_rows: RefCell<Vec<(super::Field, String)>>,
        /// The pane's options, as saved (PROP-010).
        pub(super) options: RefCell<DetailsPaneOptions>,
        /// Whether the pane shows the details or only the preview.
        pub(super) mode: std::cell::Cell<super::PaneMode>,
        /// The item under the pointer, which the pane describes while it
        /// follows the pointer.
        pub(super) hovered: RefCell<Option<FileItem>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DetailsPane {
        const NAME: &'static str = "OxDetailsPane";
        type Type = super::DetailsPane;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            ArtImage::ensure_type();
            klass.bind_template();
        }

        fn instance_init(pane: &glib::subclass::InitializingObject<Self>) {
            pane.init_template();
        }
    }

    impl ObjectImpl for DetailsPane {
        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "details-pane.ui");
            let pane = self.obj();
            pane.set_width(super::PANE_WIDTH);
            pane.show_glyphs();
            pane.bind_actions();
            pane.show_placeholder();
            pane.attach_options_menu();
            // A pane shown again catches up with the selection, whose
            // preview was not read while it was hidden.
            // It waits for the main loop: the pane is shown while the
            // window is laid out for a new width.
            pane.connect_visible_notify(|pane| {
                if !pane.is_visible() {
                    return;
                }
                let pane = pane.downgrade();
                glib::idle_add_local_once(move || {
                    let window = pane.upgrade().and_then(|pane| pane.root());
                    if let Some(window) = window.and_downcast::<super::super::BrowserWindow>() {
                        window.update_details_pane();
                    }
                });
            });
        }

        fn dispose(&self) {
            self.dispose_template();
        }
    }

    impl WidgetImpl for DetailsPane {}
}

glib::wrapper! {
    /// The details pane, shown beside the folder pane.
    pub(crate) struct DetailsPane(ObjectSubclass<imp::DetailsPane>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DetailsPane {
    /// Shows the close glyph the template leaves empty.
    fn show_glyphs(&self) {
        let close = icons::image(Icon::Dismiss16, CLOSE_GLYPH);
        self.imp().close_button.set_child(Some(&close));
    }

    /// The close button switches the pane's toggle off.
    fn bind_actions(&self) {
        WindowAction::DetailsPane.assign_to(&*self.imp().close_button);
    }

    /// Shows the folder art a pane shows before its first [`Self::set_content`].
    fn show_placeholder(&self) {
        self.imp().preview.set_art(Art::Folder, PREVIEW_SIZE);
    }

    /// Makes the pane `width` pixels wide ([`PANE_WIDTH`], or less in a
    /// narrower window).
    pub(super) fn set_width(&self, width: i32) {
        self.set_width_request(width);
    }

    /// The pane's options.
    pub(super) fn options(&self) -> DetailsPaneOptions {
        self.imp().options.borrow().clone()
    }

    /// Takes the saved `options`.
    pub(super) fn set_options(&self, options: DetailsPaneOptions) {
        self.imp().options.replace(options);
    }

    /// Takes `options` from the pane's menu, and has the window save them
    /// and redraw the pane.
    fn change_options(&self, options: DetailsPaneOptions) {
        self.set_options(options.clone());
        if !options.follow_hover {
            self.imp().hovered.replace(None);
        }
        if let Some(window) = self.root().and_downcast::<super::BrowserWindow>() {
            window.details_options_changed(options);
        }
    }

    /// Remembers the item under the pointer; true when it changed.
    pub(super) fn set_hovered(&self, item: Option<FileItem>) -> bool {
        let changed = *self.imp().hovered.borrow() != item;
        if changed {
            self.imp().hovered.replace(item);
        }
        changed
    }

    /// The item the pane describes instead of the selection: the one
    /// under the pointer, while it follows the pointer.
    pub(super) fn hovered(&self) -> Option<FileItem> {
        let following = self.imp().options.borrow().follow_hover;
        following.then(|| self.imp().hovered.borrow().clone()).flatten()
    }

    /// Shows the selection's details, or only its preview, as Windows 11's
    /// Details pane and Preview pane do in the same place (VIEW-070).
    pub(super) fn set_mode(&self, mode: PaneMode) {
        let imp = self.imp();
        imp.mode.set(mode);
        let (title, toggle, close) = match mode {
            PaneMode::Details => (
                ox_core::i18n::gettext("Details"),
                WindowAction::DetailsPane,
                ox_core::i18n::gettext("Close details pane"),
            ),
            PaneMode::Preview => (
                ox_core::i18n::gettext("Preview"),
                WindowAction::PreviewPane,
                ox_core::i18n::gettext("Close preview pane"),
            ),
        };
        if let Some(label) = self.title_label() {
            label.set_text(&title);
        }
        // The close button closes the pane that is open, and says which.
        toggle.assign_to(&*imp.close_button);
        imp.close_button.set_tooltip_text(Some(&close));
        imp.close_button
            .update_property(&[gtk::accessible::Property::Label(&close)]);
        self.show_mode();
    }

    /// Whether the pane shows the Details or only the preview.
    pub(super) fn mode(&self) -> PaneMode {
        self.imp().mode.get()
    }

    /// Makes the Preview pane show the item's icon in place of its
    /// contents with `hide`, as Windows does with "Show preview handlers
    /// in preview pane" off; true when that changed, so the content is
    /// shown again.
    pub(super) fn set_hides_media(&self, hide: bool) -> bool {
        self.imp().hides_media.replace(hide) != hide
    }

    /// The header's title, before the close button.
    fn title_label(&self) -> Option<gtk::Label> {
        self.imp()
            .close_button
            .prev_sibling()
            .and_downcast::<gtk::Label>()
    }

    /// Hides everything under the preview while only the preview shows;
    /// shows the name and the properties again otherwise. The item count
    /// follows the content.
    fn show_mode(&self) {
        let imp = self.imp();
        let preview_only = imp.mode.get() == PaneMode::Preview;
        let summary = imp.summary.upcast_ref::<gtk::Widget>();
        let mut below = imp.preview_area.next_sibling();
        while let Some(widget) = below {
            below = widget.next_sibling();
            if preview_only {
                widget.set_visible(false);
            } else if &widget == summary {
                widget.set_visible(!imp.summary.text().is_empty());
            } else {
                widget.set_visible(true);
            }
        }
    }

    /// Whether the pane shows the properties list, for tests.
    #[cfg(test)]
    pub(super) fn shows_properties(&self) -> bool {
        self.imp().properties.is_visible()
    }

    /// Whether the pane shows the preview, for tests.
    #[cfg(test)]
    pub(super) fn shows_preview(&self) -> bool {
        self.imp().preview_area.is_visible()
    }

    /// The header's title, for tests.
    #[cfg(test)]
    pub(super) fn title_text(&self) -> String {
        self.title_label()
            .map(|label| label.text().to_string())
            .unwrap_or_default()
    }

    /// The close button's tooltip, for tests.
    #[cfg(test)]
    pub(super) fn close_tooltip(&self) -> String {
        self.imp()
            .close_button
            .tooltip_text()
            .map(String::from)
            .unwrap_or_default()
    }

    /// Shows `content`.
    pub(super) fn set_content(&self, content: &PaneContent) {
        let imp = self.imp();
        imp.name.set_text(&content.name);
        imp.summary.set_text(&content.summary);
        imp.summary.set_visible(!content.summary.is_empty());
        self.show_properties(&content.properties);
        let preview_only = imp.mode.get() == PaneMode::Preview;
        let media = content
            .media
            .as_ref()
            .filter(|_| !(preview_only && imp.hides_media.get()));
        self.show_media(media);
        // After the media, which puts the art back when it has none.
        match &content.preview {
            Preview::Art(art) => {
                imp.preview.set_art(*art, PREVIEW_SIZE);
                if imp.shown_media.borrow().is_none() {
                    self.set_preview_widget(None);
                }
            }
            Preview::Stack(arts) => self.set_preview_widget(Some(&stacked_art(arts))),
        }
        if preview_only {
            self.show_mode();
        }
    }

    fn show_properties(&self, properties: &[Property]) {
        let grid = &*self.imp().properties;
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        for property in properties {
            self.add_property(property.field, &property.value);
        }
    }

    /// Adds the row of `field` with `value` under the others, unless the
    /// user turned the field off.
    fn add_property(&self, field: Field, value: &str) {
        if !self.imp().options.borrow().shows(field.key()) {
            return;
        }
        let grid = &*self.imp().properties;
        let mut row = 0;
        while grid.child_at(NAME_COLUMN, row).is_some() {
            row += 1;
        }
        grid.attach(&property_name(field.label()), NAME_COLUMN, row, 1, 1);
        grid.attach(&property_value(value), VALUE_COLUMN, row, 1, 1);
    }

    /// The item count under the folder's name, empty while hidden, for
    /// tests.
    #[cfg(test)]
    pub(super) fn shown_summary(&self) -> String {
        let summary = &self.imp().summary;
        if summary.is_visible() {
            summary.text().to_string()
        } else {
            String::new()
        }
    }

    /// The name shown under the preview, for tests.
    #[cfg(test)]
    pub(super) fn shown_name(&self) -> String {
        self.imp().name.text().to_string()
    }

    /// The property rows shown, top to bottom, for tests.
    #[cfg(test)]
    pub(super) fn shown_properties(&self) -> Vec<ShownProperty> {
        let mut shown = Vec::new();
        for row in 0.. {
            let name = self.property_text(NAME_COLUMN, row);
            let value = self.property_text(VALUE_COLUMN, row);
            let (Some(name), Some(value)) = (name, value) else {
                break;
            };
            shown.push(ShownProperty { name, value });
        }
        shown
    }

    /// The text in `column` of Properties row `row`, for tests.
    #[cfg(test)]
    fn property_text(&self, column: i32, row: i32) -> Option<String> {
        let label = self.imp().properties.child_at(column, row);
        let label = label.and_downcast::<gtk::Label>()?;
        Some(label.text().to_string())
    }
}

/// A row as the pane shows it, for tests.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShownProperty {
    /// The name on the left, such as "Type".
    pub(super) name: String,
    /// The value on the right.
    pub(super) value: String,
}

/// The pictures of several selected items, the first in front and each
/// next one behind it, up and to the right, as Windows stacks them.
fn stacked_art(arts: &[Art]) -> gtk::Widget {
    let behind = i32::try_from(arts.len().saturating_sub(1)).unwrap_or_default();
    let room = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let side = STACKED_SIZE + STACK_STEP * behind;
    room.set_size_request(side, side);
    let stack = gtk::Overlay::builder().child(&room).build();
    stack.add_css_class("detail-stack");
    let layers: Vec<(i32, &Art)> = (0..).zip(arts).collect();
    // Back to front: each picture is drawn over the ones behind it.
    for (depth, art) in layers.into_iter().rev() {
        let picture = ArtImage::new(*art, STACKED_SIZE);
        picture.set_halign(gtk::Align::Start);
        picture.set_valign(gtk::Align::Start);
        picture.set_margin_start(STACK_STEP * depth);
        picture.set_margin_top(STACK_STEP * (behind - depth));
        stack.add_overlay(&picture);
    }
    stack.upcast()
}

/// A wrapping row label. Its natural width is a few words, so a
/// long path wraps inside the pane instead of widening it.
fn property_label(text: &str, css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(10)
        .css_classes([css_class])
        .build()
}

/// A property's name in the left column.
fn property_name(name: &str) -> gtk::Label {
    let label = property_label(name, "detail-key");
    label.set_width_request(PROPERTY_NAME_WIDTH);
    label.set_yalign(0.0);
    label
}

/// A property's value, which can be selected and copied.
fn property_value(value: &str) -> gtk::Label {
    let label = property_label(value, "detail-value");
    label.set_selectable(true);
    label.set_hexpand(true);
    label
}
