// SPDX-License-Identifier: AGPL-3.0-only
//! Change Icon... (PROP-016): Windows' "Change Icon for <folder> Folder"
//! dialog. "Select an icon from the list below:" over a grid of the icon
//! theme's folder icons, where Windows lists those of a system file, then
//! Browse... for a picture of the user's own, which joins the grid
//! chosen, and OK and Cancel. OK, or a double click on an icon, hands the
//! icon chosen to the Customize tab, which applies it on its own OK or
//! Apply.
//!
//! The folder keeps its icon as the location of a picture
//! (`metadata::custom-icon`, which Files reads too), so the grid offers a
//! theme icon only when the theme has it as a file, and never its
//! `-symbolic` glyphs.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::window::{page, push_button, ApplyFuture, Pending, PropertiesWindow};

/// The size of an icon in the grid, as Windows' Change Icon shows them.
const ICON_SIZE: i32 = 32;
/// How many rows of icons show at once.
const ROWS_SHOWN: i32 = 4;

/// The icons the grid offers, by their location, and the one chosen.
struct IconGrid {
    grid: gtk::FlowBox,
    /// Each icon's location, in the grid's order.
    uris: RefCell<Vec<String>>,
    /// The icon chosen when the dialog opened.
    initial: Option<String>,
    /// Gives the Customize tab the icon OK chose.
    chosen: Box<dyn Fn(String)>,
}

impl IconGrid {
    /// The location of the icon selected, if any.
    fn selected(&self) -> Option<String> {
        let child = self.grid.selected_children().into_iter().next()?;
        let index = usize::try_from(child.index()).ok()?;
        self.uris.borrow().get(index).cloned()
    }

    /// Adds `image`, the icon at `uri` called `name`, at `position`.
    fn insert(&self, image: &gtk::Image, uri: String, name: &str, position: usize) -> gtk::FlowBoxChild {
        image.set_pixel_size(ICON_SIZE);
        let tile = gtk::FlowBoxChild::builder()
            .child(image)
            .tooltip_text(name)
            .build();
        tile.update_property(&[gtk::accessible::Property::Label(name)]);
        let at = i32::try_from(position).unwrap_or(i32::MAX);
        self.grid.insert(&tile, at);
        self.uris.borrow_mut().insert(position, uri);
        tile
    }

    /// Adds the picture at `uri` first, chosen.
    fn add_picture(&self, uri: String) {
        let file = gio::File::for_uri(&uri);
        let name = file
            .basename()
            .map_or_else(|| uri.clone(), |name| name.display().to_string());
        let image = gtk::Image::from_gicon(&gio::FileIcon::new(&file));
        let tile = self.insert(&image, uri, &name, 0);
        self.grid.select_child(&tile);
    }
}

impl Pending for IconGrid {
    fn has_changes(&self) -> bool {
        self.selected()
            .is_some_and(|uri| Some(&uri) != self.initial.as_ref())
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move {
            if let Some(uri) = self.selected() {
                (self.chosen)(uri);
            }
            Ok(())
        })
    }
}

/// Whether the theme icon `name` is one of its folder pictures.
fn is_folder_icon(name: &str) -> bool {
    (name == "folder" || name.starts_with("folder-")) && !name.ends_with("-symbolic")
}

/// The theme's folder icons that are files, as `(name, icon, location)`,
/// each file once, `folder` first.
fn theme_folder_icons(widget: &impl IsA<gtk::Widget>) -> Vec<(String, gtk::IconPaintable, String)> {
    let theme = gtk::IconTheme::for_display(&widget.display());
    let mut names: Vec<String> = theme
        .icon_names()
        .iter()
        .map(ToString::to_string)
        .filter(|name| is_folder_icon(name))
        .collect();
    names.sort();
    names.dedup();
    let mut seen = std::collections::HashSet::new();
    names
        .into_iter()
        .filter_map(|name| {
            let icon = theme.lookup_icon(
                &name,
                &[],
                ICON_SIZE,
                widget.scale_factor().max(1),
                gtk::TextDirection::None,
                gtk::IconLookupFlags::empty(),
            );
            let file = icon.file().filter(|file| file.path().is_some())?;
            let uri = file.uri().to_string();
            seen.insert(uri.clone()).then_some((name, icon, uri))
        })
        .collect()
}

/// Opens Change Icon for the folder called `folder` over `parent`, with
/// the icon `current` chosen; OK runs `chosen` with the icon chosen.
pub(super) fn open(
    parent: &PropertiesWindow,
    folder: &str,
    current: Option<&str>,
    chosen: impl Fn(String) + 'static,
) -> PropertiesWindow {
    let title = ox_core::i18n::format_message("Change Icon for {folder} Folder", &[("folder", folder)]);
    let dialog = PropertiesWindow::dialog_of(parent, &title);
    dialog.hide_apply();
    let content = page();
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .activate_on_single_click(false)
        .homogeneous(true)
        .min_children_per_line(4)
        .max_children_per_line(16)
        .valign(gtk::Align::Start)
        .css_classes(["properties-icon-grid"])
        .build();
    let caption = gtk::Label::builder()
        .label(ox_core::i18n::gettext("_Select an icon from the list below:"))
        .use_underline(true)
        .mnemonic_widget(&grid)
        .xalign(0.0)
        .css_classes(["properties-caption"])
        .build();
    content.append(&caption);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&grid)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(ROWS_SHOWN * (ICON_SIZE + 16))
        .build();
    let frame = gtk::Frame::builder()
        .child(&scroller)
        .css_classes(["properties-list-frame"])
        .build();
    content.append(&frame);
    let browse = push_button(&ox_core::i18n::gettext("Browse..."));
    let buttons = gtk::Box::builder().css_classes(["properties-button-row"]).build();
    buttons.append(&browse);
    content.append(&buttons);
    dialog.add_tab("change-icon", &title, &content);

    let icons = Rc::new(IconGrid {
        grid: grid.clone(),
        uris: RefCell::new(Vec::new()),
        initial: current.map(str::to_owned),
        chosen: Box::new(chosen),
    });
    for (name, icon, uri) in theme_folder_icons(parent) {
        let image = gtk::Image::from_paintable(Some(&icon));
        let position = icons.uris.borrow().len();
        let tile = icons.insert(&image, uri.clone(), &name, position);
        if Some(uri.as_str()) == current {
            grid.select_child(&tile);
        }
    }
    if let Some(current) = current.filter(|current| !icons.uris.borrow().iter().any(|uri| uri == current)) {
        icons.add_picture(current.to_owned());
    }
    let watched = dialog.downgrade();
    grid.connect_selected_children_changed(move |_| {
        if let Some(dialog) = watched.upgrade() {
            dialog.changed();
        }
    });
    // A double click or Enter on an icon chooses it, as OK does.
    let accepted = dialog.downgrade();
    grid.connect_child_activated(move |_, _| {
        if let Some(dialog) = accepted.upgrade() {
            dialog.accept();
        }
    });
    let browsing = Rc::clone(&icons);
    browse.connect_clicked(move |button| browse_for_picture(button, &browsing));
    dialog.add_pending(icons);
    dialog.present();
    dialog
}

/// Browse...: asks for a picture, which joins the grid chosen.
fn browse_for_picture(button: &gtk::Button, icons: &Rc<IconGrid>) {
    let images = gtk::FileFilter::new();
    images.set_name(Some(&ox_core::i18n::gettext("Images")));
    images.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&images);
    let dialog = gtk::FileDialog::builder()
        .title(ox_core::i18n::gettext("Change Icon"))
        .modal(true)
        .filters(&filters)
        .build();
    let window = button.root().and_downcast::<gtk::Window>();
    let icons = Rc::clone(icons);
    glib::spawn_future_local(async move {
        if let Ok(chosen) = dialog.open_future(window.as_ref()).await {
            icons.add_picture(chosen.uri().to_string());
        }
    });
}
