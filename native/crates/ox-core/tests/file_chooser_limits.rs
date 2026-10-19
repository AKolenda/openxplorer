// SPDX-License-Identifier: AGPL-3.0-only
//! Portal inputs keep bounded work and distinct filesystem output names.

use std::path::PathBuf;

use gio::prelude::*;
use ox_core::integration::{
    options_from_entries, ChooserAnswer, ChooserRequest, ChooserRequestError, MAX_LIST_ITEMS,
};

#[test]
fn standalone_current_filters_obey_the_pattern_limit() {
    let patterns = vec![(0u32, "*.txt".to_owned()); MAX_LIST_ITEMS + 1];
    for method in ["OpenFile", "SaveFile"] {
        let options = options_from_entries(&[("current_filter", ("Text", &patterns).to_variant())]);
        assert_eq!(
            ChooserRequest::from_call(method, "", &options),
            Err(ChooserRequestError::TooMany("filter patterns"))
        );
    }
}

#[test]
fn adding_a_current_filter_cannot_exceed_the_filter_count() {
    let listed: Vec<_> = (0..MAX_LIST_ITEMS)
        .map(|index| (format!("Type {index}"), vec![(0u32, "*".to_owned())]))
        .collect();
    for method in ["OpenFile", "SaveFile"] {
        let options = options_from_entries(&[
            ("filters", listed.to_variant()),
            ("current_filter", ("Extra", vec![(0u32, "*.txt")]).to_variant()),
        ]);
        assert_eq!(
            ChooserRequest::from_call(method, "", &options),
            Err(ChooserRequestError::TooMany("filters"))
        );
        let options = options_from_entries(&[
            ("filters", listed.to_variant()),
            ("current_filter", listed[0].to_variant()),
        ]);
        assert!(ChooserRequest::from_call(method, "", &options).is_ok());
    }
}

#[test]
fn save_files_preserves_distinct_non_utf8_names() {
    let files = vec![vec![b'a', 0xff, 0], vec![b'a', 0xfe, 0]];
    let options = options_from_entries(&[("files", files.to_variant())]);
    let request = ChooserRequest::from_call("SaveFiles", "", &options).unwrap();
    let (_, results) = request.reply(&ChooserAnswer::Chosen {
        locations: vec![PathBuf::from("/srv/example-exports")],
        filter: None,
        choices: Vec::new(),
        writable: false,
    });
    let uris: Vec<String> = glib::VariantDict::new(Some(&results))
        .lookup("uris")
        .unwrap()
        .unwrap();
    assert_eq!(
        uris,
        [
            "file:///srv/example-exports/a%FF",
            "file:///srv/example-exports/a%FE"
        ]
    );
}

#[test]
fn save_files_rejects_unsafe_raw_names() {
    for bytes in [b"../a\0".as_slice(), b"a/b\0", b"a\0b\0", b"..\0", b"\0"] {
        let options = options_from_entries(&[("files", vec![bytes.to_vec()].to_variant())]);
        assert!(matches!(
            ChooserRequest::from_call("SaveFiles", "", &options),
            Err(ChooserRequestError::BadName(_))
        ));
    }
}
