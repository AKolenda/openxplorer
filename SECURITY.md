# Security and data safety

OpenXplorer 2.x is a volunteer-maintained file manager. It is not supported under a security response SLA. Report vulnerabilities privately through [GitHub Security Advisories](https://github.com/AKolenda/openxplorer/security/advisories/new), which is enabled for this repository. Do not post credentials or sensitive filesystem inventories in a public issue.

## Important boundaries

- The app is a native Rust and GTK 4 program (`native/`). It has no web view and runs no HTML or JavaScript; the WebKit bridge of the deprecated 1.x app is gone.
- Other applications reach the running app through its command line and, once the user turns on "Show in folder", the `org.freedesktop.FileManager1` session-bus service. Its three methods (`ShowFolders`, `ShowItems`, `ShowItemProperties`) accept 1 to 100 local, SMB or device locations; other schemes and the app's own pages, such as Settings, are refused. Every location, from the bus or the command line, is normalised like an address typed in the address bar: it is data to show, never a command to run. Addresses may not carry a password; SMB and NFS addresses may not carry a user name either.
- "Check for updates" contacts only fixed GitHub HTTPS endpoints, and redirects must stay on them. Only the Debian package in `/opt/openxplorer` installs an update itself, after the user confirms: the installer is downloaded privately, checked against the release's size and SHA-256 digest and with `dpkg-deb`, installed through `pkexec apt-get`, and deleted. The digest is GitHub's, not an independent publisher signature. The Flatpak, RPM and Arch packages update through their own package manager.
- Opening a file never runs it. Only when "Ask whether to run programs and scripts" is turned on in Settings does opening a real program or script ask "Run this program?", with Open as the default answer; documents, photos and text files that merely have the executable bit open in their application.
- Dropping files onto a program, a script or a `.desktop` launcher runs it with the dropped paths as separate arguments, never through a shell command line; a launcher starts through GLib's `gio launch`. Nothing runs from a file that is not executable, a launcher must be trusted the way GNOME trusts one, and a program or launcher on a network share or a removable drive runs only after the user confirms "Run this program?".
- SMB passwords belong in the desktop's Secret Service keyring, under the 1.x schema, not in settings JSON or the search database. When the keyring is unavailable, a password is kept in memory for the session only.
- Search metadata exposes filenames and paths locally. Protect your account and review cached roots before sharing logs/backups.
- The persistent mount helper, `openxplorer-mount-share`, is a Rust program shipped by the stable distribution packages, not the Flatpak. The app never runs it and never gains privileges: it prints the `sudo` command for the administrator to review and run. The helper writes a root-only plaintext credential file and systemd units without replacing existing files. Review it before use.
- Downloads relocation and Brave profile edits need explicit confirmation; quit Brave before modifying its preferences.
- Third-party files, archive members and SMB metadata are untrusted. Keep traversal, symlink, staging and explicit-conflict protections intact.
- The website is a separate static project. It cannot mount shares or reach the desktop app. Its previews contain fixtures, not personal data.

## Reporting a problem

Prepare version, distro/desktop/session details, sanitized reproduction steps, the operation and expected/actual result. Work with disposable data. Never attach raw browser profiles, keyring exports, CIFS credential files or unredacted private paths.

## Before public deployment

Install and lock website dependencies on a connected machine; check current security advisories and run the real Next production build. Verify matching source availability and review target-machine integration. The packages name `OpenXplorer contributors <openxplorer@users.noreply.github.com>` as their maintainer, a no-reply address; private reports go through the advisory link above. Do not claim production safety from simulated tests.

## 1.0 release review

See [the dated findings and limits](docs/SECURITY-REVIEW.md) and
[stable-release gates](docs/RELEASE-CHECKLIST.md). Run `pnpm security:source` (or
`python3 tools/security_sweep.py`) for the local project-specific checks. This
is not a complete dependency scanner or security certification. The 2.x
builds are not a security certification either: keep checking the actual
dependency graph, the production website and native target-machine integrations
on each release, and report vulnerabilities through the contact above.
