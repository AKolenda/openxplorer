// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane: the selection's properties.
//!
//! Ports `renderDetails` in `v2.0.0:desktop/ui/app.js`, laid out as §4.8 of
//! `native/docs/ui-spec.md`: a header with a close button, a preview, the
//! name and type, an Open or "Pin to Quick access" button, a Properties
//! grid and a note. What the pane says is computed by [`pane_content`]
//! ([`content`]); this module draws it.
//!
//! [`DetailsPane`] is a widget subclass whose static tree is the template
//! `resources/ui/details-pane.ui`. This module adds what a template cannot
//! express: the glyphs, the window actions of the buttons and the
//! Properties rows, one per property shown.

mod content;
mod media;
mod options_menu;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use ox_core::settings::DetailsPaneOptions;

use crate::folder_view::item::FileItem;
use crate::icons::{self, Art, Icon};

use super::window_action::WindowAction;

pub(super) use content::{pane_content, PaneFacts};
use content::{PaneAction, PaneContent, Preview, Property};

/// Width of the pane (`.details` in `v2.0.0:desktop/ui/style.css`).
pub(super) const PANE_WIDTH: i32 = 262;

/// Preview size in the pane: app.js draws `fileIcon(e, 84)` and
/// `.detail-preview svg` shows it at 83 pixels.
const PREVIEW_SIZE: i32 = 83;

/// The copy glyph that stands for several selected items.
const SEVERAL_ITEMS_GLYPH: i32 = 80;

/// The glyphs in the pane's buttons and note.
const SMALL_GLYPH: i32 = 14;

/// The glyph of the header's close button.
const CLOSE_GLYPH: i32 = 12;

/// The width of the Properties names column
/// (`.detail-props{grid-template-columns:73px minmax(0,1fr)}`).
const PROPERTY_NAME_WIDTH: i32 = 73;
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
        /// The type line under the name.
        #[template_child]
        pub(super) kind: TemplateChild<gtk::Label>,
        /// Opens the one selected item.
        #[template_child]
        pub(super) open_button: TemplateChild<gtk::Button>,
        /// The glyph beside "Open".
        #[template_child]
        pub(super) open_glyph: TemplateChild<gtk::Image>,
        /// Pins the one selected folder.
        #[template_child]
        pub(super) pin_item_button: TemplateChild<gtk::Button>,
        /// The glyph of [`Self::pin_item_button`].
        #[template_child]
        pub(super) pin_item_glyph: TemplateChild<gtk::Image>,
        /// Pins the folder the tab shows.
        #[template_child]
        pub(super) pin_folder_button: TemplateChild<gtk::Button>,
        /// The glyph of [`Self::pin_folder_button`].
        #[template_child]
        pub(super) pin_folder_glyph: TemplateChild<gtk::Image>,
        /// The Properties grid.
        #[template_child]
        pub(super) properties: TemplateChild<gtk::Grid>,
        /// The info glyph before the note.
        #[template_child]
        pub(super) note_glyph: TemplateChild<gtk::Image>,
        /// The note at the bottom.
        #[template_child]
        pub(super) note: TemplateChild<gtk::Label>,
        /// Counts the content shown, so a preview read for an earlier
        /// selection is dropped.
        pub(super) media_generation: Cell<u64>,
        /// The content preview shown, which a redraw for the same item
        /// keeps (PROP-011).
        pub(super) shown_media: RefCell<Option<super::content::MediaPreview>>,
        /// The Properties rows the preview added: Dimensions, Length.
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
        icons::set_icon(&imp.open_glyph, Icon::Open, SMALL_GLYPH);
        icons::set_icon(&imp.pin_item_glyph, Icon::Pin, SMALL_GLYPH);
        icons::set_icon(&imp.pin_folder_glyph, Icon::Pin, SMALL_GLYPH);
        icons::set_icon(&imp.note_glyph, Icon::Info, SMALL_GLYPH);
    }

    /// Gives each button its window action; the close button switches the
    /// pane's toggle off.
    fn bind_actions(&self) {
        let imp = self.imp();
        WindowAction::DetailsPane.assign_to(&*imp.close_button);
        WindowAction::Open.assign_to(&*imp.open_button);
        WindowAction::PinSelected.assign_to(&*imp.pin_item_button);
        WindowAction::PinFolder.assign_to(&*imp.pin_folder_button);
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

    /// Shows `content`.
    pub(super) fn set_content(&self, content: &PaneContent) {
        let imp = self.imp();
        match content.preview {
            Preview::Art(art) => imp.preview.set_art(art, PREVIEW_SIZE),
            Preview::Several => imp.preview.set_art(Art::Glyph(Icon::Copy), SEVERAL_ITEMS_GLYPH),
        }
        imp.name.set_text(&content.name);
        imp.kind.set_text(&content.kind);
        self.show_buttons(content.action);
        self.show_properties(&content.properties);
        imp.note.set_text(content.note);
        self.show_media(content.media.as_ref());
    }

    /// Shows the buttons `action` offers and hides the others.
    fn show_buttons(&self, action: PaneAction) {
        let imp = self.imp();
        let offers_open = matches!(action, PaneAction::Open { .. });
        let offers_item_pin = action == PaneAction::Open { can_pin: true };
        let offers_folder_pin = action == PaneAction::PinFolder;
        imp.open_button.set_visible(offers_open);
        imp.pin_item_button.set_visible(offers_item_pin);
        imp.pin_folder_button.set_visible(offers_folder_pin);
    }

    fn show_properties(&self, properties: &[Property]) {
        let grid = &*self.imp().properties;
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        for property in properties {
            self.add_property(property.name, &property.value);
        }
    }

    /// Adds the row `name` with `value` under the others, unless the
    /// user turned the field off.
    fn add_property(&self, name: &str, value: &str) {
        if !self.imp().options.borrow().shows(name) {
            return;
        }
        let grid = &*self.imp().properties;
        let mut row = 0;
        while grid.child_at(NAME_COLUMN, row).is_some() {
            row += 1;
        }
        grid.attach(&property_name(name), NAME_COLUMN, row, 1, 1);
        grid.attach(&property_value(value), VALUE_COLUMN, row, 1, 1);
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

/// A Properties row as the pane shows it, for tests.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShownProperty {
    /// The name on the left, such as "Items".
    pub(super) name: String,
    /// The value on the right.
    pub(super) value: String,
}

/// A wrapping Properties label. Its natural width is a few words, so a
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
