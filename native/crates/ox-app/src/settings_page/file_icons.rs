// SPDX-License-Identifier: AGPL-3.0-only
//! File icons (LOOK-029), a group of the Appearance page: the icon each
//! kind of item shows, for people who cannot tell a ZIP archive from a
//! folder, or one type from another, by its usual icon.
//!
//! A row for folders, ZIP archives and each type the colour icons show
//! has the icon its items show now, Change..., which offers the app's
//! pictures and Browse for a picture..., and Reset while another icon is
//! chosen. "File name endings" gives files that end a certain way, such
//! as `.iso` or `.tar.gz`, an icon of their own. Each choice is saved at
//! once in `fileIcons` and every window draws its items again
//! ([`crate::icons::file_icons`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::{file_name_ending, Preferences};

use super::group::SettingsGroup;
use super::parts;
use super::row::{ControlName, RowLayout, SettingRow};
use super::search::RowText;
use super::{SettingsPage, MESSAGE};
use crate::folder_view::icon_texture;
use crate::icons::file_icons::{ChosenIcon, ART_CHOICES};
use crate::icons::{self, Art, ArtImage, FileType, Icon};
use crate::window::ButtonStyle;

/// Names the kind a chooser saves its choice for, when there is one.
type KindOf = Rc<dyn Fn() -> Option<String>>;

/// The size the rows and the chooser show icons at.
const ICON_SIZE: i32 = 32;
/// The chevron of Change... (`.select .ico{width:12px}` in the mockup).
const CHEVRON_GLYPH: i32 = 12;

/// A kind of item the list offers: its `fileIcons` name, the art its
/// items show when no icon is chosen, and its row's words.
struct Kind {
    key: &'static str,
    usual: Art,
    text: RowText,
}

/// The kinds, in the list's order.
static KINDS: [Kind; 9] = [
    Kind {
        key: "folder",
        usual: Art::Folder,
        text: RowText {
            title: "Folder icon",
            description: "Every folder, unless you gave it its own icon in its Properties.",
            keywords: "file icons icon folder folders directory kind type change vision accessibility \
                       tell apart",
        },
    },
    Kind {
        key: "zip",
        usual: Art::ZipFolder,
        text: RowText {
            title: "ZIP archive icon",
            description: ".zip files, which look like folders unless you choose another icon.",
            keywords: "file icons icon zip archive archives compressed folder zipped kind type change \
                       vision accessibility tell apart",
        },
    },
    Kind {
        key: "document",
        usual: Art::File(FileType::Document),
        text: RowText {
            title: "Document icon",
            description: "PDFs, Word and OpenDocument files, slides, and files of no other type.",
            keywords: "file icons icon document documents pdf docx odt word kind type change",
        },
    },
    Kind {
        key: "text",
        usual: Art::File(FileType::Text),
        text: RowText {
            title: "Text file icon",
            description: ".txt, .md, .log and .rst files.",
            keywords: "file icons icon text txt markdown md log kind type change",
        },
    },
    Kind {
        key: "spreadsheet",
        usual: Art::File(FileType::Spreadsheet),
        text: RowText {
            title: "Spreadsheet icon",
            description: ".ods, .xlsx, .xls, .csv and .tsv files.",
            keywords: "file icons icon spreadsheet spreadsheets excel calc csv xlsx ods kind type change",
        },
    },
    Kind {
        key: "image",
        usual: Art::File(FileType::Image),
        text: RowText {
            title: "Picture icon",
            description: "Pictures that show no preview.",
            keywords: "file icons icon picture pictures image images photo png jpg kind type change",
        },
    },
    Kind {
        key: "video",
        usual: Art::File(FileType::Video),
        text: RowText {
            title: "Video icon",
            description: "Videos that show no preview.",
            keywords: "file icons icon video videos movie mp4 mkv kind type change",
        },
    },
    Kind {
        key: "audio",
        usual: Art::File(FileType::Audio),
        text: RowText {
            title: "Music icon",
            description: ".mp3, .flac, .ogg, .wav and other audio files.",
            keywords: "file icons icon music audio song mp3 flac kind type change",
        },
    },
    Kind {
        key: "code",
        usual: Art::File(FileType::Code),
        text: RowText {
            title: "Code icon",
            description: "Source code, HTML, JSON, XML and similar files.",
            keywords: "file icons icon code source script html json xml kind type change",
        },
    },
];

const ENDINGS: RowText = RowText {
    title: "File name endings",
    description: "Give files whose names end a certain way, such as .iso or .tar.gz, an icon of their \
                  own. It takes the place of their type's icon.",
    keywords: "file icons icon ending endings suffix name type custom add iso tar gz",
};

/// The File icons group.
pub(super) fn group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("File icons"));
    for kind in &KINDS {
        group.add_row(&kind_row(page, kind));
    }
    group.add_row(&endings_row(page));
    // Windows icon files, one more source of the same icons, after the
    // choices above, which win over them (LOOK-030).
    for row in super::appearance::icon_files_rows(page) {
        group.add_row(&row);
    }
    group
}

/// The row of `kind`: the icon its items show, Reset while another is
/// chosen, and Change....
fn kind_row(page: &SettingsPage, kind: &'static Kind) -> SettingRow {
    let row = SettingRow::new(kind.text);
    let shown = ShownIcon::new(kind.usual);
    let reset = parts::button(&ox_core::i18n::gettext("Reset"), ButtonStyle::Bordered);
    reset.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.save_file_icon(kind.key.to_owned(), None)
    ));
    let change = chooser(
        page,
        &ox_core::i18n::gettext("Change…"),
        Rc::new(|| Some(kind.key.to_owned())),
    );
    page.follow_preferences(glib::clone!(
        #[weak]
        reset,
        #[strong]
        shown,
        move |preferences| {
            let chosen = preferences.file_icons.get(kind.key);
            shown.show(chosen.map(String::as_str));
            reset.set_visible(chosen.is_some());
        }
    ));
    row.add_control(&shown.widget, ControlName::OwnLabel);
    row.add_control(&reset, ControlName::OwnLabel);
    row.add_control(&change, ControlName::OwnLabel);
    row
}

/// "File name endings": a line for each ending with an icon of its own,
/// then a box to type another and Choose icon....
fn endings_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(ENDINGS);
    row.set_roomy_layout(RowLayout::ControlsBelow);
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .css_classes(["file-icon-endings"])
        .build();
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.append(&lines);
    let entry = gtk::Entry::builder()
        .placeholder_text(ox_core::i18n::gettext("Ending, such as .iso"))
        .width_chars(18)
        .valign(gtk::Align::Center)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
        "File name ending",
    ))]);
    let typed = entry.downgrade();
    let add = chooser(
        page,
        &ox_core::i18n::gettext("Choose icon…"),
        Rc::new(move || typed.upgrade().and_then(|entry| file_name_ending(&entry.text()))),
    );
    add.set_sensitive(false);
    entry.connect_changed(glib::clone!(
        #[weak]
        add,
        move |entry| add.set_sensitive(file_name_ending(&entry.text()).is_some())
    ));
    let adding = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    adding.append(&entry);
    adding.append(&add);
    column.append(&adding);
    page.follow_preferences(glib::clone!(
        #[weak]
        page,
        #[weak]
        lines,
        #[weak]
        entry,
        move |preferences| {
            show_endings(&page, &lines, preferences);
            // An ending just given an icon has its own line now.
            if file_name_ending(&entry.text())
                .is_some_and(|ending| preferences.file_icons.contains_key(&ending))
            {
                entry.set_text("");
            }
        }
    ));
    row.add_control(&column, ControlName::OwnLabel);
    row
}

/// Fills `lines` with a line for each ending `preferences` give an icon:
/// the icon, the ending, Change... and Remove.
fn show_endings(page: &SettingsPage, lines: &gtk::Box, preferences: &Preferences) {
    while let Some(line) = lines.first_child() {
        lines.remove(&line);
    }
    let endings = preferences
        .file_icons
        .iter()
        .filter(|(ending, _)| ending.starts_with('.'));
    for (ending, icon) in endings {
        let line = gtk::Box::builder()
            .spacing(8)
            .css_classes(["file-icon-ending"])
            .build();
        let shown = ShownIcon::new(Art::File(FileType::Document));
        shown.show(Some(icon));
        line.append(&shown.widget);
        let name = gtk::Label::builder()
            .label(ending)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["file-icon-ending-name"])
            .build();
        line.append(&name);
        let key = ending.clone();
        let change = chooser(
            page,
            &ox_core::i18n::gettext("Change…"),
            Rc::new(move || Some(key.clone())),
        );
        change.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::format_message(
            "Change the icon of {ending} files",
            &[("ending", ending)],
        ))]);
        line.append(&change);
        let remove = parts::button(&ox_core::i18n::gettext("Remove"), ButtonStyle::Bordered);
        remove.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::format_message(
            "Remove the icon of {ending} files",
            &[("ending", ending)],
        ))]);
        let key = ending.clone();
        remove.connect_clicked(glib::clone!(
            #[weak]
            page,
            move |_| page.save_file_icon(key.clone(), None)
        ));
        line.append(&remove);
        lines.append(&line);
    }
}

/// The icon a row shows: the app's art, or a picture of the user's.
#[derive(Clone)]
struct ShownIcon {
    widget: gtk::Box,
    art: ArtImage,
    picture: gtk::Image,
    usual: Art,
    /// Counts what the row was asked to show, so a picture that loads
    /// after the row moved on to another icon is dropped.
    shown: Rc<Cell<u32>>,
}

impl ShownIcon {
    fn new(usual: Art) -> Self {
        // An image named for what it shows, so screen readers tell the
        // icon chosen now.
        let widget = gtk::Box::builder()
            .valign(gtk::Align::Center)
            .css_classes(["file-icon-shown"])
            .accessible_role(gtk::AccessibleRole::Img)
            .build();
        let art = ArtImage::new(usual, ICON_SIZE);
        let picture = gtk::Image::builder()
            .pixel_size(ICON_SIZE)
            .visible(false)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        widget.append(&art);
        widget.append(&picture);
        Self {
            widget,
            art,
            picture,
            usual,
            shown: Rc::default(),
        }
    }

    /// Shows the icon `fileIcons` holds as `icon`, or the usual art for
    /// `None` or an icon the app does not know.
    fn show(&self, icon: Option<&str>) {
        let chosen = icon.and_then(ChosenIcon::from_key);
        let name = match &chosen {
            Some(ChosenIcon::Picture(_)) => ox_core::i18n::gettext("A picture of yours"),
            Some(ChosenIcon::Art(art)) => art_name(*art),
            None => art_name(self.usual),
        };
        self.widget
            .update_property(&[gtk::accessible::Property::Label(&name)]);
        match chosen {
            Some(ChosenIcon::Picture(uri)) => {
                // The usual art until the picture is read, as the file list
                // reads it.
                self.show_art(self.usual);
                let asked = self.shown.get();
                let row = self.clone();
                glib::spawn_future_local(async move {
                    let texture = icon_texture(uri).await;
                    if let Some(texture) = texture.filter(|_| row.shown.get() == asked) {
                        row.picture.set_paintable(Some(&texture));
                        row.picture.set_visible(true);
                        row.art.set_visible(false);
                    }
                });
            }
            Some(ChosenIcon::Art(art)) => self.show_art(art),
            None => self.show_art(self.usual),
        }
    }

    fn show_art(&self, art: Art) {
        self.shown.set(self.shown.get().wrapping_add(1));
        self.art.set_art(art, ICON_SIZE);
        self.art.set_visible(true);
        self.picture.set_visible(false);
    }
}

/// The translated name of the app's picture `art`, as the chooser names it.
fn art_name(art: Art) -> String {
    ART_CHOICES
        .iter()
        .find(|choice| choice.art == art)
        .map(|choice| ox_core::i18n::gettext(choice.name))
        .unwrap_or_default()
}

/// A button called `label` whose popover offers the app's pictures and
/// Browse for a picture...; a choice is saved for the kind `kind` names
/// then, if any.
fn chooser(page: &SettingsPage, label: &str, kind: KindOf) -> gtk::MenuButton {
    // The bundled chevron, as the settings drop-downs show it: GTK's own
    // arrow comes from the desktop's icon theme.
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&gtk::Label::new(Some(label)));
    content.append(&icons::image(Icon::ChevronDown16, CHEVRON_GLYPH));
    let button = gtk::MenuButton::builder()
        .child(&content)
        .valign(gtk::Align::Center)
        .css_classes([
            "setting-button",
            ButtonStyle::Bordered.css_class(),
            "file-icon-change",
        ])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .css_classes(["file-icon-chooser"])
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(5)
        .max_children_per_line(5)
        .homogeneous(true)
        .row_spacing(4)
        .column_spacing(4)
        .build();
    let mut choices = Vec::new();
    for choice in ART_CHOICES {
        let name = ox_core::i18n::gettext(choice.name);
        let tile = gtk::Button::builder()
            .child(&ArtImage::new(choice.art, ICON_SIZE))
            .tooltip_text(&name)
            .css_classes(["file-icon-choice"])
            .build();
        tile.update_property(&[gtk::accessible::Property::Label(&name)]);
        let kind = Rc::clone(&kind);
        tile.connect_clicked(glib::clone!(
            #[weak]
            page,
            #[weak]
            popover,
            move |_| {
                popover.popdown();
                if let Some(kind) = kind() {
                    page.save_file_icon(kind, Some(choice.key.to_owned()));
                }
            }
        ));
        grid.append(&tile);
        choices.push((choice.key, tile.downgrade()));
    }
    content.append(&grid);
    let browse = parts::button(
        &ox_core::i18n::gettext("Browse for a picture…"),
        ButtonStyle::Bordered,
    );
    browse.add_css_class("file-icon-browse");
    let browsing = Rc::clone(&kind);
    browse.connect_clicked(glib::clone!(
        #[weak]
        page,
        #[weak]
        popover,
        move |_| {
            popover.popdown();
            if let Some(kind) = browsing() {
                page.browse_for_file_icon(kind);
            }
        }
    ));
    content.append(&browse);
    popover.set_child(Some(&content));
    // The icon chosen now is marked each time the chooser opens.
    let marking = kind;
    popover.connect_show(glib::clone!(
        #[weak]
        page,
        move |_| {
            let icons = page.context().settings_data().preferences.file_icons;
            let chosen = marking().and_then(|kind| icons.get(&kind).cloned());
            for (key, tile) in choices
                .iter()
                .filter_map(|(key, tile)| Some((key, tile.upgrade()?)))
            {
                let is_chosen = chosen.as_deref() == Some(*key);
                if is_chosen {
                    tile.add_css_class("chosen");
                } else {
                    tile.remove_css_class("chosen");
                }
                // Screen readers tell the chosen one by its state.
                let pressed = if is_chosen {
                    gtk::AccessibleTristate::True
                } else {
                    gtk::AccessibleTristate::False
                };
                tile.update_state(&[gtk::accessible::State::Pressed(pressed)]);
            }
        }
    ));
    button.set_popover(Some(&popover));
    button
}

thread_local! {
    /// How many icons were asked for each kind, so a picture still being
    /// read is not saved over an icon asked for after it.
    static ASKED: RefCell<HashMap<String, u32>> = RefCell::default();
}

/// Counts a new icon asked for `kind`, returning its number.
fn ask_for_icon(kind: &str) -> u32 {
    ASKED.with_borrow_mut(|asked| {
        let count = asked.entry(kind.to_owned()).or_default();
        *count = count.wrapping_add(1);
        *count
    })
}

impl SettingsPage {
    /// Gives the items of `kind` the icon `icon`, or their usual one with
    /// `None`; every window draws its items again once it is saved.
    fn save_file_icon(&self, kind: String, icon: Option<String>) {
        ask_for_icon(&kind);
        let page = self.downgrade();
        let change: crate::settings_store::Change =
            Box::new(move |settings| settings.set_file_icon(&kind, icon.as_deref()).map(|_| ()));
        self.context().change_settings(change, move |result| {
            let (Err(error), Some(page)) = (result, page.upgrade()) else {
                return;
            };
            let message = ox_core::i18n::format_message(
                "The icon could not be saved: {error}",
                &[("error", &error.to_string())],
            );
            page.emit_by_name::<()>(MESSAGE, &[&message]);
        });
    }

    /// Asks for a picture of the user's and makes it the icon of `kind`.
    fn browse_for_file_icon(&self, kind: String) {
        let images = gtk::FileFilter::new();
        images.set_name(Some(&ox_core::i18n::gettext("Images")));
        images.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&images);
        let dialog = gtk::FileDialog::builder()
            .title(ox_core::i18n::gettext("Choose an Icon"))
            .modal(true)
            .filters(&filters)
            .build();
        let window = self.root().and_downcast::<gtk::Window>();
        let page = self.downgrade();
        glib::spawn_future_local(async move {
            let Ok(chosen) = dialog.open_future(window.as_ref()).await else {
                return;
            };
            let Some(path) = chosen.path() else {
                return;
            };
            let uri = gio::File::for_path(path).uri().to_string();
            let saved = |page: &SettingsPage| {
                let icons = page.context().settings_data().preferences.file_icons;
                icons.get(&kind).cloned()
            };
            let Some(before) = page.upgrade().map(|page| saved(&page)) else {
                return;
            };
            let asked = ask_for_icon(&kind);
            // Only a picture the file list can show, read as it reads one.
            let readable = icon_texture(uri.clone()).await.is_some();
            // An icon asked for the kind while the picture was read wins,
            // here or saved by another window of the app.
            let current = ASKED.with_borrow(|asked| asked.get(&kind).copied());
            let Some(page) = page
                .upgrade()
                .filter(|page| current == Some(asked) && saved(page) == before)
            else {
                return;
            };
            if readable {
                page.save_file_icon(kind, Some(uri));
            } else {
                let message = ox_core::i18n::gettext("This picture cannot be used as an icon.");
                page.emit_by_name::<()>(MESSAGE, &[&message]);
            }
        });
    }
}
