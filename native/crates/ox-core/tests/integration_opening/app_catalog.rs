// SPDX-License-Identifier: AGPL-3.0-only
//! One entry per visible application for Open with, and the code-editor
//! shortcuts. Ports `CatalogTests` of `desktop/tests/test_v06.py`.

use ox_core::integration::{editor_shortcuts, unique_applications};

use super::{app, TestApplication, OWN_ID};

/// Visual Studio Code under `id`, the Python helper's default name.
fn code(id: &str) -> TestApplication {
    app(id, "Visual Studio Code")
}

fn ids(applications: &[TestApplication]) -> Vec<&str> {
    applications
        .iter()
        .map(|application| application.id.as_deref().unwrap_or_default())
        .collect()
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_duplicate_desktop_id`
/// parity: OPEN-011
#[test]
fn a_launcher_listed_twice_is_offered_once() {
    let editor = code("code.desktop");

    assert_eq!(unique_applications([editor.clone(), editor], None).len(), 1);
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_duplicate_visible_name_prefers_primary`
/// parity: OPEN-011, OPEN-015
#[test]
fn of_two_launchers_with_one_name_the_primary_one_is_offered() {
    let primary = code("code.desktop");
    let flatpak = code("com.visualstudio.code.desktop");

    assert_eq!(unique_applications([flatpak, primary.clone()], None), [primary]);
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_preferred_default_wins`
/// parity: OPEN-011
#[test]
fn the_current_default_represents_its_name() {
    let primary = code("code.desktop");
    let flatpak = code("com.visualstudio.code.desktop");

    let offered = unique_applications([primary, flatpak.clone()], Some("com.visualstudio.code.desktop"));

    assert_eq!(offered, [flatpak]);
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_url_helper_omitted`
/// parity: OPEN-011, OPEN-015
#[test]
fn url_handler_helpers_are_not_offered() {
    assert!(unique_applications([code("code-url-handler.desktop")], None).is_empty());
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_hidden_omitted`
/// parity: OPEN-011
#[test]
fn hidden_launchers_are_not_offered() {
    let hidden = TestApplication {
        is_shown: false,
        ..code("code.desktop")
    };

    assert!(unique_applications([hidden], None).is_empty());
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_missing_identifier_omitted`
/// parity: OPEN-011
#[test]
fn launchers_without_an_id_are_not_offered() {
    let without_id = TestApplication {
        id: None,
        ..code("code.desktop")
    };

    assert!(unique_applications([without_id], None).is_empty());
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_self_excluded`
/// parity: OPEN-011
#[test]
fn openxplorer_is_not_offered_to_open_items() {
    assert!(unique_applications([code(OWN_ID)], None).is_empty());
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_no_argument_support_omitted`
/// parity: OPEN-011
#[test]
fn launchers_that_take_neither_files_nor_uris_are_not_offered() {
    let no_arguments = TestApplication {
        accepts_files: false,
        accepts_uris: false,
        ..code("x.desktop")
    };

    assert!(unique_applications([no_arguments], None).is_empty());
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_distinct_editors_preserved`
/// parity: OPEN-015
#[test]
fn distinct_editors_each_get_a_shortcut() {
    let editors = [
        code("code.desktop"),
        app("code-insiders.desktop", "Code Insiders"),
        app("codium.desktop", "VSCodium"),
    ];

    assert_eq!(editor_shortcuts(editors).len(), 3);
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_other_app_not_editor_shortcut`
/// parity: OPEN-015
#[test]
fn other_applications_get_no_editor_shortcut() {
    assert!(editor_shortcuts([app("evince.desktop", "Document Viewer")]).is_empty());
}

/// Ported from `desktop/tests/test_v06.py::CatalogTests::test_name_case_and_space_dedup`
/// parity: OPEN-011
#[test]
fn names_differing_in_case_and_spacing_are_one_application() {
    let offered = unique_applications([app("a.desktop", " Code "), app("b.desktop", "code")], None);

    assert_eq!(ids(&offered), ["a.desktop"]);
}

/// parity: OPEN-011
#[test]
fn applications_are_listed_by_name() {
    let offered = unique_applications(
        [
            app("z.desktop", "Text Editor"),
            app("a.desktop", "Videos"),
            app("m.desktop", "Archive Manager"),
        ],
        None,
    );

    assert_eq!(ids(&offered), ["m.desktop", "z.desktop", "a.desktop"]);
}
