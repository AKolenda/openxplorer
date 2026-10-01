// SPDX-License-Identifier: AGPL-3.0-only
//! Stable release versions. Ports `version_tuple` in `v2.0.0:desktop/updater.py`.

use std::fmt;
use std::str::FromStr;

use super::UpdateError;

/// A stable release version, `MAJOR.MINOR.PATCH`. Versions compare
/// numerically, part by part, so 1.10.0 is newer than 1.9.9.
///
/// Safety rule "only plain stable versions" (`version_tuple` in
/// `v2.0.0:desktop/updater.py`): every part is `0` or ASCII digits without a
/// leading zero. A pre-release, a path fragment or shell text is never a
/// version, so it can never reach an installer name, a download URL or a
/// package command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl ReleaseVersion {
    /// The version `major.minor.patch`.
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self { major, minor, patch }
    }
}

impl FromStr for ReleaseVersion {
    type Err = UpdateError;

    /// Parses `MAJOR.MINOR.PATCH` exactly: no `v` prefix, no suffix, no
    /// whitespace.
    ///
    /// Python's integers are unbounded; a part above [`u64::MAX`] is refused
    /// here, which no real release reaches.
    ///
    /// # Errors
    ///
    /// [`UpdateError::UnsupportedVersion`] for anything else.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut parts = text.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(UpdateError::UnsupportedVersion);
        };
        Ok(Self::new(
            parse_part(major)?,
            parse_part(minor)?,
            parse_part(patch)?,
        ))
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One part of a version: `0|[1-9][0-9]*`, as the regular expression in
/// `version_tuple` accepts it.
fn parse_part(part: &str) -> Result<u64, UpdateError> {
    let is_digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let has_leading_zero = part.len() > 1 && part.starts_with('0');
    if !is_digits || has_leading_zero {
        return Err(UpdateError::UnsupportedVersion);
    }
    part.parse().map_err(|_| UpdateError::UnsupportedVersion)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_prints_as_it_was_written() {
        let version: ReleaseVersion = "1.10.0".parse().unwrap();

        assert_eq!(version, ReleaseVersion::new(1, 10, 0));
        assert_eq!(version.to_string(), "1.10.0");
    }

    #[test]
    fn zero_is_a_part_but_a_leading_zero_is_not() {
        assert!("0.0.0".parse::<ReleaseVersion>().is_ok());
        assert!("1.0.00".parse::<ReleaseVersion>().is_err());
        assert!("1.00.0".parse::<ReleaseVersion>().is_err());
    }

    #[test]
    fn parts_that_overflow_are_refused() {
        let too_large = format!("{}0.0.0", u64::MAX);

        assert!(too_large.parse::<ReleaseVersion>().is_err());
    }
}
