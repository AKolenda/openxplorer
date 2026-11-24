// SPDX-License-Identifier: AGPL-3.0-only
//! The General and Hardware tabs of a drive, laid out as Windows 11's
//! (PROP-004).
//!
//! A drive is a folder of the kernel's mount table: Local Disk, a USB
//! stick, a disc or a share mounted by the system. Its General tab shows
//! the drive's picture beside its volume label, then Type and File
//! system; Used space and Free space, each with its colour, and Capacity,
//! in bytes and in Explorer's short size (`123,456,789,012 bytes  114
//! GB`); then the ring of the two with the drive's name under it and,
//! where a disk usage analyser is installed, Details, which opens it on
//! the drive as Windows' Details opens Storage settings. Nothing on the
//! drive is counted. The label box renames the volume on OK or Apply, as
//! Windows' does, the same way the drive's Rename does
//! ([`BrowserWindow::rename_drive_at`]): through UDisks, which may ask for a
//! password, unmounting and mounting again a file system that must be. A
//! share's or a disc's label cannot be changed, so its box is read-only.
//!
//! Where it is mounted and what is mounted there, Linux facts Windows
//! has no rows for, are on the Hardware tab, as Windows keeps the disk's
//! own facts on its Hardware tab.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::format;
use ox_core::integration::DiskTool;
use ox_core::location::LocationContext;

use super::drive_facts::DriveType;
use super::general_tab::HEADER_ART_SIZE;
use super::metadata::MountFacts;
use super::space_ring::SpaceRing;
use super::window::{divider, page, push_button, ApplyFuture, Pending, PropertiesWindow, Rows};
use crate::icons::{Art, ArtImage, Icon};
use crate::window::{BrowserWindow, WindowAction};

/// The drive's picture, as This PC's drive cards draw it.
pub(super) const DRIVE_ART: Art = Art::Glyph(Icon::HardDrive);

/// The General tab of the drive mounted at `uri`; its label box renames
/// the drive on `window`'s OK or Apply, where that can be done.
pub(super) fn drive_general_tab(
    uri: &str,
    mount: &MountFacts,
    locations: &LocationContext,
    window: Option<&PropertiesWindow>,
) -> gtk::Box {
    let page = page();
    page.append(&header(uri, mount, window));
    page.append(&divider());

    let kind = Rows::new();
    kind.add(&ox_core::i18n::gettext("Type"), &mount.drive_type.label());
    kind.add(&ox_core::i18n::gettext("File system"), &mount.filesystem);
    page.append(kind.widget());
    // Space GIO could not read is left out, never shown as zero.
    if let Some((free, total)) = mount.space {
        page.append(&divider());
        append_space(&page, free, total);
    }
    page.append(&name_row(uri, mount, locations));
    page
}

/// Used space, Free space and Capacity, then the ring of `free` of
/// `total` bytes.
fn append_space(page: &gtk::Box, free: u64, total: u64) {
    let space = SpaceRows::new();
    space.add(
        &ox_core::i18n::gettext("Used space"),
        Some("used"),
        total.saturating_sub(free),
    );
    space.add(&ox_core::i18n::gettext("Free space"), Some("free"), free);
    page.append(&space.grid);
    page.append(&divider());
    let capacity = SpaceRows::new();
    capacity.add(&ox_core::i18n::gettext("Capacity"), None, total);
    page.append(&capacity.grid);
    let text = format::free_of_text(free, total);
    let ring = SpaceRing::new(free, total, &text);
    ring.add_css_class("drive-ring");
    page.append(&ring);
}

/// The drive's picture and its volume label, which renames the drive on
/// `window`'s OK or Apply where it can be renamed.
fn header(uri: &str, mount: &MountFacts, window: Option<&PropertiesWindow>) -> gtk::Box {
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["properties-header"])
        .build();
    header.append(&ArtImage::new(DRIVE_ART, HEADER_ART_SIZE));
    let renames = window.filter(|_| can_rename(mount));
    let label = gtk::Entry::builder()
        .text(&mount.label)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .editable(renames.is_some())
        .input_hints(gtk::InputHints::NO_SPELLCHECK)
        .css_classes(["properties-name"])
        .build();
    label.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
        "Volume label",
    ))]);
    header.append(&label);
    if let Some(window) = renames {
        DriveLabel::follow(window, &label, uri, mount);
    }
    header
}

/// Whether the drive's volume label can be changed: a local or removable
/// drive whose block device is known. A share or a disc keeps its label.
fn can_rename(mount: &MountFacts) -> bool {
    matches!(mount.drive_type, DriveType::Local | DriveType::Removable)
        && mount.mounted_from.starts_with("/dev/")
}

/// A new volume label typed in the box, which OK or Apply gives the drive.
struct DriveLabel {
    entry: gtk::Entry,
    /// Where the drive is mounted.
    root: String,
    /// The Properties window, whose opener renames the drive.
    window: glib::WeakRef<PropertiesWindow>,
    /// The label the drive has.
    label: RefCell<String>,
}

impl DriveLabel {
    /// Lets `entry`, the label box of `mount`, rename the drive on
    /// `window`'s OK or Apply.
    fn follow(window: &PropertiesWindow, entry: &gtk::Entry, root: &str, mount: &MountFacts) {
        let label = Rc::new(Self {
            entry: entry.clone(),
            root: root.to_owned(),
            window: window.downgrade(),
            label: RefCell::new(mount.label.clone()),
        });
        let watched = window.downgrade();
        entry.connect_changed(move |_| {
            if let Some(window) = watched.upgrade() {
                window.changed();
            }
        });
        window.add_pending(label);
    }

    async fn rename(self: Rc<Self>) -> Result<(), String> {
        let wanted = self.entry.text().trim().to_owned();
        let opener: Option<BrowserWindow> = self.window.upgrade().and_then(|window| window.opener());
        let Some(opener) = opener else {
            return Err(ox_core::i18n::gettext(
                "The window that opened these properties is closed.",
            ));
        };
        // A dismissed password prompt answers the empty message, which
        // the window takes as CANCELLED: nothing is said.
        opener.rename_drive_at(&self.root, &wanted).await?;
        self.label.replace(wanted);
        Ok(())
    }
}

impl Pending for DriveLabel {
    fn has_changes(&self) -> bool {
        self.entry.text().trim() != self.label.borrow().as_str()
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(self.rename())
    }
}

/// The drive's name under the ring, and Details at the right where a
/// disk usage analyser is installed.
fn name_row(uri: &str, mount: &MountFacts, locations: &LocationContext) -> gtk::CenterBox {
    let row = gtk::CenterBox::builder().css_classes(["drive-name-row"]).build();
    let name = if mount.label.is_empty() || mount.mounted_on == "/" {
        locations.base_name(uri)
    } else {
        mount.label.clone()
    };
    row.set_center_widget(Some(
        &gtk::Label::builder()
            .label(name)
            // A long volume name ends in "…" inside the window's width.
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["drive-name"])
            .build(),
    ));
    if crate::window::is_disk_tool_installed(DiskTool::AnalyseUsage) {
        let details = push_button(&ox_core::i18n::gettext("Details"));
        super::window::give_access_key(&details, 'd');
        details.set_valign(gtk::Align::Center);
        details.update_property(&[gtk::accessible::Property::Description(&ox_core::i18n::gettext(
            "Opens the disk usage analyser on this drive",
        ))]);
        let uri = uri.to_owned();
        details.connect_clicked(move |button| {
            let opener = PropertiesWindow::of(button).and_then(|window| window.opener());
            if let Some(opener) = opener {
                let action = format!("win.{}", WindowAction::AnalyseDiskUsage.name());
                // A window that cannot run it any more has nothing to do.
                let _ = WidgetExt::activate_action(&opener, &action, Some(&uri.to_variant()));
            }
        });
        row.set_end_widget(Some(&details));
    }
    row
}

/// Rows of a size in bytes and in Explorer's short size, each after its
/// colour, as Windows lines up Used space, Free space and Capacity.
struct SpaceRows {
    grid: gtk::Grid,
    line: std::cell::Cell<i32>,
}

impl SpaceRows {
    fn new() -> Self {
        let grid = gtk::Grid::builder()
            .css_classes(["properties-rows", "drive-space"])
            .build();
        Self {
            grid,
            line: std::cell::Cell::new(0),
        }
    }

    /// Adds `name:` after the colour swatch `swatch` (`used` or `free`),
    /// then `bytes` in bytes and in short.
    fn add(&self, name: &str, swatch: Option<&str>, bytes: u64) {
        let line = self.line.get();
        let colour = gtk::Box::builder()
            .valign(gtk::Align::Center)
            .css_classes(["drive-swatch"])
            .build();
        if let Some(swatch) = swatch {
            colour.add_css_class(swatch);
        }
        self.grid.attach(&colour, 0, line, 1, 1);
        let label = gtk::Label::builder()
            .label(ox_core::i18n::format_message("{name}:", &[("name", name)]))
            .xalign(0.0)
            .css_classes(["property-name"])
            .build();
        self.grid.attach(&label, 1, line, 1, 1);
        let exact = format::exact_bytes(bytes);
        let short = format::short_size(bytes);
        for (column, text) in [(2, exact), (3, short)] {
            let value = gtk::Label::builder()
                .label(text)
                .xalign(1.0)
                .hexpand(column == 2)
                .selectable(true)
                .css_classes(["property-value"])
                .build();
            crate::accessible_names::name_value_label(&value, &label);
            self.grid.attach(&value, column, line, 1, 1);
        }
        self.line.set(line + 1);
    }
}

/// The Hardware tab: where the drive is mounted and what is mounted
/// there.
pub(super) fn hardware_rows(mount: &MountFacts) -> gtk::Grid {
    let rows = Rows::new();
    rows.add(&ox_core::i18n::gettext("Mounted on"), &mount.mounted_on);
    rows.add(&ox_core::i18n::gettext("Mounted from"), &mount.mounted_from);
    rows.widget().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::descendants;

    /// A drive whose space GIO could not read keeps its other facts but
    /// shows no sizes or ring, rather than zeros.
    ///
    /// parity: PROP-004
    #[gtk::test]
    fn unknown_space_is_left_out_not_shown_as_zero() {
        let mount = MountFacts {
            mounted_on: "/media/demo/USB".to_owned(),
            mounted_from: "/dev/sdz1".to_owned(),
            filesystem: "exFAT".to_owned(),
            label: "USB".to_owned(),
            drive_type: DriveType::Removable,
            space: None,
        };

        let page = drive_general_tab(
            "file:///media/demo/USB",
            &mount,
            &LocationContext::default(),
            None,
        );

        let texts: Vec<String> = descendants::<gtk::Label>(&page)
            .iter()
            .map(|label| label.text().to_string())
            .collect();
        assert!(texts.contains(&"Removable Disk".to_owned()), "{texts:?}");
        assert!(texts.contains(&"exFAT".to_owned()), "{texts:?}");
        for row in ["Used space:", "Free space:", "Capacity:"] {
            assert!(!texts.contains(&row.to_owned()), "no {row} without a measurement");
        }
        assert!(descendants::<SpaceRing>(&page).is_empty(), "no ring");
    }

    /// A local or removable drive's label box can be typed in, and a new
    /// label waits for OK or Apply; a share's and a disc's stay read-only,
    /// as their labels cannot be changed.
    ///
    /// parity: PROP-004
    #[gtk::test]
    fn the_label_box_renames_a_local_drive_only() {
        let removable = MountFacts {
            mounted_on: "/media/demo/USB".to_owned(),
            mounted_from: "/dev/sdz1".to_owned(),
            filesystem: "exFAT".to_owned(),
            label: "USB".to_owned(),
            drive_type: DriveType::Removable,
            space: None,
        };
        let window = PropertiesWindow::new("USB Properties", None, None);
        let page = drive_general_tab(
            "file:///media/demo/USB",
            &removable,
            &LocationContext::default(),
            Some(&window),
        );
        let label = descendants::<gtk::Entry>(&page)
            .into_iter()
            .next()
            .expect("the label box");
        assert!(label.is_editable());
        assert!(!window.has_changes());
        label.set_text("Holiday");
        assert!(window.has_changes(), "a new label waits for OK or Apply");
        label.set_text(" USB ");
        assert!(
            !window.has_changes(),
            "the same label, spaces aside, is no change"
        );

        for (from, kind) in [("//nas/share", DriveType::Network), ("/dev/sr0", DriveType::Cd)] {
            let mount = MountFacts {
                mounted_from: from.to_owned(),
                drive_type: kind,
                ..removable.clone()
            };
            let window = PropertiesWindow::new("Drive Properties", None, None);
            let page = drive_general_tab(
                "file:///media/demo/Drive",
                &mount,
                &LocationContext::default(),
                Some(&window),
            );
            let label = descendants::<gtk::Entry>(&page)
                .into_iter()
                .next()
                .expect("the label box");
            assert!(!label.is_editable(), "{kind:?} keeps its label");
            window.destroy();
        }
        window.destroy();
    }
}
