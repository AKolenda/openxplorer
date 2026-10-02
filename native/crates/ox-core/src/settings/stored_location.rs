// SPDX-License-Identifier: AGPL-3.0-only
//! Locations as `settings.json` keeps them: canonical, and an SFTP, FTP or
//! WebDAV address without its user name, as an SMB address never has one
//! (SAFE-010). The account is chosen again when the location is opened.

use crate::location::{self, without_user, LocationError};

/// [`location::normalise`], without a user name.
pub(super) fn normalise(address: &str) -> Result<String, LocationError> {
    location::normalise(address).map(|uri| without_user(&uri))
}

/// [`location::require_share`], without a user name.
pub(super) fn require_share(address: &str) -> Result<String, LocationError> {
    location::require_share(address).map(|uri| without_user(&uri))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SAFE-010
    #[test]
    fn settings_keep_remote_addresses_without_the_user_name() {
        assert_eq!(
            normalise("sftp://anna@build:2222/srv/").as_deref(),
            Ok("sftp://build:2222/srv")
        );
        assert_eq!(
            require_share("davs://anna@cloud/files").as_deref(),
            Ok("davs://cloud/files")
        );
        assert_eq!(normalise("ftp://mirror/pub").as_deref(), Ok("ftp://mirror/pub"));
        assert!(
            normalise("sftp://anna:secret@build/").is_err(),
            "a password is refused"
        );
    }
}
