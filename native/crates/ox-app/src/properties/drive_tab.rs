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
//! drive is counted. The label is read-only: renaming a volume would
//! need UDisks and, for some file systems, unmounting it.
//!
//! Where it is mounted and what is mounted there, Linux facts Windows
//! has no rows for, are on the Hardware tab, as Windows keeps the disk's
//! own facts on its Hardware tab.

use gtk::prelude::*;
use ox_core::format;
use ox_core::integration::DiskTool;
use ox_core::location::LocationContext;

use super::general_tab::HEADER_ART_SIZE;
use super::metadata::MountFacts;
use super::space_ring::SpaceRing;
use super::window::{divider, page, push_button, PropertiesWindow, Rows};
use crate::icons::{Art, ArtImage, Icon};
use crate::window::WindowAction;

/// The drive's picture, as This PC's drive cards draw it.
pub(super) const DRIVE_ART: Art = Art::Glyph(Icon::HardDrive);

/// The General tab of the drive mounted at `uri`.
pub(super) fn drive_general_tab(uri: &str, mount: &MountFacts, locations: &LocationContext) -> gtk::Box {
    let page = page();
    page.append(&header(mount));
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
    let text = ox_core::i18n::format_message(
        "{value1} free of {value2}",
        &[
            ("value1", &format::short_size(free)),
            ("value2", &format::short_size(total)),
        ],
    );
    let ring = SpaceRing::new(free, total, &text);
    ring.add_css_class("drive-ring");
    page.append(&ring);
}

/// The drive's picture and its volume label, which cannot be changed
/// here.
fn header(mount: &MountFacts) -> gtk::Box {
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["properties-header"])
        .build();
    header.append(&ArtImage::new(DRIVE_ART, HEADER_ART_SIZE));
    let label = gtk::Entry::builder()
        .text(&mount.label)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .editable(false)
        .css_classes(["properties-name"])
        .build();
    label.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
        "Volume label",
    ))]);
    header.append(&label);
    header
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
        let exact = ox_core::i18n::format_message("{bytes} bytes", &[("bytes", &format::grouped(bytes))]);
        let short = format::short_size(bytes);
        for (column, text) in [(2, exact), (3, short)] {
            let value = gtk::Label::builder()
                .label(text)
                .xalign(1.0)
                .hexpand(column == 2)
                .selectable(true)
                .css_classes(["property-value"])
                .build();
            value.update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
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
    use crate::properties::drive_facts::DriveType;
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

        let page = drive_general_tab("file:///media/demo/USB", &mount, &LocationContext::default());

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
}
