// SPDX-License-Identifier: AGPL-3.0-only
//! A selected catalogue changes display text without moving folders or
//! replacing the English aliases accepted by the address bar.

use std::path::Path;
use std::process::Command;

use ox_core::location::{normalise_navigation, VirtualPlace};
use ox_core::places::{FolderLocations, KnownFolder};

const CHILD_DIRECTORY: &str = "OPENXPLORER_LOCALIZED_PATH_TEST";

/// A small standard MO catalogue, stored only in the test's private data folder.
fn catalogue() -> Vec<u8> {
    let messages = [
        ("Desktop", "Translated desktop"),
        ("Home", "Translated home"),
        ("This PC", "Translated computer"),
    ];
    let count = u32::try_from(messages.len()).expect("three messages");
    let translated_table = 28 + count * 8;
    let mut bytes = vec![0; usize::try_from(28 + count * 16).expect("small tables")];
    for (index, word) in [0x9504_12de, 0, count, 28, translated_table, 0, 0]
        .into_iter()
        .enumerate()
    {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    for (index, (source, translated)) in messages.into_iter().enumerate() {
        for (table, text) in [(28, source), (translated_table, translated)] {
            let at = usize::try_from(table).expect("small table") + index * 8;
            let length = u32::try_from(text.len()).expect("short text");
            let offset = u32::try_from(bytes.len()).expect("small catalogue");
            bytes[at..at + 4].copy_from_slice(&length.to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
        }
    }
    bytes
}

fn check_localized_paths(root: &Path) {
    assert!(ox_core::i18n::install(), "the synthetic language was selected");
    assert_eq!(KnownFolder::Desktop.label(), "Translated desktop");
    let home = root.join("home");
    let locations = FolderLocations::new(home.clone(), &root.join("config"));
    for paths in [locations.default_paths(), locations.read_paths()] {
        assert_eq!(paths.path(KnownFolder::Desktop), home.join("Desktop"));
    }
    assert_eq!(KnownFolder::Desktop.xdg_key(), "DESKTOP");
    assert_eq!(VirtualPlace::ThisPc.title(), "Translated computer");
    for (english, translated, place, uri, alias) in [
        ("Home", "Translated home", VirtualPlace::Home, "ox:home", "home:"),
        (
            "This PC",
            "Translated computer",
            VirtualPlace::ThisPc,
            "ox:pc",
            "pc:",
        ),
    ] {
        assert_eq!(VirtualPlace::from_title(english), Some(place));
        assert_eq!(VirtualPlace::from_title(translated), Some(place));
        assert_eq!(place.uri(), uri);
        assert_eq!(normalise_navigation(uri, None, &home).as_deref(), Ok(uri));
        assert_eq!(normalise_navigation(alias, None, &home).as_deref(), Ok(uri));
    }
}

/// Catalogue installation is process-wide. The child tests translated
/// labels without changing the language of any other test in this run.
///
/// parity: INT-031, SIDE-006
#[test]
fn translated_labels_keep_paths_and_address_aliases_stable() {
    if let Some(root) = std::env::var_os(CHILD_DIRECTORY) {
        check_localized_paths(Path::new(&root));
        return;
    }
    assert!(
        std::env::var_os("OX_ISOLATED_SESSION").is_some(),
        "run this test through native/tools/check.py's isolated environment"
    );
    let directory = tempfile::tempdir().expect("a private test folder");
    let data = directory.path().join("data");
    let messages = data.join("locale/zz/LC_MESSAGES");
    std::fs::create_dir_all(&messages).expect("the synthetic locale folder");
    std::fs::write(messages.join("openxplorer.mo"), catalogue()).expect("the synthetic catalogue");
    let child = Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            "--exact",
            "translated_labels_keep_paths_and_address_aliases_stable",
            "--nocapture",
        ])
        .env(CHILD_DIRECTORY, directory.path())
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", &data)
        .env("LANGUAGE", "zz")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("the isolated catalogue test starts");
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}
