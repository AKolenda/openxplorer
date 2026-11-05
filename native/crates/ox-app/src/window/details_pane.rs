// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane, laid out as Windows 11 Explorer's Details pane
//! (View > Show > Details pane, Alt+Shift+P; PROP-009).
//!
//! From top to bottom: a large preview of the selection (the picture
//! itself for a photo, a document's thumbnail, a player for a recording,
//! else the large icon), its small icon and name, the Share button, a
//! "Details" heading over the name and value rows, and the Properties
//! button. What the pane says is computed by [`pane_content`]
//! ([`content`]); a photo's, video's or audio file's own rows are read by
//! Properties' metadata reader ([`metadata`]) without holding up the
//! rest; the content preview is [`media`]. This module draws it.
//!
//! [`DetailsPane`] is a widget subclass whose static tree is the template
//! `resources/ui/details-pane.ui`. This module adds what a template cannot
//! express: the glyphs, the window actions of the buttons and the
//! Details rows, one per property shown.

mod content;
mod media;
mod metadata;
mod options_menu;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use ox_core::settings::DetailsPaneOptions;

use crate::folder_view::item::FileItem;
use crate::icons::{self, Art, Icon};

use super::window_action::WindowAction;

pub(super) use content::{pane_content, PaneFacts};
use content::{PaneContent, Preview};

/// Width of the pane (`.details` in `v2.0.0:desktop/ui/style.css`).
pub(super) const PANE_WIDTH: i32 = 262;

/// The large icon of the preview, when there is no picture of the
/// content: Windows shows its jumbo icon there.
const PREVIEW_SIZE: i32 = 96;

/// The copy glyph that stands for several selected items.
const SEVERAL_ITEMS_GLYPH: i32 = 80;

/// The small icon beside the name.
const SMALL_ART: i32 = 20;

/// The glyphs in the pane's buttons.
const SMALL_GLYPH: i32 = 16;

/// The glyph of the close button.
const CLOSE_GLYPH: i32 = 12;

/// The width of the names column, so every value starts at one line.
const PROPERTY_NAME_WIDTH: i32 = 88;
/// The Details column of the names.
const NAME_COLUMN: i32 = 0;
/// The Details column of the values.
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
        /// The close button above the preview.
        #[template_child]
        pub(super) close_button: TemplateChild<gtk::Button>,
        /// The art or glyph at the top.
        #[template_child]
        pub(super) preview: TemplateChild<ArtImage>,
        /// The area around the art, which a content preview takes over
        /// (PROP-011).
        #[template_child]
        pub(super) preview_area: TemplateChild<gtk::CenterBox>,
        /// The small icon beside the name.
        #[template_child]
        pub(super) small_art: TemplateChild<ArtImage>,
        /// The item's name, "N items selected" or "N items".
        #[template_child]
        pub(super) name: TemplateChild<gtk::Label>,
        /// Shares the selected files (win.share).
        #[template_child]
        pub(super) share_button: TemplateChild<gtk::Button>,
        /// The glyph of [`Self::share_button`].
        #[template_child]
        pub(super) share_glyph: TemplateChild<gtk::Image>,
        /// The Details rows.
        #[template_child]
        pub(super) properties: TemplateChild<gtk::Grid>,
        /// Opens Properties (win.properties).
        #[template_child]
        pub(super) properties_button: TemplateChild<gtk::Button>,
        /// The glyph of [`Self::properties_button`].
        #[template_child]
        pub(super) properties_glyph: TemplateChild<gtk::Image>,
        /// The window can share the selection.
        pub(super) share_offered: Cell<bool>,
        /// Some item is selected, which Share would share.
        pub(super) has_selection: Cell<bool>,
        /// The file rows of the content shown.
        pub(super) base_rows: RefCell<Vec<super::content::Property>>,
        /// The selected file's own details, read by the reader.
        pub(super) metadata: super::metadata::MetadataState,
        /// Counts the content shown, so a preview read for an earlier
        /// selection is dropped.
        pub(super) media_generation: Cell<u64>,
        /// The content preview shown, which a redraw for the same item
        /// keeps (PROP-011).
        pub(super) shown_media: RefCell<Option<super::content::MediaPreview>>,
        /// The Details rows the preview added: Dimensions, Length.
        pub(super) media_rows: RefCell<Vec<(&'static str, String)>>,
        /// The pane's options, as saved (PROP-010).
        pub(super) options: RefCell<DetailsPaneOptions>,
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
    /// Shows the glyphs the template leaves empty.
    fn show_glyphs(&self) {
        let imp = self.imp();
        let close = icons::image(Icon::Dismiss16, CLOSE_GLYPH);
        imp.close_button.set_child(Some(&close));
        icons::set_icon(&imp.share_glyph, Icon::Share, SMALL_GLYPH);
        icons::set_icon(&imp.properties_glyph, Icon::Info, SMALL_GLYPH);
    }

    /// Gives each button its window action; the close button switches the
    /// pane's toggle off.
    fn bind_actions(&self) {
        let imp = self.imp();
        WindowAction::DetailsPane.assign_to(&*imp.close_button);
        WindowAction::Share.assign_to(&*imp.share_button);
        WindowAction::Properties.assign_to(&*imp.properties_button);
    }

    /// Shows the folder art a pane shows before its first [`Self::set_content`].
    fn show_placeholder(&self) {
        let imp = self.imp();
        imp.preview.set_art(Art::Folder, PREVIEW_SIZE);
        imp.small_art.set_art(Art::Folder, SMALL_ART);
        imp.share_button.set_visible(false);
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

    /// Shows `content`.
    pub(super) fn set_content(&self, content: &PaneContent) {
        let imp = self.imp();
        match content.preview {
            Preview::Art(art) => imp.preview.set_art(art, PREVIEW_SIZE),
            Preview::Several => imp.preview.set_art(Art::Glyph(Icon::Copy), SEVERAL_ITEMS_GLYPH),
        }
        imp.small_art.set_art(content.preview.art(), SMALL_ART);
        imp.name.set_text(&content.name);
        imp.has_selection.set(content.has_selection);
        self.show_share();
        imp.properties_button.set_visible(content.has_properties);
        imp.base_rows.replace(content.properties.clone());
        self.follow_metadata(content.metadata.as_ref());
        self.redraw_properties();
        self.show_media(content.media.as_ref());
    }

    /// Shows Share while the window can share the selection (CMD-037):
    /// the desktop can share, and the selection is files it can attach.
    /// Unlike the command bar, which keeps a disabled Share, the pane
    /// leaves it out for a folder.
    pub(super) fn offer_share(&self, offered: bool) {
        self.imp().share_offered.set(offered);
        self.show_share();
    }

    fn show_share(&self) {
        let imp = self.imp();
        imp.share_button
            .set_visible(imp.share_offered.get() && imp.has_selection.get());
    }

    /// Draws the Details rows again: the file rows, then the rows the
    /// reader found, then those the preview found (an image's Dimensions,
    /// a recording's Length) that the reader did not give.
    pub(super) fn redraw_properties(&self) {
        let imp = self.imp();
        let grid = &*imp.properties;
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        let mut shown: Vec<&str> = Vec::new();
        for property in imp.base_rows.borrow().iter() {
            self.add_property(property.key, &property.name, &property.value);
            shown.push(property.key);
        }
        for (key, name, value) in imp.metadata.rows() {
            if !shown.contains(&key) {
                self.add_property(key, &name, &value);
                shown.push(key);
            }
        }
        for (key, value) in imp.media_rows.borrow().iter() {
            if !shown.contains(key) {
                self.add_property(key, ox_core::i18n::gettext_static(key), value);
            }
        }
    }

    /// Adds the row `name` with `value` under the others, unless the
    /// user turned off the field `key`.
    fn add_property(&self, key: &str, name: &str, value: &str) {
        if !self.imp().options.borrow().shows(key) {
            return;
        }
        let grid = &*self.imp().properties;
        let mut row = 0;
        while grid.child_at(NAME_COLUMN, row).is_some() {
            row += 1;
        }
        let name_label = property_name(name);
        let value_label = property_value(value);
        value_label.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::format_message(
            "{name}: {value}",
            &[("name", name), ("value", value)],
        ))]);
        grid.attach(&name_label, NAME_COLUMN, row, 1, 1);
        grid.attach(&value_label, VALUE_COLUMN, row, 1, 1);
    }

    /// The name shown under the preview, for tests.
    #[cfg(test)]
    pub(super) fn shown_name(&self) -> String {
        self.imp().name.text().to_string()
    }

    /// Whether Share shows in the pane, for tests.
    #[cfg(test)]
    pub(super) fn shows_share(&self) -> bool {
        self.imp().share_button.is_visible()
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

/// A Details row as the pane shows it, for tests.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShownProperty {
    /// The name on the left, such as "Items".
    pub(super) name: String,
    /// The value on the right.
    pub(super) value: String,
}

/// A wrapping Details label. Its natural width is a few words, so a
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
