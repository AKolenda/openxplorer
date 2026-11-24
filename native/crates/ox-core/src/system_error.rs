// SPDX-License-Identifier: AGPL-3.0-only
//! The system's error in Windows' words: the one place a GIO or local I/O
//! error becomes the sentence the user reads.
//!
//! Windows shows a failed system call in its own words: "Access is
//! denied.", "The system cannot find the file specified.", and where it
//! offers an error code, as the copy engine's "Interrupted Action" does,
//! the line "Error 0x80070005: Access is denied.". GIO's messages are
//! developer text instead ("Error opening file “/home/…/a.txt”:
//! Permission denied", "Error splicing file: Input/output error"), with
//! paths, URIs and the C library's `strerror` words. [`SystemError`] sorts
//! an error by its GIO code, or by the `errno` whose words end GIO's
//! message, into the Win32 error Windows reports for the same failure and
//! says it in that error's words, from Microsoft's "System Error Codes"
//! pages (learn.microsoft.com/windows/win32/debug/system-error-codes);
//! anything it cannot sort is Windows' "Unspecified error" (`E_FAIL`).
//! GIO's own message never reaches the interface: it goes to the debug
//! log, where `G_MESSAGES_DEBUG=OpenXplorer` shows it.
//!
//! Each error has the code Windows shows with it: a Win32 error `x` is
//! the `HRESULT` `0x8007xxxx` (`HRESULT_FROM_WIN32`), `E_FAIL` is
//! `0x80004005`.

use gio::prelude::*;

use crate::i18n::{format_message, gettext, message_id};

/// The `HRESULT` of Windows' "Unspecified error".
const E_FAIL: u32 = 0x8000_4005;

/// A failure in Windows' words: its code and its sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemError {
    /// The `HRESULT` Windows shows with it.
    hresult: u32,
    /// Its sentence, untranslated.
    sentence: &'static str,
}

/// A Win32 error by its code, with Microsoft's sentence for it.
const fn win32(code: u32, sentence: &'static str) -> SystemError {
    SystemError {
        hresult: 0x8007_0000 | (code & 0xFFFF),
        sentence,
    }
}

/// `ERROR_INVALID_FUNCTION`.
pub const INVALID_FUNCTION: SystemError = win32(1, message_id("Incorrect function."));
/// `ERROR_FILE_NOT_FOUND`.
pub const FILE_NOT_FOUND: SystemError = win32(2, message_id("The system cannot find the file specified."));
/// `ERROR_PATH_NOT_FOUND`.
pub const PATH_NOT_FOUND: SystemError = win32(3, message_id("The system cannot find the path specified."));
/// `ERROR_TOO_MANY_OPEN_FILES`.
pub const TOO_MANY_OPEN_FILES: SystemError = win32(4, message_id("The system cannot open the file."));
/// `ERROR_ACCESS_DENIED`.
pub const ACCESS_DENIED: SystemError = win32(5, message_id("Access is denied."));
/// `ERROR_INVALID_HANDLE`.
pub const INVALID_HANDLE: SystemError = win32(6, message_id("The handle is invalid."));
/// `ERROR_NOT_ENOUGH_MEMORY`.
pub const NOT_ENOUGH_MEMORY: SystemError = win32(
    8,
    message_id("Not enough memory resources are available to process this command."),
);
/// `ERROR_INVALID_DATA`.
pub const INVALID_DATA: SystemError = win32(13, message_id("The data is invalid."));
/// `ERROR_NOT_SAME_DEVICE`.
pub const NOT_SAME_DEVICE: SystemError = win32(
    17,
    message_id("The system cannot move the file to a different disk drive."),
);
/// `ERROR_WRITE_PROTECT`.
pub const WRITE_PROTECT: SystemError = win32(19, message_id("The media is write protected."));
/// `ERROR_NOT_READY`.
pub const NOT_READY: SystemError = win32(21, message_id("The device is not ready."));
/// `ERROR_SHARING_VIOLATION`.
pub const SHARING_VIOLATION: SystemError = win32(
    32,
    message_id("The process cannot access the file because it is being used by another process."),
);
/// `ERROR_NOT_SUPPORTED`.
pub const NOT_SUPPORTED: SystemError = win32(50, message_id("The request is not supported."));
/// `ERROR_BAD_NETPATH`.
pub const BAD_NETPATH: SystemError = win32(53, message_id("The network path was not found."));
/// `ERROR_UNEXP_NET_ERR`.
pub const UNEXPECTED_NETWORK_ERROR: SystemError =
    win32(59, message_id("An unexpected network error occurred."));
/// `ERROR_NETNAME_DELETED`.
pub const NETNAME_DELETED: SystemError = win32(
    64,
    message_id("The specified network name is no longer available."),
);
/// `ERROR_BAD_NET_NAME`.
pub const BAD_NET_NAME: SystemError = win32(67, message_id("The network name cannot be found."));
/// `ERROR_INVALID_PARAMETER`.
pub const INVALID_PARAMETER: SystemError = win32(87, message_id("The parameter is incorrect."));
/// `ERROR_DISK_FULL`.
pub const DISK_FULL: SystemError = win32(112, message_id("There is not enough space on the disk."));
/// `ERROR_SEM_TIMEOUT`, what Windows reports for a share that stops
/// answering.
pub const SEM_TIMEOUT: SystemError = win32(121, message_id("The semaphore timeout period has expired."));
/// `ERROR_INVALID_NAME`.
pub const INVALID_NAME: SystemError = win32(
    123,
    message_id("The filename, directory name, or volume label syntax is incorrect."),
);
/// `ERROR_DIR_NOT_EMPTY`.
pub const DIR_NOT_EMPTY: SystemError = win32(145, message_id("The directory is not empty."));
/// `ERROR_ALREADY_EXISTS`.
pub const ALREADY_EXISTS: SystemError = win32(
    183,
    message_id("Cannot create a file when that file already exists."),
);
/// `ERROR_FILENAME_EXCED_RANGE`.
pub const FILENAME_EXCED_RANGE: SystemError =
    win32(206, message_id("The filename or extension is too long."));
/// `ERROR_FILE_TOO_LARGE`.
pub const FILE_TOO_LARGE: SystemError = win32(
    223,
    message_id("The file size exceeds the limit allowed and cannot be saved."),
);
/// `ERROR_DIRECTORY`.
pub const DIRECTORY: SystemError = win32(267, message_id("The directory name is invalid."));
/// `ERROR_DIRECTORY_NOT_SUPPORTED`.
pub const DIRECTORY_NOT_SUPPORTED: SystemError =
    win32(336, message_id("An operation is not supported on a directory."));
/// `ERROR_IO_DEVICE`.
pub const IO_DEVICE: SystemError = win32(
    1117,
    message_id("The request could not be performed because of an I/O device error."),
);
/// `ERROR_NO_ASSOCIATION`.
pub const NO_ASSOCIATION: SystemError = win32(
    1155,
    message_id("No application is associated with the specified file for this operation."),
);
/// `ERROR_DEVICE_NOT_CONNECTED`, what Windows reports for a drive pulled
/// out mid-copy.
pub const DEVICE_NOT_CONNECTED: SystemError = win32(1167, message_id("The device is not connected."));
/// `ERROR_CANCELLED`.
pub const CANCELLED: SystemError = win32(1223, message_id("The operation was canceled by the user."));
/// `ERROR_CONNECTION_REFUSED`.
pub const CONNECTION_REFUSED: SystemError = win32(
    1225,
    message_id("The remote computer refused the network connection."),
);
/// `ERROR_NETWORK_UNREACHABLE`. Windows goes on "For information about
/// network troubleshooting, see Windows Help.", which is not here.
pub const NETWORK_UNREACHABLE: SystemError =
    win32(1231, message_id("The network location cannot be reached."));
/// `ERROR_HOST_UNREACHABLE`.
pub const HOST_UNREACHABLE: SystemError = win32(1232, message_id("The network location cannot be reached."));
/// `ERROR_CONNECTION_ABORTED`.
pub const CONNECTION_ABORTED: SystemError = win32(
    1236,
    message_id("The network connection was aborted by the local system."),
);
/// `ERROR_DISK_QUOTA_EXCEEDED`.
pub const DISK_QUOTA_EXCEEDED: SystemError = win32(
    1295,
    message_id(
        "The requested file operation failed because the storage quota was exceeded. To free up disk space, move files to a different location or delete unnecessary files. For more information, contact your system administrator.",
    ),
);
/// `ERROR_LOGON_FAILURE`.
pub const LOGON_FAILURE: SystemError = win32(1326, message_id("The user name or password is incorrect."));
/// `ERROR_FILE_CORRUPT`.
pub const FILE_CORRUPT: SystemError = win32(
    1392,
    message_id("The file or directory is corrupted and unreadable."),
);
/// `ERROR_TIMEOUT`.
pub const TIMEOUT: SystemError = win32(
    1460,
    message_id("This operation returned because the timeout period expired."),
);
/// `ERROR_CANT_RESOLVE_FILENAME`, a link that leads in a loop.
pub const CANT_RESOLVE_FILENAME: SystemError = win32(
    1921,
    message_id("The name of the file cannot be resolved by the system."),
);
/// `ERROR_NOT_A_REPARSE_POINT`.
pub const NOT_A_REPARSE_POINT: SystemError =
    win32(4390, message_id("The file or directory is not a reparse point."));
/// `E_FAIL`, for anything else.
pub const UNSPECIFIED: SystemError = SystemError {
    hresult: E_FAIL,
    sentence: message_id("Unspecified error"),
};

/// Every error, for finding one by its words.
const ALL: [SystemError; 41] = [
    INVALID_FUNCTION,
    FILE_NOT_FOUND,
    PATH_NOT_FOUND,
    TOO_MANY_OPEN_FILES,
    ACCESS_DENIED,
    INVALID_HANDLE,
    NOT_ENOUGH_MEMORY,
    INVALID_DATA,
    NOT_SAME_DEVICE,
    WRITE_PROTECT,
    NOT_READY,
    SHARING_VIOLATION,
    NOT_SUPPORTED,
    BAD_NETPATH,
    UNEXPECTED_NETWORK_ERROR,
    NETNAME_DELETED,
    BAD_NET_NAME,
    INVALID_PARAMETER,
    DISK_FULL,
    SEM_TIMEOUT,
    INVALID_NAME,
    DIR_NOT_EMPTY,
    ALREADY_EXISTS,
    FILENAME_EXCED_RANGE,
    FILE_TOO_LARGE,
    DIRECTORY,
    DIRECTORY_NOT_SUPPORTED,
    IO_DEVICE,
    NO_ASSOCIATION,
    DEVICE_NOT_CONNECTED,
    CANCELLED,
    CONNECTION_REFUSED,
    NETWORK_UNREACHABLE,
    HOST_UNREACHABLE,
    CONNECTION_ABORTED,
    DISK_QUOTA_EXCEEDED,
    LOGON_FAILURE,
    FILE_CORRUPT,
    TIMEOUT,
    CANT_RESOLVE_FILENAME,
    UNSPECIFIED,
];

impl SystemError {
    /// The error Windows reports for the GIO error `error`. Its `errno`
    /// decides when GIO's message ends with that `errno`'s words, as the
    /// local and `GVfs` backends' messages do; otherwise its GIO code.
    pub fn from_glib(error: &glib::Error) -> Self {
        glib::g_debug!(crate::LOG_DOMAIN, "System error: {error}");
        if let Some(found) = errno_in_message(error.message()) {
            return found;
        }
        if let Some(code) = error.kind::<gio::IOErrorEnum>() {
            return Self::from_gio_code(code);
        }
        if let Some(code) = error.kind::<glib::FileError>() {
            return Self::from_file_error(code);
        }
        if let Some(code) = error.kind::<gio::ResolverError>() {
            return match code {
                gio::ResolverError::NotFound | gio::ResolverError::TemporaryFailure => BAD_NETPATH,
                _ => UNEXPECTED_NETWORK_ERROR,
            };
        }
        UNSPECIFIED
    }

    /// The error Windows reports for `error` from mounting or reaching a
    /// share: a share the server does not have, and a server that
    /// refuses, cannot be reached or does not answer, is "The network name
    /// cannot be found." (as Windows 11 said it for an address with nothing
    /// on it), a name that does not resolve "The network path was not
    /// found.", and a server that went away "The specified network name is
    /// no longer available.", as Windows says them.
    pub fn from_network(error: &glib::Error) -> Self {
        Self::from_glib(error).on_network()
    }

    /// This error as Windows reports it for a share it mounts or reaches,
    /// as [`SystemError::from_network`] says.
    #[must_use]
    pub fn on_network(self) -> Self {
        match self {
            FILE_NOT_FOUND | PATH_NOT_FOUND => BAD_NET_NAME,
            // Windows 11 said 0x80070043 for an address with nothing on
            // it (checked on a VM).
            CONNECTION_REFUSED | HOST_UNREACHABLE | NETWORK_UNREACHABLE | SEM_TIMEOUT | TIMEOUT => {
                BAD_NET_NAME
            }
            // Samba's word for a server that went away.
            CONNECTION_ABORTED => NETNAME_DELETED,
            found => found,
        }
    }

    /// The error Windows reports for the local I/O error `error`.
    pub fn from_io(error: &std::io::Error) -> Self {
        glib::g_debug!(crate::LOG_DOMAIN, "System error: {error}");
        if let Some(found) = error.raw_os_error().and_then(Self::from_errno) {
            return found;
        }
        // An error made from one of these sentences, as a GIO error is
        // carried as local I/O, keeps it.
        if let Some(found) = error
            .get_ref()
            .and_then(|inner| Self::from_sentence(&inner.to_string()))
        {
            return found;
        }
        match error.kind() {
            std::io::ErrorKind::NotFound => FILE_NOT_FOUND,
            std::io::ErrorKind::PermissionDenied => ACCESS_DENIED,
            std::io::ErrorKind::AlreadyExists => ALREADY_EXISTS,
            std::io::ErrorKind::StorageFull => DISK_FULL,
            std::io::ErrorKind::ReadOnlyFilesystem => WRITE_PROTECT,
            std::io::ErrorKind::ResourceBusy | std::io::ErrorKind::ExecutableFileBusy => SHARING_VIOLATION,
            std::io::ErrorKind::InvalidFilename => FILENAME_EXCED_RANGE,
            std::io::ErrorKind::InvalidInput => INVALID_PARAMETER,
            std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof => INVALID_DATA,
            std::io::ErrorKind::TimedOut => SEM_TIMEOUT,
            std::io::ErrorKind::Unsupported => NOT_SUPPORTED,
            std::io::ErrorKind::OutOfMemory => NOT_ENOUGH_MEMORY,
            std::io::ErrorKind::Interrupted => CANCELLED,
            _ => UNSPECIFIED,
        }
    }

    /// The error Windows reports for the C library's `errno`, `None` for
    /// one it has no counterpart for.
    pub fn from_errno(errno: i32) -> Option<Self> {
        use rustix::io::Errno;
        let errno = Errno::from_raw_os_error(errno);
        let found = match errno {
            Errno::PERM | Errno::ACCESS => ACCESS_DENIED,
            Errno::NOENT => FILE_NOT_FOUND,
            Errno::EXIST => ALREADY_EXISTS,
            Errno::NOSPC => DISK_FULL,
            Errno::DQUOT => DISK_QUOTA_EXCEEDED,
            Errno::ROFS => WRITE_PROTECT,
            Errno::NAMETOOLONG => FILENAME_EXCED_RANGE,
            Errno::ILSEQ => INVALID_NAME,
            Errno::INVAL => INVALID_PARAMETER,
            Errno::BUSY | Errno::TXTBSY => SHARING_VIOLATION,
            Errno::IO => IO_DEVICE,
            Errno::NOTDIR => DIRECTORY,
            Errno::ISDIR => DIRECTORY_NOT_SUPPORTED,
            Errno::NOTEMPTY => DIR_NOT_EMPTY,
            Errno::XDEV => NOT_SAME_DEVICE,
            Errno::NOTSUP | Errno::NOSYS => NOT_SUPPORTED,
            Errno::NOMEM => NOT_ENOUGH_MEMORY,
            Errno::MFILE | Errno::NFILE => TOO_MANY_OPEN_FILES,
            Errno::BADF => INVALID_HANDLE,
            Errno::FBIG => FILE_TOO_LARGE,
            Errno::LOOP => CANT_RESOLVE_FILENAME,
            Errno::TIMEDOUT => SEM_TIMEOUT,
            Errno::CONNREFUSED => CONNECTION_REFUSED,
            Errno::HOSTUNREACH | Errno::HOSTDOWN => HOST_UNREACHABLE,
            Errno::NETUNREACH | Errno::NETDOWN => NETWORK_UNREACHABLE,
            Errno::CONNRESET | Errno::PIPE | Errno::STALE => NETNAME_DELETED,
            Errno::CONNABORTED => CONNECTION_ABORTED,
            // A FUSE file system whose process died, or a USB drive pulled
            // out: "Transport endpoint is not connected", "No such device".
            Errno::NOTCONN | Errno::NODEV | Errno::NXIO | Errno::SHUTDOWN => DEVICE_NOT_CONNECTED,
            Errno::NOMEDIUM => NOT_READY,
            Errno::UCLEAN => FILE_CORRUPT,
            Errno::CANCELED => CANCELLED,
            Errno::REMOTEIO => UNEXPECTED_NETWORK_ERROR,
            _ => return None,
        };
        Some(found)
    }

    /// The error Windows reports for the GIO error code `code`.
    pub fn from_gio_code(code: gio::IOErrorEnum) -> Self {
        use gio::IOErrorEnum as Io;
        match code {
            Io::NotFound => FILE_NOT_FOUND,
            Io::Exists | Io::WouldMerge => ALREADY_EXISTS,
            Io::IsDirectory => DIRECTORY_NOT_SUPPORTED,
            Io::NotDirectory => DIRECTORY,
            Io::NotEmpty => DIR_NOT_EMPTY,
            Io::NotSymbolicLink => NOT_A_REPARSE_POINT,
            Io::FilenameTooLong => FILENAME_EXCED_RANGE,
            Io::InvalidFilename => INVALID_NAME,
            Io::TooManyLinks => CANT_RESOLVE_FILENAME,
            Io::NoSpace => DISK_FULL,
            Io::InvalidArgument => INVALID_PARAMETER,
            Io::PermissionDenied => ACCESS_DENIED,
            Io::NotSupported | Io::NotRegularFile | Io::NotMountableFile => NOT_SUPPORTED,
            Io::NotMounted => NOT_READY,
            Io::Closed => INVALID_HANDLE,
            Io::Cancelled | Io::FailedHandled => CANCELLED,
            Io::ReadOnly => WRITE_PROTECT,
            Io::TimedOut => SEM_TIMEOUT,
            Io::Busy => SHARING_VIOLATION,
            Io::HostNotFound => BAD_NETPATH,
            Io::TooManyOpenFiles => TOO_MANY_OPEN_FILES,
            Io::NotInitialized => INVALID_FUNCTION,
            Io::InvalidData | Io::PartialInput => INVALID_DATA,
            Io::HostUnreachable => HOST_UNREACHABLE,
            Io::NetworkUnreachable
            | Io::ProxyFailed
            | Io::ProxyAuthFailed
            | Io::ProxyNeedAuth
            | Io::ProxyNotAllowed => NETWORK_UNREACHABLE,
            Io::ConnectionRefused => CONNECTION_REFUSED,
            // `CONNECTION_CLOSED` is `BROKEN_PIPE` by another name.
            Io::BrokenPipe => NETNAME_DELETED,
            Io::NotConnected | Io::NoSuchDevice => DEVICE_NOT_CONNECTED,
            _ => UNSPECIFIED,
        }
    }

    /// The error Windows reports for the `GLib` file error `code`, which
    /// is an `errno` by another name.
    fn from_file_error(code: glib::FileError) -> Self {
        use glib::FileError as File;
        match code {
            File::Exist => ALREADY_EXISTS,
            File::Isdir => DIRECTORY_NOT_SUPPORTED,
            File::Acces | File::Perm => ACCESS_DENIED,
            File::Nametoolong => FILENAME_EXCED_RANGE,
            File::Noent => FILE_NOT_FOUND,
            File::Notdir => DIRECTORY,
            File::Nxio | File::Nodev => DEVICE_NOT_CONNECTED,
            File::Rofs => WRITE_PROTECT,
            File::Txtbsy => SHARING_VIOLATION,
            File::Loop => CANT_RESOLVE_FILENAME,
            File::Nospc => DISK_FULL,
            File::Nomem => NOT_ENOUGH_MEMORY,
            File::Mfile | File::Nfile => TOO_MANY_OPEN_FILES,
            File::Badf => INVALID_HANDLE,
            File::Inval => INVALID_PARAMETER,
            File::Pipe => NETNAME_DELETED,
            File::Io => IO_DEVICE,
            File::Nosys => NOT_SUPPORTED,
            _ => UNSPECIFIED,
        }
    }

    /// The error whose sentence, in the interface's language, is `text`:
    /// a message that is one of these errors' sentences already.
    pub fn from_sentence(text: &str) -> Option<Self> {
        ALL.into_iter()
            .find(|error| error.sentence == text || gettext(error.sentence) == text)
    }

    /// The code Windows shows with it, such as `0x80070005`.
    pub fn hresult(self) -> u32 {
        self.hresult
    }

    /// Its sentence in the interface's language, such as "Access is
    /// denied.".
    pub fn message(self) -> String {
        gettext(self.sentence)
    }

    /// The error code line of Windows' "Interrupted Action": "Error
    /// 0x80070005: Access is denied."
    pub fn code_line(self) -> String {
        format_message(
            "Error {code}: {message}",
            &[
                ("code", &format!("0x{:08X}", self.hresult)),
                ("message", &self.message()),
            ],
        )
    }
}

/// The message of the GIO error `error` in Windows' words: what the
/// interface shows for a failure GIO, GTK or `GLib` reported.
pub fn message_of(error: &glib::Error) -> String {
    SystemError::from_glib(error).message()
}

/// `error` in Windows' words, naming the item GIO named, as a path:
/// "/home/ada/Locked/one.txt: Access is denied." for GIO's "Error opening
/// file “/home/ada/Locked/one.txt”: Permission denied". An error that
/// names no item is its sentence alone.
pub fn describe(error: &glib::Error) -> String {
    let message = message_of(error);
    match quoted_location(error.message()) {
        Some(location) => format!("{location}: {message}"),
        None => message,
    }
}

/// The local I/O error `error` in Windows' words. The system's own error
/// is its sentence; one the app made with a message keeps that message,
/// with the item it names, and only the system's words at its end become
/// Windows': "/home/ada/Locked/New document.txt: Access is denied.".
pub fn describe_io(error: &std::io::Error) -> String {
    let found = SystemError::from_io(error);
    if error.raw_os_error().is_some() {
        return found.message();
    }
    let Some(inner) = error.get_ref().map(ToString::to_string) else {
        return found.message();
    };
    if SystemError::from_sentence(&inner).is_some() {
        return inner;
    }
    let (named, words) = match inner.rsplit_once(": ") {
        Some((named, words)) => (Some(named), words),
        None => (None, inner.as_str()),
    };
    match errno_in_message(words) {
        Some(system) => match named {
            Some(named) => format!("{named}: {}", system.message()),
            None => system.message(),
        },
        None => inner,
    }
}

/// The file GIO's message names, as a path when it is a local one: GIO
/// quotes the path or URI it failed on (“/home/…”), or, in some of the
/// local backend's messages, writes the path and then its URI in brackets
/// ("Error opening file /home/… (file:///home/…): Permission denied").
fn quoted_location(message: &str) -> Option<String> {
    if !message.contains('“') {
        let (named, _) = message.rsplit_once(": ")?;
        let path = &named[named.find(" /")? + 1..];
        let path = match path.rsplit_once(" (") {
            Some((path, uri)) if uri.ends_with(')') => path,
            _ => path,
        };
        return Some(path.to_owned());
    }
    let start = message.find('“')? + '“'.len_utf8();
    let length = message[start..].find('”')?;
    let quoted = &message[start..start + length];
    if quoted.is_empty() {
        return None;
    }
    if quoted.starts_with('/') {
        return Some(quoted.to_owned());
    }
    let file = gio::File::for_uri(quoted);
    Some(file.path().map_or_else(
        || file.parse_name().to_string(),
        |path| path.display().to_string(),
    ))
}
/// The error of the `errno` whose words end `message`, as in "Error
/// opening file “/x”: Permission denied" or "Input/output error", or end
/// it in brackets, as `GLib`'s "Failed to execute child process “x” (No
/// such file or directory)" does.
fn errno_in_message(message: &str) -> Option<SystemError> {
    let message = message.trim();
    // Rust's own words for an errno end with "(os error 13)".
    let message = match message.rsplit_once(" (os error ") {
        Some((words, code)) if code.ends_with(')') => words,
        _ => message,
    };
    let words = match message.strip_suffix(')').and_then(|rest| rest.rsplit_once(" (")) {
        Some((_, bracketed)) => bracketed,
        None => message.rsplit(": ").next().unwrap_or(message),
    };
    let words = words.trim();
    if words.is_empty() {
        return None;
    }
    // Linux's errno values run from 1 to 133.
    (1..=133).find_map(|errno| {
        let described = std::io::Error::from_raw_os_error(errno).to_string();
        let described = described
            .strip_suffix(&format!(" (os error {errno})"))
            .unwrap_or(&described);
        (described == words)
            .then(|| SystemError::from_errno(errno))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An error that names an item keeps it, as a path, before Windows'
    /// words; one of the app's own keeps its message.
    #[test]
    fn the_item_an_error_names_is_kept_as_a_path() {
        let gio = glib::Error::new(
            gio::IOErrorEnum::PermissionDenied,
            "Error creating directory “/home/ada/Locked/one.txt”: Permission denied",
        );
        assert_eq!(describe(&gio), "/home/ada/Locked/one.txt: Access is denied.");
        let uri = glib::Error::new(
            gio::IOErrorEnum::NotFound,
            "Error when getting information for file “file:///home/ada/b%20track.mp3”: No such file or directory",
        );
        assert_eq!(
            describe(&uri),
            "/home/ada/b track.mp3: The system cannot find the file specified."
        );
        let unquoted = glib::Error::new(
            gio::IOErrorEnum::PermissionDenied,
            "Error opening file /home/ada/tree/sub/b.txt (file:///home/ada/tree/sub/b.txt): Permission denied",
        );
        assert_eq!(describe(&unquoted), "/home/ada/tree/sub/b.txt: Access is denied.");
        let plain = glib::Error::new(gio::IOErrorEnum::NoSpace, "No space left on device");
        assert_eq!(describe(&plain), "There is not enough space on the disk.");
        let made = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "/home/ada/Locked/New document.txt: {}",
                std::io::Error::from_raw_os_error(13)
            ),
        );
        assert_eq!(
            describe_io(&made),
            "/home/ada/Locked/New document.txt: Access is denied."
        );
        let own = std::io::Error::other("The file dialog service did not start again.");
        assert_eq!(describe_io(&own), "The file dialog service did not start again.");
        assert_eq!(
            describe_io(&std::io::Error::from_raw_os_error(28)),
            "There is not enough space on the disk."
        );
    }
}
