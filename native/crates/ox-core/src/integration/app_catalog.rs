// SPDX-License-Identifier: AGPL-3.0-only
//! One entry per visible application for Open with, and the code-editor
//! shortcuts of the context menu.
//!
//! Ports `desktop/app_catalog.py` (OPEN-011, OPEN-015). Nothing here
//! launches an application or changes a default.

use std::collections::btree_map::{BTreeMap, Entry};

use super::applications::ApplicationInfo;
use super::default_apps::APP_ID;

/// The desktop IDs of the code editors that get an "Open in <name>"
/// shortcut: Visual Studio Code, Code Insiders and `VSCodium`, as
/// distribution packages and as Flatpaks.
const EDITOR_DESKTOP_IDS: [&str; 5] = [
    "code.desktop",
    "com.visualstudio.code.desktop",
    "codium.desktop",
    "com.vscodium.codium.desktop",
    "code-insiders.desktop",
];

/// The editors' primary launchers, preferred over their Flatpak twins.
const PRIMARY_EDITOR_IDS: [&str; 2] = ["code.desktop", "codium.desktop"];

/// An "Open in <name>" shortcut for an installed code editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorShortcut {
    /// The editor's desktop ID.
    pub id: String,
    /// The editor's name, as the menu shows it.
    pub name: String,
}

/// One application per visible name, sorted by that name.
///
/// A distribution package and a Flatpak have different desktop IDs but
/// the same name, so only one of them is offered: `preferred_id` (the
/// current default) first, then a primary editor launcher, then the
/// lowest ID. Names are compared ignoring case and spacing, so distinct
/// names such as "Code Insiders" and "`VSCodium`" stay separate. The app
/// itself, hidden launchers, URL-handler helpers and applications that
/// accept neither files nor URIs are left out.
pub fn unique_applications<A: ApplicationInfo>(
    applications: impl IntoIterator<Item = A>,
    preferred_id: Option<&str>,
) -> Vec<A> {
    let mut by_name: BTreeMap<String, (Rank, A)> = BTreeMap::new();
    for application in applications {
        let Some(id) = application.id().filter(|id| !id.is_empty()) else {
            continue;
        };
        if !is_offered(&application, &id) {
            continue;
        }
        let name = name_key(&application, &id);
        let rank = Rank::new(id, preferred_id);
        match by_name.entry(name) {
            Entry::Vacant(slot) => {
                slot.insert((rank, application));
            }
            Entry::Occupied(mut slot) if rank < slot.get().0 => {
                slot.insert((rank, application));
            }
            Entry::Occupied(_) => {}
        }
    }
    by_name
        .into_values()
        .map(|(_, application)| application)
        .collect()
}

/// The code editors among `applications`, one per visible name.
pub fn editor_shortcuts<A: ApplicationInfo>(
    applications: impl IntoIterator<Item = A>,
) -> Vec<EditorShortcut> {
    let editors = applications.into_iter().filter(|application| {
        application
            .id()
            .is_some_and(|id| EDITOR_DESKTOP_IDS.contains(&id.as_str()))
    });
    unique_applications(editors, None)
        .into_iter()
        .map(|editor| EditorShortcut {
            id: editor.id().unwrap_or_default(),
            name: editor.display_name(),
        })
        .collect()
}

/// How strongly a launcher represents its name: lower is preferred. The
/// fields compare in declaration order, as the Python rank tuple does.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    is_not_preferred: bool,
    is_not_primary_editor: bool,
    id: String,
}

impl Rank {
    fn new(id: String, preferred_id: Option<&str>) -> Self {
        Self {
            is_not_preferred: preferred_id != Some(id.as_str()),
            is_not_primary_editor: !PRIMARY_EDITOR_IDS.contains(&id.as_str()),
            id,
        }
    }
}

/// Whether `application` may be offered at all.
///
/// Safety rule "never offer the app for opening an item"
/// (`unique_applications` in `app_catalog.py`): its own launcher is left
/// out, so Open with can never hand a file back to the app.
fn is_offered<A: ApplicationInfo>(application: &A, id: &str) -> bool {
    id != APP_ID
        && application.should_show()
        && !is_url_handler(id)
        && (application.supports_files() || application.supports_uris())
}

/// True for helper launchers such as `code-url-handler.desktop`
/// (`url[-_]?handler`, ignoring case).
fn is_url_handler(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    ["urlhandler", "url-handler", "url_handler"]
        .iter()
        .any(|pattern| id.contains(pattern))
}

/// The visible name with runs of spaces joined and case folded; the ID
/// when the name is empty.
fn name_key<A: ApplicationInfo>(application: &A, id: &str) -> String {
    let name = application.display_name();
    let name = if name.is_empty() { id } else { name.as_str() };
    let words: Vec<&str> = name.split_whitespace().collect();
    glib::casefold(words.join(" ")).to_string()
}
