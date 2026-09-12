# Release candidate 2: native update and installer fixes

Project website: **https://openxplorer.app**. The release includes a Next.js/pnpm
website configured for that domain. Buying/configuring the domain does not deploy
the website or create an APT repository; those are separate owner actions.

## Safe update on Zorin

Finish active copy/move/extraction operations. Install the new package, then ask
its new launcher to stop the old single-instance process and reopen the installed
build:

```sh
sudo apt install './openxplorer_1.0.0~rc2_all.deb'
openxplorer --restart
```

Restart closes the old windows. It requests the existing application's safe Quit
action over the session bus, waits for the exact bus owner to go away, and refuses
to force-stop active writes. It never runs `pkill`, targets unrelated Python
processes, or resets your pins/settings/keyring. If the old process refuses,
finish/cancel its operations and retry. Logging out and back in is an alternative
once your work is saved.

```sh
openxplorer --version
openxplorer --diagnose
```

Diagnostics show installed/running release and build IDs, without browsing paths
or credentials. A normal launch detects mismatched or legacy background versions
and offers a restart; service-only activation prints instructions rather than
opening an unsolicited dialog. The interface also checks the backend version
before issuing file actions. This prevents new on-disk JavaScript from silently
speaking to an older Python process after an in-place package update.

## Extra top row and unknown terminal action

The inspected rc1 package already included `openTerminal` and the per-window
`set_show_menubar(False)` call. The reported combination is consistent with an
older background process loading newer UI files; it is not proven solely by a
status-bar label. This release adds the checks above and stops registering the
GTK fallback application-menu model entirely. Launcher and in-app actions remain.

The main script is executed from its installed source path. The package's default
source timestamp also differs from earlier releases, avoiding reused timestamp-
based bytecode where a system has compiled it. The isolated native GTK harness
reproduces a 27-pixel menu row before the policy and no visible row afterward.

## Software icon, license and launcher search

The package now supplies all of these consistently:

- Desktop launcher with `GenericName=File Explorer` and file-manager search keywords.
- Installed yellow folder SVG and PNG icons matching the existing desktop ID.
- AGPL-3.0-only project license and CC0-1.0 metainfo license (different purposes).
- An AppStream *catalog* with `pkgname=openxplorer`, not only upstream metainfo.
- Homepage field in both Debian control metadata and AppStream.
- Machine-readable Debian copyright metadata plus the complete AGPL text and
  retained original attribution.

After installing, finish the Software transaction, close Software and reopen it
so it can load the new application catalog. The installer refreshes system icon,
desktop and AppStream caches when the associated tools are present; it does not
remove per-user caches or launch/kill Software on your behalf.

**Fresh, uninstalled local `.deb` previews remain a packaging-system limitation.**
GNOME Software's PackageKit route initially constructs a generic package from
package details. It cannot use metadata that is only available *inside an
uninstalled* package as if it were already a catalog. Before installation, a
machine without OpenXplorer catalog metadata can still show a gear or unknown
license. A signed APT repository with AppStream data (or a properly configured
Flatpak catalog) is the publication solution; a homepage field alone is not.
No repository, signing key, or service contact was fabricated or installed.

## ZIP engine and the extraction error report

OpenXplorer does **not** use Dolphin as an archive backend. ZIP parsing and
decompression use Python's maintained standard-library `zipfile` and codecs.
OpenXplorer owns destination policy: preflight member paths, reject traversal,
links and ambiguous names, cap entries/bytes/ratio, write to a private new staging
folder, publish without overwriting, and support cancellation. This code is a
security-sensitive responsibility; passing tests is not a vulnerability-free
claim. No archive security limit is disabled in this update.

Keep broader formats and encrypted archives in the distribution's archive
manager instead of extending an ad hoc decompressor. **Open in archive manager**
is now directly available in the extraction dialog as well as the ZIP browser.
It explicitly delegates to the registered external application.

A real remote-read defect was corrected: GIO can return fewer bytes than requested
without EOF. The seekable ZIP reader now accumulates short reads, checks
cancellation, and closes the stream if initialization fails. A real ZIP passed
through deliberately short reads in the regression test. The UI validates the
archive summary before enabling extraction and shows an actionable mismatch/error
message rather than relying on missing fields.

The exact text from the last extraction screenshot was not available during this
review. It is not claimed that the remote-read correction reproduces that exact
report. No supplied archive was examined. Retest after a full restart, and report
the exact text plus whether source/destination are local or SMB; no password is
needed. Unsupported/unsafe archives should not be bypassed by weakening limits.

## Validation boundary

Native GTK menu behavior and GApplication/session-bus restart behavior can be
exercised through the system libraries in isolated harnesses here. The actual
Python dispatcher and extractor are also exercised together against disposable
local files, including a browser-to-dispatch contract test. Those do not stand
in for full PyGObject/WebKit, Zorin Software, live NAS, or physical-desktop tests.
The website still requires a reviewed pnpm lockfile and connected production
build/audit before deployment. See the current TEST-REPORT.md.
