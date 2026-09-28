// SPDX-License-Identifier: AGPL-3.0-only
//! Runs the same tab states and `FileManager1` requests through
//! `desktop/window_state.py` and the Rust port: both must keep the same
//! fields and refuse the same input with the same message. The native app
//! writes its own pages as `ox:` URIs, which are mapped back to the Python
//! spellings before comparing.

mod python_support;

use std::fs;

use ox_core::location::{HOME_URI, NETWORK_URI, PC_URI, SETTINGS_URI};
use ox_core::session::{FileManagerRequest, TabSnapshot};
use python_support::run_python;
use serde::Deserialize;
use serde_json::{json, Value};

/// Reads the cases, runs each through `window_state.py` and prints one
/// outcome per case: `{"ok": ...}` or `{"error": message}`.
const PYTHON_SCRIPT: &str = r"
import json, sys
from window_state import filemanager_request, tab_snapshot

def outcome(function, *arguments):
    try:
        return {'ok': function(*arguments)}
    except ValueError as error:
        return {'error': str(error)}

cases = json.load(open(sys.argv[1]))
print(json.dumps({
    'tabs': [outcome(tab_snapshot, state) for state in cases['tabs']],
    'requests': [outcome(filemanager_request, case['method'], case['uris'])
                 for case in cases['requests']],
}))
";

/// What Python answered for each table.
#[derive(Debug, Deserialize)]
struct PythonAnswers {
    tabs: Vec<Outcome>,
    requests: Vec<Outcome>,
}

/// One answer: a result or a `ValueError`'s message.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    Ok(Value),
    Error(String),
}

/// A `FileManager1` request case.
#[derive(Debug, Clone)]
struct RequestCase {
    method: &'static str,
    uris: Vec<&'static str>,
}

fn tab_cases() -> Vec<Value> {
    vec![
        json!({
            "uri": "smb://nas/work",
            "history": ["file:///home/demo", "smb://nas/work"],
            "index": 1,
            "selection": ["smb://nas/work/report.pdf"],
            "view": "grid",
            "scroll": 1600,
            "sort": "size",
            "descending": true,
        }),
        json!({"uri": "home:", "password": "not a real password"}),
        json!({"uri": "javascript:alert(1)"}),
        json!({"uri": "home:", "index": true}),
        json!({"uri": "home:", "index": 3}),
        json!({"uri": "home:", "history": ["network:"], "index": 0}),
        json!({"history": vec!["home:"; 201]}),
        json!({"history": vec!["pc:"; 200]}),
        json!({"history": "home:"}),
        json!({"scroll": "inf"}),
        json!({"scroll": " 12.5 "}),
        json!({"scroll": true}),
        json!({"scroll": -40}),
        json!({"scroll": [1]}),
        json!({"selection": vec!["/tmp/a"; 10_001]}),
        json!({"selection": ["/tmp/a b", "smb://nas/work/x"]}),
        json!({"selection": [5]}),
        json!({"selection": ["settings:"]}),
        json!({"uri": "settings:", "settingsSection": "brave"}),
        json!({"uri": "settings:", "settingsSection": "danger"}),
        json!({"view": "tiles", "sort": "owner", "descending": "yes"}),
        json!({"uri": 7}),
        json!({"uri": "/tmp/x\u{1}"}),
        json!(["home:"]),
    ]
}

fn request_cases() -> Vec<RequestCase> {
    let case = |method, uris| RequestCase { method, uris };
    vec![
        case("ShowItems", vec!["file:///tmp/movie.mp4"]),
        case("ShowFolders", vec!["smb://nas/work"]),
        case("ShowItemProperties", vec!["/tmp/a;touch b"]),
        case("Execute", vec!["/tmp/script"]),
        case("ShowItems", vec![]),
        case("ShowItems", vec!["/tmp/x"; 101]),
        case("ShowFolders", vec!["settings:"]),
        case("ShowFolders", vec!["https://example.invalid/"]),
        case("ShowItems", vec!["smb://user:secret@nas/work"]),
    ]
}

/// The Python spelling of the app's pages.
fn python_spelling(uri: &str) -> String {
    let spelling = match uri {
        HOME_URI => "home:",
        PC_URI => "pc:",
        NETWORK_URI => "network:",
        SETTINGS_URI => "settings:",
        other => other,
    };
    spelling.to_owned()
}

/// A Rust snapshot in the Python app's JSON form and spellings.
fn tab_outcome(state: &Value) -> Outcome {
    match TabSnapshot::from_json(state) {
        Ok(mut snapshot) => {
            snapshot.uri = python_spelling(&snapshot.uri);
            snapshot.history = snapshot.history.iter().map(|uri| python_spelling(uri)).collect();
            Outcome::Ok(snapshot.to_json())
        }
        Err(error) => Outcome::Error(error.to_string()),
    }
}

/// Python's `max(0, min(1e9, scroll))` gives the integer 0 for a scroll
/// position of zero or less; the value is the same number.
fn with_float_scroll(outcome: Outcome) -> Outcome {
    let Outcome::Ok(mut snapshot) = outcome else {
        return outcome;
    };
    let scroll = snapshot["scroll"]
        .as_f64()
        .expect("a snapshot's scroll is a number");
    snapshot["scroll"] = json!(scroll);
    Outcome::Ok(snapshot)
}

fn request_outcome(case: &RequestCase) -> Outcome {
    match FileManagerRequest::new(case.method, &case.uris) {
        Ok(request) => Outcome::Ok(json!({"method": request.method.dbus_name(), "uris": request.uris})),
        Err(error) => Outcome::Error(error.to_string()),
    }
}

fn python_answers(tabs: &[Value], requests: &[RequestCase]) -> PythonAnswers {
    let folder = tempfile::tempdir().unwrap();
    let cases_file = folder.path().join("cases.json");
    let requests: Vec<Value> = requests
        .iter()
        .map(|case| json!({"method": case.method, "uris": case.uris}))
        .collect();
    fs::write(
        &cases_file,
        json!({"tabs": tabs, "requests": requests}).to_string(),
    )
    .unwrap();
    serde_json::from_str(&run_python(PYTHON_SCRIPT, &[&cases_file])).unwrap()
}

/// parity: TAB-038, SAFE-017
#[test]
fn tab_states_and_requests_match_the_python_window_state() {
    let tabs = tab_cases();
    let requests = request_cases();

    let python = python_answers(&tabs, &requests);

    for (state, expected) in tabs.iter().zip(&python.tabs) {
        let expected = with_float_scroll(expected.clone());
        assert_eq!(tab_outcome(state), expected, "tab {state}");
    }
    for (case, expected) in requests.iter().zip(&python.requests) {
        assert_eq!(&request_outcome(case), expected, "request {case:?}");
    }
}
