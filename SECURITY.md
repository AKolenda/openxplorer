# Security and data safety

OpenXplorer 1.1.3 is a volunteer-maintained file manager. It is not supported under a security response SLA. Report vulnerabilities privately through [GitHub Security Advisories](https://github.com/AKolenda/openxplorer/security/advisories/new), which is enabled for this repository. Do not post credentials or sensitive filesystem inventories in a public issue.

## Important boundaries

- The desktop WebKit view loads packaged local UI; it must not browse remote websites inside its privileged native bridge.
- SMB passwords belong in the system keyring/session secret store, not settings JSON or the search database. A keyring-unavailable fallback has different persistence limits.
- Search metadata exposes filenames and paths locally. Protect your account and review cached roots before sharing logs/backups.
- Persistent CIFS mount credentials are a separate root-readable plaintext file. Review the optional helper and configuration before using it.
- Downloads relocation and native Brave profile edits need explicit confirmation; quit Brave before modifying its preferences.
- Third-party files, archive members and SMB metadata are untrusted. Keep traversal, symlink, staging and explicit-conflict protections intact.
- The website is a separate static project. It cannot mount shares or invoke the desktop bridge. Its previews contain fixtures, not personal data.

## Reporting a problem

Prepare version, distro/desktop/session details, sanitized reproduction steps, the operation and expected/actual result. Work with disposable data. Never attach raw browser profiles, keyring exports, CIFS credential files or unredacted private paths.

## Before public deployment

Install and lock website dependencies on a connected machine; check current security advisories and run the real Next production build. Verify matching source availability and review target-machine integration. The Debian `Maintainer` field is still a placeholder address; private reports go through the advisory link above. Do not claim production safety from browser mock tests.

## 1.0 release review

See [the dated findings and limits](docs/SECURITY-REVIEW.md) and
[stable-release gates](docs/RELEASE-CHECKLIST.md). Run `pnpm security:source` (or
`python3 tools/security_sweep.py`) for the local project-specific checks. This
is not a complete dependency scanner or security certification. The 1.1.3
build is not a security certification either: keep checking the actual
dependency graph, the production website and native target-machine integrations
on each release, and report vulnerabilities through the contact above.
