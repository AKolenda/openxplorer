// SPDX-License-Identifier: AGPL-3.0-only
//! What GitHub's "latest release" answer says, checked. Ports
//! `release_metadata` in `desktop/updater.py`.

use serde_json::{Map, Value};

use super::{ReleaseVersion, TrustedUrl, UpdateError, RELEASE_REPOSITORIES};

/// The largest installer the updater downloads: 100 MiB.
pub const MAX_INSTALLER_SIZE: u64 = 100 * 1024 * 1024;

/// How many characters of release notes are kept.
pub const MAX_NOTES_CHARS: usize = 20_000;

/// The Debian architecture of the installer. The Python app is packaged as
/// `all`; the native package's architecture is decided with its packaging
/// (UPD-017), and this is the one place to change it.
pub(super) const INSTALLER_ARCHITECTURE: &str = "all";

/// The prefix GitHub puts before an asset's SHA-256 digest.
const DIGEST_PREFIX: &str = "sha256:";

/// A stable release that passed every check of [`parse_release`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The release's version, from its `vMAJOR.MINOR.PATCH` tag.
    pub version: ReleaseVersion,
    /// Newer than the running version.
    pub is_newer: bool,
    /// The release notes, at most [`MAX_NOTES_CHARS`] characters.
    pub notes: String,
    /// The release page, built from the repository and tag rather than
    /// taken from the answer.
    pub release_url: String,
    /// The Debian installer the release publishes.
    pub installer: Installer,
}

/// A release's Debian installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    /// `openxplorer_<version>_all.deb`; see [`installer_name`].
    pub name: String,
    /// Where GitHub publishes it.
    pub url: TrustedUrl,
    /// The SHA-256 digest GitHub computed for it.
    pub sha256: Sha256Digest,
    /// Its size in bytes, 1 to [`MAX_INSTALLER_SIZE`].
    pub size: u64,
}

/// A SHA-256 digest as 64 lower-case hexadecimal digits.
///
/// GitHub's asset digest verifies the download; it is not an independent
/// publisher signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Reads GitHub's `sha256:<64 lower-case hex digits>` form; `None` for
    /// anything else, upper-case digits included.
    pub fn from_github(digest: &str) -> Option<Self> {
        let hex = digest.strip_prefix(DIGEST_PREFIX)?;
        let is_lower_hex = hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        (hex.len() == 64 && is_lower_hex).then(|| Self(hex.to_owned()))
    }

    /// The 64 hexadecimal digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A new SHA-256 checksum, to digest a download or a build.
pub(super) fn sha256_checksum() -> glib::Checksum {
    glib::Checksum::new(glib::ChecksumType::Sha256).expect("GLib always supports SHA-256")
}

/// The installer's file name for `version`: `openxplorer_<version>_all.deb`.
pub fn installer_name(version: ReleaseVersion) -> String {
    format!("openxplorer_{version}_{INSTALLER_ARCHITECTURE}.deb")
}

/// Checks GitHub's answer for the latest release and compares its version
/// with `current`.
///
/// Safety rule "only stable, verified releases" (`release_metadata` in
/// `desktop/updater.py`): the release must not be a draft or pre-release,
/// its tag must be `vMAJOR.MINOR.PATCH`, and it must publish the installer
/// named for that version at exactly
/// `<repository>/releases/download/<tag>/<name>` in one of
/// [`RELEASE_REPOSITORIES`], with a SHA-256 digest and a size of 1 byte to
/// [`MAX_INSTALLER_SIZE`]. Nothing else in the answer is trusted: the
/// release page is built locally and the notes are cut to
/// [`MAX_NOTES_CHARS`].
///
/// # Errors
///
/// The first check that fails, in `release_metadata`'s order:
/// [`UpdateError::NoStableRelease`], [`UpdateError::InvalidTag`],
/// [`UpdateError::UnsupportedVersion`], [`UpdateError::InvalidAssetList`],
/// [`UpdateError::MissingInstaller`], [`UpdateError::MissingDigest`] or
/// [`UpdateError::InvalidInstallerSize`].
pub fn parse_release(answer: &Value, current: ReleaseVersion) -> Result<Release, UpdateError> {
    let Some(release) = answer.as_object() else {
        return Err(UpdateError::NoStableRelease);
    };
    if is_truthy(release.get("draft")) || is_truthy(release.get("prerelease")) {
        return Err(UpdateError::NoStableRelease);
    }
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(version_text) = tag.strip_prefix('v') else {
        return Err(UpdateError::InvalidTag);
    };
    let version: ReleaseVersion = version_text.parse()?;
    let Some(assets) = release.get("assets").and_then(Value::as_array) else {
        return Err(UpdateError::InvalidAssetList);
    };
    let name = installer_name(version);
    let (asset, repository) = find_installer(assets, tag, &name)?;
    let installer = Installer {
        url: TrustedUrl::parse(&download_url(repository, tag, &name))?,
        sha256: asset_digest(asset)?,
        size: asset_size(asset)?,
        name,
    };
    Ok(Release {
        version,
        is_newer: version > current,
        notes: release_notes(release.get("body")),
        release_url: format!("{repository}/releases/tag/{tag}"),
        installer,
    })
}

/// The first asset named `name`, and the repository its download URL
/// belongs to.
fn find_installer<'a>(
    assets: &'a [Value],
    tag: &str,
    name: &str,
) -> Result<(&'a Map<String, Value>, &'static str), UpdateError> {
    let asset = assets
        .iter()
        .filter_map(Value::as_object)
        .find(|asset| asset.get("name").and_then(Value::as_str) == Some(name))
        .ok_or(UpdateError::MissingInstaller)?;
    let url = asset.get("browser_download_url").and_then(Value::as_str);
    let repository = RELEASE_REPOSITORIES
        .into_iter()
        .find(|repository| url == Some(download_url(repository, tag, name).as_str()))
        .ok_or(UpdateError::MissingInstaller)?;
    Ok((asset, repository))
}

/// Where `repository` publishes the asset `name` of release `tag`.
fn download_url(repository: &str, tag: &str, name: &str) -> String {
    format!("{repository}/releases/download/{tag}/{name}")
}

/// The asset's `digest`, which must be GitHub's SHA-256 form.
fn asset_digest(asset: &Map<String, Value>) -> Result<Sha256Digest, UpdateError> {
    asset
        .get("digest")
        .and_then(Value::as_str)
        .and_then(Sha256Digest::from_github)
        .ok_or(UpdateError::MissingDigest)
}

/// The asset's `size`: a JSON integer (not a boolean or a fraction) from 1
/// to [`MAX_INSTALLER_SIZE`].
fn asset_size(asset: &Map<String, Value>) -> Result<u64, UpdateError> {
    asset
        .get("size")
        .and_then(Value::as_u64)
        .filter(|size| (1..=MAX_INSTALLER_SIZE).contains(size))
        .ok_or(UpdateError::InvalidInstallerSize)
}

/// The first [`MAX_NOTES_CHARS`] characters of the release notes.
///
/// Python turns any other JSON value into its `str()`; notes are never
/// shown, so a non-text value simply gives no notes here.
fn release_notes(body: Option<&Value>) -> String {
    let text = body.and_then(Value::as_str).unwrap_or_default();
    text.chars().take(MAX_NOTES_CHARS).collect()
}

/// Python's truth test of `release.get(key)`: missing, `null`, `false`,
/// zero, and empty text, lists and objects are false.
fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(fields)) => !fields.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn python_truthiness_decides_draft_and_prerelease() {
        let falsy = [
            json!(null),
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!([]),
            json!({}),
        ];
        let truthy = [
            json!(true),
            json!(1),
            json!(-0.5),
            json!("no"),
            json!([0]),
            json!({"a": 1}),
        ];

        assert!(!is_truthy(None));
        assert!(falsy.iter().all(|value| !is_truthy(Some(value))));
        assert!(truthy.iter().all(|value| is_truthy(Some(value))));
    }

    #[test]
    fn notes_are_cut_by_characters_not_bytes() {
        let body = json!("é".repeat(MAX_NOTES_CHARS + 5));

        let notes = release_notes(Some(&body));

        assert_eq!(notes.chars().count(), MAX_NOTES_CHARS);
        assert_eq!(release_notes(Some(&json!(5))), "");
    }

    #[test]
    fn the_installer_name_carries_the_version_and_architecture() {
        assert_eq!(
            installer_name(ReleaseVersion::new(1, 2, 3)),
            "openxplorer_1.2.3_all.deb"
        );
    }

    #[test]
    fn only_lower_case_sha256_digests_are_accepted() {
        let lower = format!("sha256:{}", "a".repeat(64));

        assert_eq!(
            Sha256Digest::from_github(&lower).unwrap().as_str(),
            "a".repeat(64)
        );
        assert!(Sha256Digest::from_github(&lower.to_uppercase()).is_none());
        assert!(Sha256Digest::from_github(&format!("sha256:{}", "a".repeat(63))).is_none());
    }
}
