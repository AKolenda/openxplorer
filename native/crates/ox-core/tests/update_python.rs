// SPDX-License-Identifier: AGPL-3.0-only
//! Runs the same inputs through `v2.0.0:desktop/updater.py` and the Rust port:
//! `version_tuple`, `trusted_url` and `release_metadata` must accept and
//! refuse the same inputs, with the same messages and results. Where the
//! port is deliberately stricter, the case is listed as such.

mod python_support;
mod update_support;

use std::fs;

use ox_core::update::{parse_release, ReleaseVersion, TrustedUrl, RELEASE_REPOSITORIES, TRUSTED_HOSTS};
use python_support::run_python;
use serde::Deserialize;
use serde_json::{json, Value};
use update_support::{next_release, release_answer, CURRENT, NEXT};

/// Reads the cases, runs each through `updater.py` and prints one outcome
/// per case: `{"ok": ...}` or `{"error": message}`.
const PYTHON_SCRIPT: &str = r"
import json, sys
from updater import release_metadata, trusted_url, version_tuple

def outcome(function, *arguments):
    try:
        return {'ok': function(*arguments)}
    except ValueError as error:
        return {'error': str(error)}

def release(case):
    result = outcome(release_metadata, case['answer'], case['current'])
    if 'ok' in result:
        fields = ('version', 'available', 'notes', 'releaseUrl', 'url', 'sha256', 'size', 'name')
        result['ok'] = {key: result['ok'][key] for key in fields}
    return result

cases = json.load(open(sys.argv[1]))
print(json.dumps({
    'versions': [outcome(lambda text: '.'.join(map(str, version_tuple(text))), text)
                 for text in cases['versions']],
    'urls': [outcome(trusted_url, url) for url in cases['urls']],
    'releases': [release(case) for case in cases['releases']],
}))
";

/// What Python answered for each table.
#[derive(Debug, Deserialize)]
struct PythonAnswers {
    versions: Vec<Outcome>,
    urls: Vec<Outcome>,
    releases: Vec<Outcome>,
}

/// One answer: a result or a `ValueError`'s message.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    Ok(Value),
    Error(String),
}

impl<T: Into<Value>> From<Result<T, ox_core::update::UpdateError>> for Outcome {
    fn from(result: Result<T, ox_core::update::UpdateError>) -> Self {
        match result {
            Ok(value) => Self::Ok(value.into()),
            Err(error) => Self::Error(error.to_string()),
        }
    }
}

fn version_cases() -> Vec<String> {
    let texts = [
        "0.0.0",
        "1.0.1",
        "1.10.0",
        "10.20.30",
        "01.0.1",
        "1.0",
        "1.0.1-rc1",
        "v1.0.1",
        "1.0.1\n",
        " 1.0.1",
        "1.0.1/../../evil",
        "1.0.1;touch /tmp/example",
        "",
        "1..1",
        "1.0.1.2",
        "1.+0.1",
        "1.0x1.0",
        "٣.0.0",
    ];
    texts.map(String::from).to_vec()
}

/// Addresses both apps agree on.
fn agreed_url_cases() -> Vec<String> {
    let mut urls: Vec<String> = TRUSTED_HOSTS
        .iter()
        .map(|host| format!("https://{host}/fixture"))
        .collect();
    let github = TRUSTED_HOSTS[1];
    let more = [
        format!("https://{}/x", github.to_uppercase()),
        format!("https://{github}:443/x"),
        format!("https://{github}./x"),
        format!("https://{github}#frag"),
        format!("https://{github}?q=1"),
        format!("https://{github}/a b"),
        format!("https://{github}:0443/x"),
        format!("https://{github}:/x"),
        format!("https://{github}"),
        format!("https://{github}%2eevil.com/x"),
        format!("https://evil.com\\@{github}/x"),
        format!("https:{github}/x"),
        format!("https:///{github}/x"),
        format!("https://{github}:444/x"),
        format!("https://user@{github}/x"),
        format!("https://user:secret@{github}/x"),
        format!("http://{github}/x"),
        "https://[::1]/x".to_owned(),
        "file:///tmp/example".to_owned(),
        "https://example.invalid/fixture".to_owned(),
        format!("https://{github}.example.invalid/fixture"),
    ];
    urls.extend(more);
    urls
}

/// Addresses Python's `urlsplit` accepts and the port refuses: an empty
/// user name, surrounding whitespace, a control character and a broken
/// percent escape. GitHub never returns such an address, and
/// refusing it keeps the trust check and the HTTP request from reading it
/// differently.
fn stricter_url_cases() -> Vec<String> {
    let github = TRUSTED_HOSTS[1];
    vec![
        format!("https://@{github}/x"),
        format!(" https://{github}/x"),
        format!("https://{github}/\u{0}"),
        format!("https://{github}/%zz"),
        format!("https://{github}\t/x"),
    ]
}

/// Release answers, each changed in one way from the fixture release.
fn release_cases() -> Vec<Value> {
    let change = |path: &str, value: Value| {
        let mut answer = next_release();
        answer
            .pointer_mut(path)
            .expect("the fixture has the field")
            .clone_from(&value);
        answer
    };
    vec![
        next_release(),
        release_answer(NEXT, RELEASE_REPOSITORIES[1]),
        release_answer(CURRENT, RELEASE_REPOSITORIES[0]),
        release_answer(ReleaseVersion::new(1, 10, 0), RELEASE_REPOSITORIES[0]),
        json!(null),
        json!([next_release()]),
        change("/draft", json!(1)),
        change("/prerelease", json!("yes")),
        change("/draft", json!([])),
        change("/tag_name", json!("1.0.1")),
        change("/tag_name", json!("v1.0.1-rc1")),
        change("/tag_name", json!(101)),
        change("/assets", json!({})),
        change("/assets", json!([1, "x", null])),
        change("/assets/0/name", json!("openxplorer_1.0.1_amd64.deb")),
        change("/assets/0/browser_download_url", json!(null)),
        change("/assets/0/digest", json!("sha256:abc")),
        change("/assets/0/digest", json!(false)),
        change("/assets/0/size", json!(0)),
        change("/assets/0/size", json!(104_857_600)),
        change("/assets/0/size", json!(104_857_601)),
        change("/assets/0/size", json!(true)),
        change("/body", json!("é".repeat(20_001))),
        change("/body", json!(null)),
    ]
}

/// The fields of a Rust release that Python's result has.
fn release_outcome(answer: &Value, current: ReleaseVersion) -> Outcome {
    let release = parse_release(answer, current).map(|release| {
        json!({
            "version": release.version.to_string(),
            "available": release.is_newer,
            "notes": release.notes,
            "releaseUrl": release.release_url,
            "url": release.installer.url.as_str(),
            "sha256": release.installer.sha256.as_str(),
            "size": release.installer.size,
            "name": release.installer.name,
        })
    });
    release.into()
}

fn python_answers(versions: &[String], urls: &[String], releases: &[Value]) -> PythonAnswers {
    let folder = tempfile::tempdir().unwrap();
    let cases_file = folder.path().join("cases.json");
    let release_cases: Vec<Value> = releases
        .iter()
        .map(|answer| json!({"answer": answer, "current": CURRENT.to_string()}))
        .collect();
    let cases = json!({"versions": versions, "urls": urls, "releases": release_cases});
    fs::write(&cases_file, cases.to_string()).unwrap();
    serde_json::from_str(&run_python(PYTHON_SCRIPT, &[&cases_file])).unwrap()
}

/// parity: UPD-002
#[test]
fn versions_urls_and_releases_match_the_python_updater() {
    let versions = version_cases();
    let urls = agreed_url_cases();
    let releases = release_cases();

    let python = python_answers(&versions, &urls, &releases);

    for (text, expected) in versions.iter().zip(&python.versions) {
        let parsed = text.parse::<ReleaseVersion>().map(|version| version.to_string());
        assert_eq!(&Outcome::from(parsed), expected, "version {text:?}");
    }
    for (url, expected) in urls.iter().zip(&python.urls) {
        let checked = TrustedUrl::parse(url).map(|url| url.as_str().to_owned());
        assert_eq!(&Outcome::from(checked), expected, "url {url:?}");
    }
    for (answer, expected) in releases.iter().zip(&python.releases) {
        assert_eq!(&release_outcome(answer, CURRENT), expected, "release {answer}");
    }
}

/// parity: UPD-002
#[test]
fn the_port_refuses_addresses_python_would_have_let_through() {
    let urls = stricter_url_cases();

    let python = python_answers(&[], &urls, &[]);

    for (url, python_outcome) in urls.iter().zip(&python.urls) {
        assert!(matches!(python_outcome, Outcome::Ok(_)), "Python accepts {url:?}");
        assert!(TrustedUrl::parse(url).is_err(), "the port refuses {url:?}");
    }
}
