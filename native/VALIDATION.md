# Browsing milestone validation — 2026-09-26

This records the checks run for the native browsing checkpoint. It does not
certify the unfinished replacement workflows listed in [ROADMAP.md](ROADMAP.md).
It is a dated record: the counts are those of that run, and the lint set and
test suites have grown since, so rerun the commands for current results.

| Check | Result |
| --- | --- |
| `python3 native/tools/check.py` | Passed: formatting, Clippy with the workspace lints of the time (the `all` group), 8 bridge-inventory tests, 345 Rust tests across 12 executables, and doctest invocation (no doctests defined). |
| `cargo build --workspace --locked --manifest-path native/Cargo.toml` | Passed for the debug executable. |
| Existing desktop Python regression suite | 649 tests passed with disposable home and XDG directories. |
| `python3 -m unittest discover -s tests -p 'test_release_source.py'` | 9 tests passed. |
| `python3 -m unittest discover -s tests -p 'test_public_data.py'` | 10 tests passed. |
| Native GTK captures | Light, dark and 1000-pixel window layouts visually inspected; size text fits and the inspector artwork uses its own rendering size. |
| `python3 native/parity/check.py --require-replacement` | Correctly blocked replacement: all 77 legacy bridge operations still need complete native workflow verification. |
| Public-data audit, including native captures, and repository audit | Failed solely on the existing screenshot fixture-source hash mismatch described below. |

The native runner used Rust 1.98.1 and real GTK/GIO libraries on Linux, with a
private Xvfb display and D-Bus session for each test executable. Application
settings and caches were isolated in temporary directories. This machine lacked
GTK development package metadata, so local builds used temporary pkg-config and
linker metadata for the installed runtime libraries. CI is configured for Rust
1.92.0 and Ubuntu's GTK development packages; that hosted run was not performed
as part of this local checkpoint.

The GTK scenario exercises asynchronous listing, cancellation and stale results,
sorting, native selection, filters, tab histories, refresh, filesystem monitor
updates, shared appearance and errors. Separate tests cover controller release,
Python/Rust settings locking, clipboard compatibility and transfer failure
recovery. Device transfer tests use simulated backends. Real SMB/phone behaviour,
Wayland, assistive technology and startup performance remain unverified.

The public-data audit reports `Screenshot fixture source has changed since
capture`. The saved base commit already had a mismatch between
`desktop/ui/app.js` and the published capture manifest's fixture-source hash.
All seven published screenshot image hashes still verify. The audit used no
private identifier rules, so its fingerprint check is limited; the three native
captures were also visually reviewed and use synthetic files. The mismatch
must be resolved by a proper recapture before release, not by updating hashes
without regenerating the images. No release package, source archive or public
deployment was produced for this checkpoint.
