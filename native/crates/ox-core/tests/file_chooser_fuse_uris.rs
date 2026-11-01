// SPDX-License-Identifier: AGPL-3.0-only
//! A dialog's answer for a file on a network share mounted by `GVfs` names
//! its FUSE path as a `file://` URI, never the share's own `smb://` one.
//!
//! New in the native app (INT-032). The portal hands the reply's URIs to
//! an unsandboxed caller unchanged, and Chromium, Electron and most other
//! callers take only `file://` URIs: Chrome logs "Ignoring unknown file
//! chooser URI" for any other and treats the dialog as cancelled, so a
//! page saved to a share was never written and nothing said so. Under
//! `GVfs`, `g_file_new_for_path` of a path inside its FUSE export returns
//! the remote file, whose URI is `smb://…`. This test process installs a
//! VFS that does the same, so the reply is checked as it is built in a
//! desktop session; it is the only test in this executable because a
//! process has one default VFS.

use std::path::{Path, PathBuf};

use gio::prelude::*;
use gio::subclass::prelude::*;
use ox_core::integration::{options_from_entries, ChooserAnswer, ChooserRequest};

/// The FUSE export of the fictional share `smb://nas.invalid/projects`.
const FUSE_SHARE: &str = "/run/user/1000/gvfs/smb-share:server=nas.invalid,share=projects";

mod fuse_vfs {
    use super::*;

    glib::wrapper! {
        /// A VFS that maps paths inside [`FUSE_SHARE`] to the share's
        /// `smb://` location, as `GVfs`'s does.
        pub struct FuseVfs(ObjectSubclass<imp::FuseVfs>) @extends gio::Vfs;
    }

    mod imp {
        use super::*;

        #[derive(Default)]
        pub struct FuseVfs;

        #[glib::object_subclass]
        impl ObjectSubclass for FuseVfs {
            const NAME: &'static str = "OxTestFuseVfs";
            type Type = super::FuseVfs;
            type ParentType = gio::Vfs;
        }

        impl ObjectImpl for FuseVfs {}

        impl VfsImpl for FuseVfs {
            fn is_active(&self) -> bool {
                true
            }

            fn get_file_for_path(&self, path: &Path) -> gio::File {
                match path.strip_prefix(FUSE_SHARE) {
                    Ok(inside) => {
                        let uri = format!(
                            "smb://nas.invalid/projects/{}",
                            glib::Uri::escape_string(&inside.to_string_lossy(), Some("/"), false)
                        );
                        gio::Vfs::local().file_for_uri(&uri)
                    }
                    Err(_) => gio::Vfs::local().file_for_path(path),
                }
            }

            fn get_file_for_uri(&self, uri: &str) -> gio::File {
                gio::Vfs::local().file_for_uri(uri)
            }

            fn get_supported_uri_schemes(&self) -> &'static glib::StrVRef {
                static SCHEMES: std::sync::OnceLock<glib::StrV> = std::sync::OnceLock::new();
                SCHEMES.get_or_init(|| glib::StrV::from(vec!["file", "smb"]))
            }

            fn parse_name(&self, parse_name: &str) -> gio::File {
                gio::Vfs::local().parse_name(parse_name)
            }
        }
    }
}

/// parity: INT-032, INT-035
#[test]
fn a_file_on_a_mounted_share_is_answered_with_its_fuse_path() {
    // Registered here, before GIO registers its own and picks the default.
    gio::IOExtensionPoint::builder("gio-vfs")
        .required_type(gio::Vfs::static_type())
        .build();
    gio::IOExtensionPoint::implement("gio-vfs", fuse_vfs::FuseVfs::static_type(), "ox-test-fuse", 1000)
        .expect("the VFS extension point");
    let saved = PathBuf::from(FUSE_SHARE).join("Reports/Quarterly notes.html");
    // The VFS really maps the path, as GVfs's does.
    assert!(gio::File::for_path(&saved).uri().starts_with("smb://"));

    let request = ChooserRequest::from_call(
        "SaveFile",
        "",
        &options_from_entries(&[("current_name", "Quarterly notes.html".to_variant())]),
    )
    .expect("a valid call");
    let (response, results) = request.reply(&ChooserAnswer::Chosen {
        locations: vec![saved],
        filter: None,
        choices: Vec::new(),
        writable: true,
    });

    assert_eq!(response, 0);
    let uris: Vec<String> = glib::VariantDict::new(Some(&results))
        .lookup("uris")
        .expect("typed")
        .expect("uris");
    assert_eq!(
        uris,
        [format!("file://{FUSE_SHARE}/Reports/Quarterly%20notes.html")]
    );
}
