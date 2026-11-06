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
//! Details rows, one per property shown. As in Windows, the pane has no
//! close button: View > Details pane, the command bar's Details toggle
//! and Alt+Shift+P show and hide it.

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

/// What the pane at the right shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum PaneMode {
    /// The Details pane: the preview, the name, Share, the Details rows
    /// and Properties.
    #[default]
    Details,
    /// The Preview pane: the preview alone.
    Preview,
}

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
        /// The art or glyph at the top.
        #[template_child]
        pub(super) preview: TemplateChild<ArtImage>,
        /// The area around the art, which a content preview takes over
        /// (PROP-011).
        #[template_child]
        pub(super) preview_area: TemplateChild<gtk::CenterBox>,
        /// The row of the small icon and the name.
        #[template_child]
        pub(super) title_row: TemplateChild<gtk::Box>,
        /// The small icon beside the name.
        #[template_child]
        pub(super) small_art: TemplateChild<ArtImage>,
        /// Windows' hint box, with nothing selected.
        #[template_child]
        pub(super) hint_row: TemplateChild<gtk::Box>,
        /// The (i) glyph of [`Self::hint_row`].
        #[template_child]
        pub(super) hint_glyph: TemplateChild<gtk::Image>,
        /// The "Details" heading over the rows.
        #[template_child]
        pub(super) details_heading: TemplateChild<gtk::Label>,
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
        /// The small icon shows beside the name.
        pub(super) small_icon: Cell<bool>,
        /// The hint box shows in place of the Details rows.
        pub(super) shows_hint: Cell<bool>,
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
        /// The Preview pane shows the item's icon in place of its
        /// contents (Folder Options' "Show preview handlers in preview
        /// pane" off).
        pub(super) hides_media: Cell<bool>,
        /// The Details rows the preview added: Dimensions, Length.
        pub(super) media_rows: RefCell<Vec<(&'static str, String)>>,
        /// The pane's options, as saved (PROP-010).
        pub(super) options: RefCell<DetailsPaneOptions>,
        /// Whether the pane shows the details or only the preview.
        pub(super) mode: Cell<super::PaneMode>,
        /// What the pane shows can have Properties opened.
        pub(super) has_properties: Cell<bool>,
        /// The pane's accessible name: "Details pane" or "Preview pane".
        pub(super) title: RefCell<String>,
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
            pane.set_mode(super::PaneMode::Details);
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
        icons::set_icon(&imp.share_glyph, Icon::Share, SMALL_GLYPH);
        // Windows' pane draws Properties with a wrench, as its "…" menu
        // does.
        icons::set_icon(&imp.properties_glyph, Icon::Wrench, SMALL_GLYPH);
        icons::set_icon(&imp.hint_glyph, Icon::Info, SMALL_GLYPH);
    }

    /// Gives each button its window action.
    fn bind_actions(&self) {
        let imp = self.imp();
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

    /// Shows the selection's details, or only its preview, as Windows 11's
    /// Details pane and Preview pane do in the same place (VIEW-070).
    /// Like Windows' panes, neither has a heading or a close button: its
    /// accessible name says which pane it is.
    pub(super) fn set_mode(&self, mode: PaneMode) {
        let imp = self.imp();
        imp.mode.set(mode);
        let title = match mode {
            PaneMode::Details => ox_core::i18n::gettext("Details pane"),
            PaneMode::Preview => ox_core::i18n::gettext("Preview pane"),
        };
        self.update_property(&[gtk::accessible::Property::Label(&title)]);
        imp.title.replace(title);
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

    /// Shows what the mode and the content call for under the preview:
    /// nothing in the Preview pane; in the Details pane the name (with the
    /// small icon for a file), then Share, the Details rows and
    /// Properties as the selection allows, or the hint box with nothing
    /// selected.
    fn show_mode(&self) {
        let imp = self.imp();
        let details = imp.mode.get() == PaneMode::Details;
        let hint = imp.shows_hint.get();
        imp.title_row.set_visible(details);
        imp.small_art.set_visible(imp.small_icon.get());
        imp.hint_row.set_visible(details && hint);
        imp.details_heading.set_visible(details && !hint);
        imp.properties.set_visible(details && !hint);
        self.show_share();
        imp.properties_button
            .set_visible(details && imp.has_properties.get());
    }

    /// Whether the pane shows the Details rows, for tests.
    #[cfg(test)]
    pub(super) fn shows_properties(&self) -> bool {
        self.imp().properties.is_visible()
    }

    /// Whether the pane shows the preview, for tests.
    #[cfg(test)]
    pub(super) fn shows_preview(&self) -> bool {
        self.imp().preview_area.is_visible()
    }

    /// The pane's accessible name, for tests.
    #[cfg(test)]
    pub(super) fn title_text(&self) -> String {
        self.imp().title.borrow().clone()
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
        imp.has_selection
            .set(content.subject == content::Subject::Selection);
        imp.has_properties.set(content.has_properties);
        imp.small_icon.set(content.small_icon);
        imp.shows_hint.set(content.subject == content::Subject::Folder);
        self.show_mode();
        imp.base_rows.replace(content.properties.clone());
        self.follow_metadata(content.metadata.as_ref());
        self.redraw_properties();
        let preview_only = imp.mode.get() == PaneMode::Preview;
        let media = content
            .media
            .as_ref()
            .filter(|_| !(preview_only && imp.hides_media.get()));
        self.show_media(media);
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
        let details = imp.mode.get() == PaneMode::Details;
        imp.share_button
            .set_visible(details && imp.share_offered.get() && imp.has_selection.get());
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

    /// Whether the hint box shows, for tests.
    #[cfg(test)]
    pub(super) fn shows_hint(&self) -> bool {
        self.imp().hint_row.is_visible()
    }

    /// The hint box's text, for tests.
    #[cfg(test)]
    pub(super) fn hint_text(&self) -> String {
        crate::window::widget_tree::descendants::<gtk::Label>(&*self.imp().hint_row)
            .iter()
            .map(|label| label.text().to_string())
            .collect()
    }

    /// Whether the Properties button shows, for tests.
    #[cfg(test)]
    pub(super) fn shows_properties_button(&self) -> bool {
        self.imp().properties_button.is_visible()
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

/// A property's value, which the pointer can select and copy. It is no
/// Tab stop, as in Windows' pane, whose keyboard stops are its buttons:
/// a selectable label that takes the keyboard while focus moves on its
/// own (after a menu closes, say) has GTK 4.14 compare it with the widget
/// that had the keyboard, which can be gone by then
/// (`gtk_label_grab_focus`), a critical that tests make fatal. Not
/// `focusable` alone: GTK still offers a Tab to a selectable label, which
/// then claims it without taking the keyboard; without `can-focus` Tab
/// passes it by.
fn property_value(value: &str) -> gtk::Label {
    let label = property_label(value, "detail-value");
    label.set_selectable(true);
    label.set_focusable(false);
    label.set_can_focus(false);
    label.set_hexpand(true);
    label
}
