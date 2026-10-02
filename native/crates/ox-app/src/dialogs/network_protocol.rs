// SPDX-License-Identifier: AGPL-3.0-only
//! The protocols Map network location connects with, and the address it
//! builds from the dialog's fields.
//!
//! SMB is what the Python app offered; SFTP, FTP, FTPS, WebDAV and NFS are
//! the `GVfs` backends Dolphin's and Files' "Add Network Folder" offer
//! (NET-002). The dialog only builds the address; validating it is
//! [`ox_core::location::require_share`]'s job, so a typed address and a
//! mapped one follow the same rules.

/// A protocol in the dialog's Type list, in the order shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Protocol {
    /// A Windows or Samba share: the Python app's only choice.
    Smb,
    /// SSH (SFTP).
    Sftp,
    /// Plain FTP.
    Ftp,
    /// FTP over TLS.
    Ftps,
    /// WebDAV over HTTP.
    WebDav,
    /// WebDAV over HTTPS, such as Nextcloud.
    WebDavs,
    /// An NFS export.
    Nfs,
}

impl Protocol {
    /// Every protocol, in the order of the Type list.
    pub(crate) const ALL: [Self; 7] = [
        Self::Smb,
        Self::Sftp,
        Self::Ftp,
        Self::Ftps,
        Self::WebDav,
        Self::WebDavs,
        Self::Nfs,
    ];

    /// The protocol at `position` in the Type list; SMB when out of range.
    pub(crate) fn at(position: u32) -> Self {
        let index = usize::try_from(position).unwrap_or(usize::MAX);
        Self::ALL.get(index).copied().unwrap_or(Self::Smb)
    }

    /// The name in the Type list.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Smb => ox_core::i18n::gettext_static("Windows share (SMB)"),
            Self::Sftp => ox_core::i18n::gettext_static("SSH (SFTP)"),
            Self::Ftp => ox_core::i18n::gettext_static("FTP"),
            Self::Ftps => ox_core::i18n::gettext_static("FTP with TLS (FTPS)"),
            Self::WebDav => ox_core::i18n::gettext_static("WebDAV"),
            Self::WebDavs => ox_core::i18n::gettext_static("Secure WebDAV (HTTPS)"),
            Self::Nfs => ox_core::i18n::gettext_static("NFS"),
        }
    }

    /// The protocol of URI scheme `scheme`, aliases such as `ssh`
    /// included; `None` for a scheme that is not a network protocol.
    pub(crate) fn from_scheme(scheme: &str) -> Option<Self> {
        let scheme = ox_core::location::canonical_remote_scheme(scheme);
        Self::ALL.into_iter().find(|protocol| protocol.scheme() == scheme)
    }

    /// The short name a server card shows, such as "SFTP".
    pub(crate) fn short_name(self) -> &'static str {
        match self {
            Self::Smb => "SMB",
            Self::Sftp => "SFTP",
            Self::Ftp => "FTP",
            Self::Ftps => "FTPS",
            Self::WebDav | Self::WebDavs => "WebDAV",
            Self::Nfs => "NFS",
        }
    }

    /// The URI scheme `GVfs` mounts.
    pub(crate) fn scheme(self) -> &'static str {
        match self {
            Self::Smb => "smb",
            Self::Sftp => "sftp",
            Self::Ftp => "ftp",
            Self::Ftps => "ftps",
            Self::WebDav => "dav",
            Self::WebDavs => "davs",
            Self::Nfs => "nfs",
        }
    }

    /// The example in the Folder field.
    pub(crate) fn placeholder(self) -> &'static str {
        match self {
            Self::Smb => "\\\\nas\\Projects",
            Self::Sftp => "server/home/anna",
            Self::Ftp | Self::Ftps => "ftp.example.com/pub",
            Self::WebDav | Self::WebDavs => "cloud.example.com/remote.php/dav/files/anna",
            Self::Nfs => "nas/export/media",
        }
    }

    /// Whether the dialog asks for a port and a user name. SMB takes both
    /// from the address and the sign-in dialog; NFS has no user name.
    pub(crate) fn asks_port(self) -> bool {
        self != Self::Smb
    }

    /// Whether the dialog asks for a user name (SFTP, FTP and WebDAV).
    pub(crate) fn asks_user(self) -> bool {
        !matches!(self, Self::Smb | Self::Nfs)
    }

    /// The address of the folder the fields describe: `folder` as typed
    /// for SMB or when it is already a URL, else
    /// `scheme://[user@]server[:port]/path` from `server/path`.
    pub(crate) fn address(self, folder: &str, port: &str, user: &str) -> String {
        let folder = folder.trim();
        if self == Self::Smb || folder.contains("://") {
            return folder.to_owned();
        }
        let folder = folder.trim_start_matches(['/', '\\']);
        let (server, path) = folder.split_once('/').unwrap_or((folder, ""));
        let user = match user.trim() {
            "" => String::new(),
            user if self.asks_user() => format!("{user}@"),
            _ => String::new(),
        };
        let port = match port.trim() {
            "" => String::new(),
            port => format!(":{port}"),
        };
        format!("{}://{user}{server}{port}/{path}", self.scheme())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: NET-002
    #[test]
    fn the_fields_build_the_address_of_each_protocol() {
        let cases = [
            (Protocol::Smb, "\\\\nas\\Projects", "", "", "\\\\nas\\Projects"),
            (
                Protocol::Sftp,
                "build/home/anna",
                "2222",
                "anna",
                "sftp://anna@build:2222/home/anna",
            ),
            (
                Protocol::Ftp,
                " ftp.example.com ",
                "",
                "",
                "ftp://ftp.example.com/",
            ),
            (Protocol::Ftps, "files/pub", "990", "", "ftps://files:990/pub"),
            (
                Protocol::WebDavs,
                "cloud/remote.php/dav",
                "",
                "anna",
                "davs://anna@cloud/remote.php/dav",
            ),
            (Protocol::Nfs, "nas/export", "", "anna", "nfs://nas/export"),
            (
                Protocol::Sftp,
                "sftp://other/typed",
                "22",
                "ignored",
                "sftp://other/typed",
            ),
        ];
        for (protocol, folder, port, user, expected) in cases {
            assert_eq!(protocol.address(folder, port, user), expected, "{protocol:?}");
        }
        assert_eq!(Protocol::at(6), Protocol::Nfs);
        assert_eq!(Protocol::at(99), Protocol::Smb);
    }
}
