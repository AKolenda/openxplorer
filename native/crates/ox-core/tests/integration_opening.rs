// SPDX-License-Identifier: AGPL-3.0-only
//! Activating items, choosing the application that opens a file, and the
//! Open with and editor lists.
//!
//! The application mocks of the Python tests become [`TestApplication`]
//! values, shared by the case files:
//!
//! | Case file | What it covers | Ports |
//! |---|---|---|
//! | `activation` | What activating does; the application chosen | `OpeningTests` of `v2.0.0:desktop/tests/test_v05.py` |
//! | `app_catalog` | Open with and the editor shortcuts | `CatalogTests` of `v2.0.0:desktop/tests/test_v06.py` |
//! | `default_opener` | Preparing a real file for its application | `prepare_default` of `v2.0.0:desktop/native_opening.py` |

mod integration_support;

use ox_core::integration::ApplicationInfo;

/// The app's own launcher, the `smb://` scheme handler.
const OWN_ID: &str = "io.winspace.Development.desktop";

/// An installed application, like the Python tests' `Mock` apps.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplication {
    id: Option<String>,
    name: String,
    is_shown: bool,
    accepts_files: bool,
    accepts_uris: bool,
}

impl ApplicationInfo for TestApplication {
    fn id(&self) -> Option<String> {
        self.id.clone()
    }

    fn display_name(&self) -> String {
        self.name.clone()
    }

    fn should_show(&self) -> bool {
        self.is_shown
    }

    fn supports_files(&self) -> bool {
        self.accepts_files
    }

    fn supports_uris(&self) -> bool {
        self.accepts_uris
    }
}

/// A shown application that accepts files, as the Python `app()` helper
/// makes by default.
fn app(id: &str, name: &str) -> TestApplication {
    TestApplication {
        id: Some(id.to_owned()),
        name: name.to_owned(),
        is_shown: true,
        accepts_files: true,
        accepts_uris: false,
    }
}

#[path = "integration_opening/activation.rs"]
mod activation;
#[path = "integration_opening/app_catalog.rs"]
mod app_catalog;
#[path = "integration_opening/default_opener.rs"]
mod default_opener;
