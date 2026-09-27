// SPDX-License-Identifier: AGPL-3.0-only
//! Differential tests: every expected value below was produced by running
//! `normalise_location(value, home=Path('/home/demo'))` and
//! `require_item_uri(value)` from `desktop/core.py` (Python 3.12) with
//! `HOME=/home/demo`. `Error` rows carry a message `core.py` itself writes;
//! `Rejected` rows fail in Python's standard library with its own wording.

use std::path::Path;

use ox_core::location::{normalise_location, require_item_uri, LocationError};

/// What the Python function returned for one input.
#[derive(Debug)]
enum Expected {
    Uri(&'static str),
    Error(&'static str),
    Rejected,
}

use Expected::{Error, Rejected, Uri};

const HOME: &str = "/home/demo";

/// Ported from `desktop/core.py::normalise_location` (differential table).
const NORMALISE_CASES: &[(&str, Expected)] = &[
    ("/tmp/file", Uri("file:///tmp/file")),
    ("/tmp/a/../b/", Uri("file:///tmp/b")),
    ("/tmp/(a) b", Uri("file:///tmp/%28a%29%20b")),
    ("/tmp/Q3 #1?.txt", Uri("file:///tmp/Q3%20%231%3F.txt")),
    ("/", Uri("file:///")),
    ("~", Uri("file:///home/demo")),
    ("~/", Uri("file:///home/demo")),
    ("~/Docs", Uri("file:///home/demo/Docs")),
    ("~//etc", Uri("file:///etc")),
    ("Plans", Uri("file:///home/demo/Plans")),
    ("../../etc", Uri("file:///etc")),
    ("./x/./y", Uri("file:///home/demo/x/y")),
    (
        "\\\\NAS\\Team files\\Q3 #1",
        Uri("smb://nas/Team%20files/Q3%20%231"),
    ),
    ("//nas/share", Uri("smb://nas/share")),
    ("\\\\nas", Uri("smb://nas/")),
    (
        "\\\\u@nas\\share",
        Error("Use a server name without credentials, for example \\\\nas\\share."),
    ),
    (
        "\\\\nas:445\\share",
        Error("Use a server name without credentials, for example \\\\nas\\share."),
    ),
    ("file:///tmp/x", Uri("file:///tmp/x")),
    ("file://localhost/tmp/x", Uri("file:///tmp/x")),
    ("file://LOCALHOST/tmp", Uri("file:///tmp")),
    ("file:////tmp/x", Uri("file:////tmp/x")),
    ("file:///tmp/(a)!", Uri("file:///tmp/%28a%29%21")),
    (
        "file:///tmp/%C3%89t%C3%A9%20%231%3F.txt",
        Uri("file:///tmp/%C3%89t%C3%A9%20%231%3F.txt"),
    ),
    ("file:///tmp/Read me.txt", Uri("file:///tmp/Read%20me.txt")),
    ("file:///", Uri("file:///")),
    ("file:///tmp/%zz", Uri("file:///tmp/%25zz")),
    ("file:///a/b/%2F", Uri("file:///a/b")),
    ("file:", Error("A file URL must contain an absolute path.")),
    (
        "file:relative",
        Error("A file URL must contain an absolute path."),
    ),
    (
        "file://nas/share",
        Error("For network folders, use smb://server/share rather than file://server/…"),
    ),
    (
        "file:///tmp/a%0Ab",
        Error("Encoded control characters are not allowed."),
    ),
    ("file:///tmp/%FF", Rejected),
    (" file:///tmp/x ", Uri("file:///tmp/x")),
    ("file:///tmp/x\u{1f}", Uri("file:///tmp/x")),
    (
        "smb://NAS/Team%20files/100%25.pdf",
        Uri("smb://nas/Team%20files/100%25.pdf"),
    ),
    ("smb://ALPHA/", Uri("smb://alpha/")),
    ("smb://nas/work/", Uri("smb://nas/work")),
    ("smb://nas/a/./b/../c", Uri("smb://nas/a/c")),
    ("smb://nas:445/share", Uri("smb://nas:445/share")),
    ("smb://nas:0445/s", Uri("smb://nas:445/s")),
    ("smb://nas:/s", Uri("smb://nas/s")),
    ("smb://[FE80::1]/share", Uri("smb://[fe80::1]/share")),
    ("smb://[FE80::1]:445/s", Uri("smb://[fe80::1]:445/s")),
    ("smb://[::ffff:1.2.3.4]/s", Uri("smb://[::ffff:1.2.3.4]/s")),
    ("smb://nas", Uri("smb://nas/")),
    ("smb://NAS/a\\b", Uri("smb://nas/a/b")),
    ("smb://nas/%2e%2e/x", Uri("smb://nas/x")),
    ("smb://nas/a%2Fb", Uri("smb://nas/a/b")),
    ("smb://ÄB/x", Uri("smb://äb/x")),
    ("smb://nas/a?", Uri("smb://nas/a")),
    ("smb://nas/a#", Uri("smb://nas/a")),
    (
        "smb://nas/Team%20files/%C3%89t%C3%A9",
        Uri("smb://nas/Team%20files/%C3%89t%C3%A9"),
    ),
    (
        "smb://u:p@nas/share",
        Error("Do not put a username or password in the address. Use the OpenXplorer sign-in dialog."),
    ),
    (
        "smb://u@nas/share",
        Error("Do not put a username or password in the address. Use the OpenXplorer sign-in dialog."),
    ),
    (
        "smb://u%40nas/share",
        Error("Use an unescaped server name without credentials or control characters."),
    ),
    (
        "smb:///share",
        Error("Enter an SMB server name, for example smb://nas/Projects."),
    ),
    (
        "smb://nas/a%00b",
        Error("Encoded control characters are not allowed."),
    ),
    (
        "smb://nas/a#b",
        Error("In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path."),
    ),
    (
        "smb://nas/a?b",
        Error("In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path."),
    ),
    (
        "smb://nas%0a/share",
        Error("Use an unescaped server name without credentials or control characters."),
    ),
    ("smb://nas:99999/share", Error("Invalid SMB port.")),
    ("smb://nas:44x/share", Error("Invalid SMB port.")),
    ("smb://nas:+1/s", Error("Invalid SMB port.")),
    ("smb://a:b:c/s", Error("Invalid SMB port.")),
    ("smb://[not-v6]/share", Rejected),
    ("smb://[1.2.3.4]/s", Rejected),
    ("smb://x[fe80::1]/s", Rejected),
    ("smb://[fe80::1]x/s", Rejected),
    (
        "smb://na s/x",
        Error("Enter an SMB server name, for example smb://nas/Projects."),
    ),
    ("smb://a／b/x", Rejected),
    (
        "smb:nas/share",
        Error("Enter an SMB server name, for example smb://nas/Projects."),
    ),
    (
        "smb://[fe80::1%25eth0]/s",
        Error("Use an unescaped server name without credentials or control characters."),
    ),
    (
        "mtp://[usb:001,010]/Internal storage/DCIM",
        Uri("mtp://[usb:001,010]/Internal%20storage/DCIM"),
    ),
    (
        "gphoto2://[usb:001,002]/DCIM",
        Uri("gphoto2://[usb:001,002]/DCIM"),
    ),
    ("afc://00008020-001C/", Uri("afc://00008020-001C/")),
    ("MTP://[usb:001,010]", Uri("mtp://[usb:001,010]/")),
    ("mtp://[usb:001,010]//a/../b", Uri("mtp://[usb:001,010]/b")),
    ("afc://x", Uri("afc://x/")),
    (
        "mtp://user@device/DCIM",
        Error("Invalid connected-device identifier."),
    ),
    (
        "mtp://[usb:001,002/DCIM",
        Error("Invalid connected-device identifier."),
    ),
    (
        "mtp://[usb:001,002]/DCIM?mode=write",
        Error("A connected-device address must include a device identifier and path."),
    ),
    (
        "afc:///DCIM",
        Error("A connected-device address must include a device identifier and path."),
    ),
    (
        "mtp://[usb:001,002]/a%00b",
        Error("Encoded control characters are not allowed."),
    ),
    ("mtp://a b/x", Error("Invalid connected-device identifier.")),
    ("mtp://[a]b]/x", Error("Invalid connected-device identifier.")),
    (
        "mtp:/x",
        Error("A connected-device address must include a device identifier and path."),
    ),
    ("", Error("Enter a local folder path or an SMB address.")),
    ("   ", Error("Enter a local folder path or an SMB address.")),
    (
        "http://example.org",
        Error("Only local paths, smb:// locations and connected devices are supported in this build."),
    ),
    (
        "javascript:alert(1)",
        Error("Only local paths, smb:// locations and connected devices are supported in this build."),
    ),
    (
        "C:\\Windows",
        Error("Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share."),
    ),
    (
        "c:/x",
        Error("Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share."),
    ),
    (
        "trash:///",
        Error("Only local paths, smb:// locations and connected devices are supported in this build."),
    ),
    (
        "archive:///tmp/a.zip/file",
        Error("Only local paths, smb:// locations and connected devices are supported in this build."),
    ),
    (
        "a:b",
        Error("Only local paths, smb:// locations and connected devices are supported in this build."),
    ),
];

/// Ported from `desktop/core.py::require_item_uri` (differential table).
const REQUIRE_ITEM_CASES: &[(&str, Expected)] = &[
    ("/tmp/file", Uri("file:///tmp/file")),
    ("/tmp/a/../b/", Uri("file:///tmp/b")),
    ("/tmp/(a) b", Uri("file:///tmp/%28a%29%20b")),
    ("/tmp/Q3 #1?.txt", Uri("file:///tmp/Q3%20%231%3F.txt")),
    ("/", Uri("file:///")),
    ("~", Uri("file:///home/demo")),
    ("~/", Uri("file:///home/demo")),
    ("~/Docs", Uri("file:///home/demo/Docs")),
    ("~//etc", Uri("file:///etc")),
    ("Plans", Uri("file:///home/demo/Plans")),
    ("../../etc", Uri("file:///etc")),
    ("./x/./y", Uri("file:///home/demo/x/y")),
    ("\\\\NAS\\Team files\\Q3 #1", Uri("smb://nas/Team%20files/Q3%20%231")),
    ("//nas/share", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("\\\\nas", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("\\\\u@nas\\share", Error("Use a server name without credentials, for example \\\\nas\\share.")),
    ("\\\\nas:445\\share", Error("Use a server name without credentials, for example \\\\nas\\share.")),
    ("file:///tmp/x", Uri("file:///tmp/x")),
    ("file://localhost/tmp/x", Uri("file:///tmp/x")),
    ("file://LOCALHOST/tmp", Uri("file:///tmp")),
    ("file:////tmp/x", Uri("file:////tmp/x")),
    ("file:///tmp/(a)!", Uri("file:///tmp/%28a%29%21")),
    ("file:///tmp/%C3%89t%C3%A9%20%231%3F.txt", Uri("file:///tmp/%C3%89t%C3%A9%20%231%3F.txt")),
    ("file:///tmp/Read me.txt", Uri("file:///tmp/Read%20me.txt")),
    ("file:///", Uri("file:///")),
    ("file:///tmp/%zz", Uri("file:///tmp/%25zz")),
    ("file:///a/b/%2F", Uri("file:///a/b")),
    ("file:", Error("A file URL must contain an absolute path.")),
    ("file:relative", Error("A file URL must contain an absolute path.")),
    ("file://nas/share", Error("For network folders, use smb://server/share rather than file://server/…")),
    ("file:///tmp/a%0Ab", Error("Encoded control characters are not allowed.")),
    ("file:///tmp/%FF", Rejected),
    (" file:///tmp/x ", Uri("file:///tmp/x")),
    ("file:///tmp/x\u{1f}", Uri("file:///tmp/x")),
    ("smb://NAS/Team%20files/100%25.pdf", Uri("smb://nas/Team%20files/100%25.pdf")),
    ("smb://ALPHA/", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas/work/", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas/a/./b/../c", Uri("smb://nas/a/c")),
    ("smb://nas:445/share", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas:0445/s", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas:/s", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://[FE80::1]/share", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://[FE80::1]:445/s", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://[::ffff:1.2.3.4]/s", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://NAS/a\\b", Uri("smb://nas/a/b")),
    ("smb://nas/%2e%2e/x", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas/a%2Fb", Uri("smb://nas/a/b")),
    ("smb://ÄB/x", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas/a?", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas/a#", Error("Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.")),
    ("smb://nas/Team%20files/%C3%89t%C3%A9", Uri("smb://nas/Team%20files/%C3%89t%C3%A9")),
    ("smb://u:p@nas/share", Error("Do not put a username or password in the address. Use the OpenXplorer sign-in dialog.")),
    ("smb://u@nas/share", Error("Do not put a username or password in the address. Use the OpenXplorer sign-in dialog.")),
    ("smb://u%40nas/share", Error("Use an unescaped server name without credentials or control characters.")),
    ("smb:///share", Error("Enter an SMB server name, for example smb://nas/Projects.")),
    ("smb://nas/a%00b", Error("Encoded control characters are not allowed.")),
    ("smb://nas/a#b", Error("In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path.")),
    ("smb://nas/a?b", Error("In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path.")),
    ("smb://nas%0a/share", Error("Use an unescaped server name without credentials or control characters.")),
    ("smb://nas:99999/share", Error("Invalid SMB port.")),
    ("smb://nas:44x/share", Error("Invalid SMB port.")),
    ("smb://nas:+1/s", Error("Invalid SMB port.")),
    ("smb://a:b:c/s", Error("Invalid SMB port.")),
    ("smb://[not-v6]/share", Rejected),
    ("smb://[1.2.3.4]/s", Rejected),
    ("smb://x[fe80::1]/s", Rejected),
    ("smb://[fe80::1]x/s", Rejected),
    ("smb://na s/x", Error("Enter an SMB server name, for example smb://nas/Projects.")),
    ("smb://a／b/x", Rejected),
    ("smb:nas/share", Error("Enter an SMB server name, for example smb://nas/Projects.")),
    ("smb://[fe80::1%25eth0]/s", Error("Use an unescaped server name without credentials or control characters.")),
    ("mtp://[usb:001,010]/Internal storage/DCIM", Uri("mtp://[usb:001,010]/Internal%20storage/DCIM")),
    ("gphoto2://[usb:001,002]/DCIM", Uri("gphoto2://[usb:001,002]/DCIM")),
    ("afc://00008020-001C/", Error("Open the device storage first, then select files or folders inside it. The device itself cannot be moved or copied.")),
    ("MTP://[usb:001,010]", Error("Open the device storage first, then select files or folders inside it. The device itself cannot be moved or copied.")),
    ("mtp://[usb:001,010]//a/../b", Uri("mtp://[usb:001,010]/b")),
    ("afc://x", Error("Open the device storage first, then select files or folders inside it. The device itself cannot be moved or copied.")),
    ("mtp://user@device/DCIM", Error("Invalid connected-device identifier.")),
    ("mtp://[usb:001,002/DCIM", Error("Invalid connected-device identifier.")),
    ("mtp://[usb:001,002]/DCIM?mode=write", Error("A connected-device address must include a device identifier and path.")),
    ("afc:///DCIM", Error("A connected-device address must include a device identifier and path.")),
    ("mtp://[usb:001,002]/a%00b", Error("Encoded control characters are not allowed.")),
    ("mtp://a b/x", Error("Invalid connected-device identifier.")),
    ("mtp://[a]b]/x", Error("Invalid connected-device identifier.")),
    ("mtp:/x", Error("A connected-device address must include a device identifier and path.")),
    ("", Error("Enter a local folder path or an SMB address.")),
    ("   ", Error("Enter a local folder path or an SMB address.")),
    ("http://example.org", Error("Only local paths, smb:// locations and connected devices are supported in this build.")),
    ("javascript:alert(1)", Error("Only local paths, smb:// locations and connected devices are supported in this build.")),
    ("C:\\Windows", Error("Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share.")),
    ("c:/x", Error("Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share.")),
    ("trash:///", Error("Only local paths, smb:// locations and connected devices are supported in this build.")),
    ("archive:///tmp/a.zip/file", Error("Only local paths, smb:// locations and connected devices are supported in this build.")),
    ("a:b", Error("Only local paths, smb:// locations and connected devices are supported in this build.")),
];

fn check(name: &str, value: &str, actual: Result<String, LocationError>, expected: &Expected) {
    match (expected, actual) {
        (Uri(uri), Ok(actual)) => assert_eq!(actual, *uri, "{name}({value:?})"),
        (Error(message), Err(actual)) => assert_eq!(actual.0, *message, "{name}({value:?})"),
        (Rejected, Err(_)) => {}
        (expected, actual) => panic!("{name}({value:?}): expected {expected:?}, got {actual:?}"),
    }
}

/// Ported from `desktop/core.py::normalise_location`: the behaviour the
/// entry classifier, the clipboard and file drops rely on.
#[test]
fn normalise_matches_python() {
    for (value, expected) in NORMALISE_CASES {
        check(
            "normalise",
            value,
            normalise_location(value, None, Path::new(HOME)),
            expected,
        );
    }
}

/// Ported from `desktop/core.py::require_item_uri`.
#[test]
fn require_item_uri_matches_python() {
    for (value, expected) in REQUIRE_ITEM_CASES {
        let actual = normalise_location(value, None, Path::new(HOME)).and_then(|uri| require_item_uri(&uri));
        check("require_item_uri", value, actual, expected);
    }
}
