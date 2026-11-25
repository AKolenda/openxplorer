// SPDX-License-Identifier: AGPL-3.0-only
//! The small parsers of text and bytes other programs give the app:
//! addresses and URIs, clipboard file lists, file dialog requests over
//! D-Bus, mount tables, `user-dirs.dirs`, compiled catalogs, an SMB
//! server's snapshot list, bookmarks and service menus.
//!
//! The first byte of the input picks the parser; the rest is its input.

use std::path::Path;
use std::sync::OnceLock;

use gio::prelude::FileExt;

use crate::clipboard::{decode, FileListFormat};
use crate::integration::{ChooserRequest, FileManagerMethod, FileManagerRequest};
use crate::location::{self, ArchiveLocation};

/// The parsers, in the order the first byte picks them.
const PARSERS: u8 = 14;

/// Runs the parser the first byte of `data` picks on the rest.
pub fn parse(data: &[u8]) {
    let Some((&which, input)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(input);
    match which % PARSERS {
        0 => locations(&text),
        1 => {
            let _ = decode(FileListFormat::Custom, input);
            let _ = decode(FileListFormat::Gnome, input);
            let (list, marker) = split_at_nul(input);
            let _ = decode(FileListFormat::UriList { kde_cut_marker: None }, list);
            let _ = decode(
                FileListFormat::UriList {
                    kde_cut_marker: Some(marker),
                },
                list,
            );
        }
        2 => chooser(input),
        3 => {
            let uris: Vec<&str> = text.lines().collect();
            for method in ["ShowFolders", "ShowItems", "ShowItemProperties", &text] {
                let _ = FileManagerRequest::from_method_name(method, &uris);
            }
            let _ = FileManagerRequest::new(FileManagerMethod::ShowItems, &uris);
        }
        4 => crate::search::fuzz_mounts(input),
        5 => {
            let _ = crate::sizes::mounts::parse_mounts(input);
        }
        6 => crate::admin_copy::fuzz_mount_tables(input),
        7 => {
            let _ = crate::network::parse_mount_table(&text);
        }
        8 => crate::places::fuzz_user_dirs(&text),
        9 => {
            crate::i18n::fuzz_mo(input);
            if let Some(catalog) = crate::i18n::Catalog::from_mo(input) {
                let _ = catalog.gettext("Copy");
                let _ = catalog.ngettext("{count} item", "{count} items", 3);
            }
        }
        10 => crate::network::fuzz_snapshot_reply(input),
        11 => {
            let _ = crate::places::merged_bookmarks(&text, &["file:///home/alex/Projects".to_owned()], &[]);
        }
        12 => service_menu(input),
        _ => names(&text),
    }
}

fn split_at_nul(bytes: &[u8]) -> (&[u8], &[u8]) {
    match bytes.iter().position(|&byte| byte == 0) {
        Some(at) => (&bytes[..at], &bytes[at + 1..]),
        None => (bytes, &[]),
    }
}

/// The address bar, D-Bus and settings take locations as text.
fn locations(text: &str) {
    let home = Path::new("/home/alex");
    for line in text.lines().take(8) {
        let _ = location::normalise(line);
        let _ = location::normalise_location(line, Some("file:///home/alex/Documents"), home);
        let _ = location::normalise_navigation(line, Some("smb://nas.example/share/Folder"), home);
        let _ = location::normalise_item(line);
        let _ = location::require_share(line);
        let _ = location::require_item_uri(line);
        let _ = location::typed_local_path(line);
        let _ = location::split_location(line);
        let _ = location::parent_location(line);
        let _ = location::device_root(line);
        let _ = location::crumb_divider(line, 1);
        let _ = location::is_smb_share_root(line);
        let _ = location::is_virtual_location(line);
        let _ = location::without_user(line);
        let _ = crate::network::local_path(line);
        let _ = crate::network::fuse_export_path(line, Path::new("/run/user/1000/gvfs"));
        let _ = crate::network::outside_location(line.to_owned());
        if let Ok(Some(archive)) = ArchiveLocation::parse(line) {
            let _ = archive.member("Docs/a.txt");
        }
        let _ = ArchiveLocation::root(line).member(line);
    }
}

/// A file dialog request as a portal sends it: the options dictionary
/// (`a{sv}`) in `GVariant`'s serialized form.
/// Input in `GVariant`'s text form is read as that too, so the seeds can
/// be written by hand.
fn chooser(bytes: &[u8]) {
    let variant = glib::Variant::from_bytes_with_type(&glib::Bytes::from(bytes), glib::VariantTy::VARDICT);
    chooser_options(&variant.normal_form());
    if let Ok(text) = std::str::from_utf8(bytes) {
        if let Ok(variant) = glib::Variant::parse(Some(glib::VariantTy::VARDICT), text) {
            chooser_options(&variant);
        }
    }
}

fn chooser_options(variant: &glib::Variant) {
    let options = glib::VariantDict::new(Some(variant));
    for method in ["OpenFile", "SaveFile", "SaveFiles"] {
        if let Ok(request) = ChooserRequest::from_call(method, "Fuzz", &options) {
            let _ = request.window_title();
            let _ = request.accept_label();
            for filter in &request.filters {
                let _ = filter.matches("Report 2024.final.PDF", Some("application/pdf"));
                let _ = filter.is_all_files();
            }
        }
    }
}

/// A KDE service menu installed by another program.
fn service_menu(bytes: &[u8]) {
    static FOLDER: OnceLock<tempfile::TempDir> = OnceLock::new();
    let folder = FOLDER.get_or_init(|| tempfile::tempdir().expect("a temporary folder"));
    let path = folder.path().join("fuzz.desktop");
    std::fs::write(&path, bytes).expect("the service menu");
    let uris = [
        gio::File::for_path("/home/alex/Documents/Report.txt")
            .uri()
            .to_string(),
        "smb://nas.example/share/Notes.txt".to_owned(),
    ];
    crate::service_actions::fuzz_service_menu(&path, &uris);
}

/// Names typed for new items, and the names made from them.
fn names(text: &str) {
    use crate::location::ItemKind;
    for line in text.lines().take(8) {
        let _ = location::validate_name(line);
        let _ = location::safe_label(line, "New folder");
        let _ = location::without_line_breaks(line);
        for kind in [ItemKind::File, ItemKind::Folder] {
            let _ = location::new_copy_name(line, 3, kind);
            let _ = location::kept_both_name(line, 2, kind);
            let _ = location::name_with_copy_marker(line, " - Copy", kind);
            let _ = location::fitted_name(line, kind, |name| name.chars().take(40).collect());
        }
        let _ = crate::integration::glob_matches(line, "Report 2024.final.pdf");
        let _ = crate::integration::glob_matches("*.[pP][dD][fF]", line);
    }
}
