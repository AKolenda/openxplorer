# Security and data-safety notes

Development build: test with disposable files and a non-critical SMB share.
Do not treat testing with mocks or browser fixtures as a filesystem safety audit.

The native UI is a local, ephemeral WebKit context, with a restrictive content
security policy, external navigation/download blocking, and a JSON message bridge.
File names are rendered as text rather than HTML. The application does not listen
on a localhost server. Passwords enter only the mount/authentication bridge and
Secret Service, not the metadata index or settings JSON. The OS may still ask to
unlock a protected keyring. An unavailable keyring restricts reuse to process memory.

Clipboard data is untrusted. Only supported file/SMB URIs are accepted. A pasted
operation is validated again and presents a duplicate policy. The clipboard and
cached filenames expose metadata to the local logged-in user/session.

The optional administrator mount helper is separate from the unprivileged GUI.
It runs only on explicit user invocation, confirms changes, refuses unsafe/root-
unowned configuration parents and existing files, and uses an explicit root-only
plaintext CIFS credential file. Do not copy that file into bug reports. Installation
scripts never invoke this helper or make OpenXplorer the default handler.

Copies are staged and existing names are not intentionally overwritten. No
permanent-delete fallback is used when Trash is unsupported. Cross-filesystem
cut/move is not implemented. Cancelled transfers or interrupted applications may
leave staging items requiring inspection. Do not remove unidentified temporary
files without checking them.

ZIP listing is read-only. Path traversal, unsafe names, symlink members, duplicate
ambiguous members, excessive central directories and oversized previews are rejected.
Opening one supported member creates a private temporary copy. It does not modify
the archive. Preview files are not an offline document cache.

Exposed snapshot/backup directories are protected from writes only within
OpenXplorer. This does not set immutable flags or protect against other applications.
Restoring a version creates a new copy, not an in-place overwrite of the live item.

When reporting bugs, include version and `winspace --check` output, but remove
passwords, usernames, confidential hostnames, file contents and private paths.

Folder size scans read metadata only, do not follow links, skip nested mounts and
snapshot collections, and have explicit entry/time limits. They are not an atomic
snapshot, file-content cache, or server-side ZFS space query. A remote read can
still take time to cancel. Results/partial metadata may reveal names and sizes
to the logged-in session, and are not a promise that the directory never changed.


0.7 additions: FileManager1 accepts only validated file/SMB location requests from
the same session bus; it does not execute arbitrary commands. The service is
opt-in, never installed as the global FileManager1 default. Third-party per-user
overrides are refused, and modified user files are preserved on disable. This does
not harden an already-compromised desktop session. It may remain alive with no
window after opting in, but has no network listener.

Tab handoff forwards a whitelist of navigation/selection fields, not passwords or
arbitrary JS. The source is kept until the destination sends ready. Native
compositor pointer delivery and new-window startup remain unverified here.

Brave sync is an explicit, one-time edit of detected native profiles. The browser
must stay closed. File ownership, symlinks, consent and unchanged original bytes
are checked; backups and writes are atomic/private. A process can start after a
check, so do not run Brave concurrently with sync. Preferences is an internal
browser format, not a guaranteed stable API. Full Preferences backups may contain
private metadata: keep them private and out of reports. Restore touches only
OpenXplorer-applied directory keys that still match; it never rolls back unrelated
newer settings. No enforced browser policy or process termination is used.
