# Packaging the native app

The native app ships as a Debian package, an RPM, an Arch package and a
Flatpak. Every format installs the same files through
[`tools/package_data.py`](../tools/package_data.py), and
[`tools/verify_layout.py`](../tools/verify_layout.py) checks them, so a desktop
entry, icon or licence cannot go missing from one format only.

| Format | Distributions | Built by | Release asset |
|---|---|---|---|
| Debian package | Ubuntu 24.04 and newer (Zorin OS 18), Debian 13 and newer | [`tools/build_deb.py`](../tools/build_deb.py) | `openxplorer_<version>_all.deb` (stable), `openxplorer-native_<version>_<arch>.deb` (preview) |
| RPM | Fedora, openSUSE Tumbleweed | [`rpm/openxplorer.spec`](rpm/openxplorer.spec) | `openxplorer[-native]-<version>-1.<dist>.<arch>.rpm` |
| Arch package | Arch Linux and its derivatives | [`arch/PKGBUILD`](arch/PKGBUILD) | `openxplorer[-native]-<version>-1-<arch>.pkg.tar.zst` |
| Flatpak | every distribution with Flatpak | [`flatpak/`](flatpak/) | `io.winspace.Development[.Native].flatpak` |

The distribution packages need GTK 4.14 or newer. **Debian 12** has GTK 4.8, so
its users install the Flatpak; the `.deb` refuses to install there because it
depends on `libgtk-4-1 (>= 4.14)`. The same applies to any other distribution
with an older GTK.

## Application ID: preview and stable

**Since 2.0.0 every release package is the stable channel**: `tools/release.py`
and the `native-package` and `native-flatpak` jobs of
`.github/workflows/checks.yml` build `io.winspace.Development`. The preview
remains the default of a plain build and of `native-distros.yml`.

Until the native app has every behaviour of the Python app
(`python3 native/parity/check.py --gate replace`), it is a **preview** that
installs beside the Python app. The release that replaces the Python app builds
the **stable** channel instead.

| | Preview | Stable |
|---|---|---|
| Application ID | `io.winspace.Development.Native` | `io.winspace.Development` (the Python app's) |
| Package and command | `openxplorer-native` | `openxplorer`, plus the Python package's `winspace`, `openxplorer-mount-share` and `winspace-mount-share` |
| Desktop entry | no MIME types, one quick action | the Python package's, key for key |
| Beside the Python app | yes | replaces it |

The application ID is a compatibility contract (AGENTS.md): the desktop entry,
the AppStream ID, the Flatpak ID and the D-Bus name the program registers must
agree. `build.rs` in `crates/ox-app` reads `OX_APP_ID` at build time, accepts
only these two IDs, and defaults to the preview's, so a plain `cargo build`
never takes over the Python app's name. Every package build sets it.

## Installed files

`package_data.py` installs, for the application ID `<id>` and the command
`<command>` (`openxplorer-native` or `openxplorer`):

| File | Debian | RPM and Arch | Flatpak |
|---|---|---|---|
| Program | `/opt/<command>/bin/<command>` | `/usr/bin/<command>` | `/app/bin/<command>` |
| Command | `/usr/bin/<command>`, a link to the program | the program | the program |
| Desktop entry | `/usr/share/applications/<id>.desktop` | same | `/app/share/applications/<id>.desktop` |
| AppStream metainfo | `/usr/share/metainfo/<id>.metainfo.xml` | same | under `/app/share` |
| Launcher icon | `/usr/share/icons/hicolor/scalable/apps/<id>.svg` | same | under `/app/share` |
| D-Bus service file | `/usr/share/dbus-1/services/<id>.service` | same | under `/app/share` |
| Licences | `/usr/share/doc/<command>/` | `/usr/share/licenses/<command>/` | `/app/share/licenses/<id>/` |
| Mount helper (stable) | `/opt/openxplorer/bin/openxplorer-mount-share`, linked from `/usr/bin` | `/usr/bin/openxplorer-mount-share` | none |

- **Program folder.** The Debian program lives in `/opt/openxplorer/bin`
  because the in-app updater allows installing updates only for that folder
  (`Installation::detect` in `crates/ox-core/src/update/installation.rs`, which
  ports `can_install` of `desktop/updater.py`). An RPM or Arch program in
  `/usr/bin` counts as "installed by another package manager", which updates it.
- **Licences.** The AGPL, `THIRD_PARTY_NOTICES.md`, every text in `licenses/`,
  and the licence files of each Rust crate linked into the program, with an
  index (`rust-crates/INDEX.txt`). The Debian package adds a machine-readable
  `copyright`.
- **Launcher icon.** The Fluent Emoji file folder the app shows for folders,
  byte for byte, following the rule that every icon is an unmodified Fluent
  file. It is an SVG, which GNOME, KDE, GNOME Software and Flatpak all accept.
- **Mount helper.** `openxplorer-mount-share`, the port of
  `desktop/mount_share.py`, is a second Rust program
  (`crates/ox-core/src/bin/openxplorer-mount-share.rs`) with the same command
  line. Only the stable host packages install it, with the Python package's
  legacy name `winspace-mount-share` as a link: the preview installs beside
  the Python package, which owns that command, and a Flatpak cannot add host
  commands. The mount assistant in a folder's Location tab prints
  `sudo /usr/bin/openxplorer-mount-share …` for the administrator to run; the
  app never runs it. No package ships Python.

## Desktop integration data

- **Desktop entries** ([`data/`](data/)). The stable entry is the Python
  package's (`DESKTOP` in `desktop/tools/build_deb.py`) key for key: name,
  `GenericName=File Explorer`, keywords, `Exec=openxplorer %U`, the icon, the
  MIME types `inode/directory`, `x-scheme-handler/smb` and the three ZIP types,
  `StartupWMClass=io.winspace.Development` and the quick actions New window,
  Open windows… and Settings. Declaring MIME types makes OpenXplorer *offer*
  to open folders and ZIP files; it becomes a default only when the user asks
  in Settings. The preview's entry declares no MIME type. Neither sets
  `DBusActivatable`: the quick actions pass command-line options, which D-Bus
  activation would replace with action calls under other names.
- **AppStream metainfo** ([`data/`](data/)). The stable metainfo keeps the
  Python app's ID, name, licences and release history, so GNOME Software shows
  one application across the switch; each release adds its `<release>` at the
  top, and `verify_layout.py` requires the newest release to be the package
  version. The preview's releases are marked `development`. The Debian package
  also installs a one-application catalog in `/usr/share/swcatalog/xml` that
  maps the package name to the application, as the Python package does.
- **D-Bus service file** ([`data/dbus-service.in`](data/dbus-service.in)). It
  lets the session bus start the app under its own ID with
  `--gapplication-service` when something calls that name, such as
  `gapplication launch` or a notification action; the app then opens a window
  only when asked. Flatpak exports it with a `flatpak run` command.
- **No FileManager1 service.** No package installs
  `org.freedesktop.FileManager1.service`: a system-wide file would take
  "Show in folder" requests from every user without asking. The opt-in
  integration in Settings writes the per-user file, which runs
  `/usr/bin/winspace --filemanager-service`, so the stable packages keep that
  command.

## Dependency grouping

Each format expresses the same three groups in its own package names.

**Required.** The libraries the program links, found from the program itself:
`dpkg-shlibdeps` for the `.deb` (with GTK raised to 4.14, the oldest version
the app is built and tested against), RPM's automatic library dependencies, and
the PKGBUILD's hand-written list (which also names SQLite and libsoup, which the
program links once its interface uses search and updates). Plus
`hicolor-icon-theme`, which owns the icon folders.

**Recommended** (installed by default by APT, DNF and zypper; optional
dependencies in Arch). What features beyond local browsing need; the app
explains a missing one when that feature is used:

| Feature | Debian | Fedora | openSUSE | Arch |
|---|---|---|---|---|
| SMB shares, phones, Recycle Bin, drive list | `gvfs`, `gvfs-backends`, `gvfs-fuse` | `gvfs`, `gvfs-smb`, `gvfs-mtp`, `gvfs-fuse` | `gvfs`, `gvfs-backends`, `gvfs-backend-samba`, `gvfs-fuse` | `gvfs`, `gvfs-smb`, `gvfs-mtp` |
| Remembering SMB passwords (Secret Service) | `gnome-keyring \| keepassxc` | `gnome-keyring` | `gnome-keyring` | `gnome-keyring` |
| Making OpenXplorer the default file manager | `xdg-utils` | `xdg-utils` | `xdg-utils` | `xdg-utils` |
| Open in Terminal | `gnome-terminal \| x-terminal-emulator` | (every desktop has one) | (every desktop has one) | (every desktop has one) |
| Open in archive manager | `file-roller` | `file-roller` | `file-roller` | `file-roller` |
| In-app updates (stable `.deb` only) | `pkexec` | | | |
| Persistent SMB mount helper (stable only) | `cifs-utils` | `cifs-utils` | `cifs-utils` | `cifs-utils` |

**Not dependencies.** The Python package depended on Python, PyGObject,
WebKitGTK, libsecret and `xdg-user-dirs`; the native programs need none of
them (the Secret Service client is pure Rust, and the mount helper is a Rust
program). Python is only a build tool: `tools/package_data.py` installs the
files. RPM and Arch have no virtual
terminal package, so they name none.

## Debian package

```sh
python3 native/tools/build_deb.py                      # the preview
python3 native/tools/build_deb.py --app-id io.winspace.Development   # the stable app
python3 native/tools/verify_deb.py dist/native/<package>.deb
```

It needs Cargo, `dpkg-deb` and `dpkg-shlibdeps` (`sudo apt install dpkg-dev`).
Build release packages on Ubuntu 24.04, the oldest supported base:
`dpkg-shlibdeps` records the library versions of the build system.

- **"Architecture: all" and the preinst.** The 1.1.x updater downloads only
  `openxplorer_<version>_all.deb` and installs it only if its fields are
  exactly Package `openxplorer`, the release's version and Architecture `all`.
  The stable package therefore says `all` although its program is built for
  one processor, and its preinst
  ([`debian/preinst.in`](debian/preinst.in)) stops the installation on any
  other processor before a file changes, pointing to the Flatpak. The preview
  names its real architecture.
- **Maintainer scripts.** postinst and postrm
  ([`debian/refresh-caches`](debian/refresh-caches)) refresh the application,
  icon and AppStream caches, as the Python package's do, and nothing else:
  installing never changes defaults, user data, passwords, folder locations or
  mounts. dpkg triggers do not refresh the AppStream cache for the swcatalog
  catalog, which is why the scripts exist.
- **Relations.** The stable package replaces, conflicts with and provides
  `winspace-explorer (<< 0.8.0)`, as the Python package does.
- **Reproducible.** File times come from `SOURCE_DATE_EPOCH` or the last
  commit, every entry is owned by root, directories are 0755, files 0644 or
  0755, `DEBIAN/md5sums` lists every file, and the payload is xz-compressed.
- **Verification.** `verify_deb.py` checks the identity and dependencies, that
  the payload cannot escape the package (no absolute or traversing paths, no
  links out of it, root-owned, nothing group- or world-writable), the scripts,
  the checksums, the processor, the catalog and every installed file
  (`verify_layout.py`); for the stable package it also runs the real
  `Updater.check` and `Updater.install` of `desktop/updater.py` against it,
  with only the network, the administrator prompt and the package database
  simulated.

## Moving existing users to the native app

The Python app's updater and the native updater (`crates/ox-core/src/update/`)
accept exactly one asset: **`openxplorer_<version>_all.deb`**, published at
`https://github.com/<owner>/openxplorer/releases/download/v<version>/`, from a
non-draft, non-prerelease release tagged `v<version>` with a newer
`MAJOR.MINOR.PATCH` version. Its control fields must be Package `openxplorer`,
Version `<version>` and Architecture `all`. The other assets (preview `.deb`,
RPMs, Arch packages, Flatpak bundles and the source archive) may sit beside it;
the updaters ignore them.

The release that replaces the Python app:

1. Passes `python3 native/parity/check.py --require-replacement --gate replace`.
   The installed app must accept the options the Python app's files and
   updater use: `--restart` (the Python updater runs
   `/usr/bin/openxplorer --restart` after installing), `--filemanager-service`
   (the per-user "Show in folder" files), `--new-window`, `--windows` and
   `--settings` (the desktop entry), and set its program name to the
   application ID so its X11 window class matches `StartupWMClass`.
2. Raises `[workspace.package] version` in `native/Cargo.toml` above the Python
   app's (`build_deb.py` refuses anything else), with the same version in
   `rpm/openxplorer.spec` and `arch/PKGBUILD` and a new `<release>` at the top of
   `data/io.winspace.Development.metainfo.xml`.
3. Builds the stable `.deb` on Ubuntu 24.04 and checks it:

   ```sh
   python3 native/tools/build_deb.py --app-id io.winspace.Development
   python3 native/tools/verify_deb.py dist/native/openxplorer_<version>_all.deb
   python3 native/tools/verify_upgrade.py --python openxplorer_1.1.4_all.deb \
       --stable dist/native/openxplorer_<version>_all.deb \
       --preview dist/native/openxplorer-native_<version>_amd64.deb
   ```

   `verify_upgrade.py` installs real packages into a disposable dpkg root inside
   a read-only bubblewrap sandbox: the Python package then the native one (no
   Python file may remain, `openxplorer` and `winspace` must run the native
   program, the desktop entry must keep every key), the Python package again
   (a rollback restores exactly its files), and the preview beside the Python
   package.
4. Publishes `openxplorer_<version>_all.deb` with its source archive and
   `SHA256SUMS`, never overwriting an existing asset.

**Rollback.** Installing the previous Python package restores it completely:
`sudo apt install --allow-downgrades ./openxplorer_1.1.4_all.deb`. Settings,
pins, the search cache and saved passwords are shared by both apps and stay.

## Flatpak

The manifests build on **GNOME 51**, the newest stable GNOME runtime, whose base
is Freedesktop 26.08, with the `rust-stable` SDK extension. flatpak-builder
builds without network access, so
[`flatpak/cargo-sources.json`](flatpak/cargo-sources.json) lists every crate of
`Cargo.lock` with its checksum; regenerate it after every `Cargo.lock` change
(a test fails until you do):

```sh
python3 native/tools/flatpak_cargo_sources.py
```

Build, install and run it for the current user only (no administrator rights):

```sh
flatpak install --user flathub org.flatpak.Builder org.gnome.Sdk//51 \
    org.freedesktop.Sdk.Extension.rust-stable//26.08
flatpak run org.flatpak.Builder --user --force-clean --repo=repo build \
    native/packaging/flatpak/io.winspace.Development.Native.yml
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    repo io.winspace.Development.Native.flatpak io.winspace.Development.Native
flatpak install --user io.winspace.Development.Native.flatpak
flatpak run io.winspace.Development.Native
```

### Flatpak permissions

| Permission | Why |
|---|---|
| `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc` | The window: Wayland, and X11 only without Wayland, which needs shared memory to draw at a usable speed. |
| `--device=dri` | GTK 4 renders with the GPU. |
| `--filesystem=host` | A file manager shows, copies and changes the user's files wherever they are: home, other disks under `/media`, `/run/media` and `/mnt`, `/opt`, `/srv`. |
| `--talk-name=org.gtk.vfs.*`, `--filesystem=xdg-run/gvfsd`, `--filesystem=xdg-run/gvfs` | GVfs: the sandbox's GIO asks the host's GVfs daemons for `smb://`, `mtp://`, `trash:///` and the drive and phone list, reaches their private sockets, and opens files on shares through their FUSE paths. |
| `--talk-name=org.freedesktop.secrets` | Saved SMB passwords live in the desktop's Secret Service under the Python app's schema, so both apps find each other's sign-ins. |
| `--talk-name=org.gnome.NautilusPreviewer` | Quick Look: Space asks GNOME Sushi on the host to preview the selected file, and the app follows its window. |
| `--own-name=org.freedesktop.FileManager1` | "Show in folder" (opt-in): after the user turns it on in Settings, the running app answers browsers' and other apps' requests to show a downloaded file in its folder. Flatpak only permits owning the name; the app claims it only while the integration is on. |
| `--talk-name=org.freedesktop.Flatpak` | `flatpak-spawn --host`: Open in Terminal starts the host's terminal, and making OpenXplorer the default file manager (opt-in) runs the host's `xdg-mime`. |
| `--share=network` | "Check for updates" asks GitHub whether a newer release exists. SMB and phone traffic goes through GVfs on the host. |

The desktop portals, which every Flatpak may use, cover opening files with other
applications, moving files to the Trash, notifications and the desktop's dark
style, so none of them needs a permission. A test
(`tools/test_flatpak.py`) keeps this table and the manifest's list equal.

### Differences inside the Flatpak

- Settings, pins and the search cache live in
  `~/.var/app/<id>/`, Flatpak's per-app folders, not in `~/.config/winspace`, so
  the Flatpak does not share them with a distribution package's app. Saved
  passwords are shared through the Secret Service.
- System folders (`/usr`, `/etc`) are the sandbox's own; the host's appear
  under `/run/host` only with the `host-os` and `host-etc` permissions, which
  the manifest does not request.
- "Show in folder" works while OpenXplorer runs, and after login if the desktop
  allows it. A Flatpak may not install the per-user D-Bus service file and
  autostart entry the host package writes, so turning it on writes only an
  opt-in record in the app's settings folder, claims
  `org.freedesktop.FileManager1` (the permission above), and asks the
  Background portal (`org.freedesktop.portal.Background`) to start the app at
  login without a window. When the desktop refuses or has no Background
  portal, a message says that Show in folder answers only while OpenXplorer
  runs. Nothing starts the app on demand when a request arrives
  (`crates/ox-core/src/integration/reveal.rs`,
  `crates/ox-core/src/integration/background_portal.rs`).
- Updates come from Flatpak (GNOME Software or `flatpak update`); the app
  never installs one itself.
- The System theme follows the desktop's light or dark style through the
  Settings portal (`org.freedesktop.appearance` `color-scheme`), not GNOME's
  `org.gnome.desktop.interface` keys: the runtime ships that schema too, but
  the sandbox cannot read the host's values, so it holds only defaults
  (`crates/ox-app/src/theme/system.rs`). With `color-scheme` at "default",
  the app looks for "dark" in the GTK theme's name, as the host package
  does; GNOME-based portals serve it (`org.gnome.desktop.interface`
  `gtk-theme`). Where the portal has neither, the Flatpak keeps GTK's own
  preference.

### Publishing on Flathub

The Flatpak is published as a bundle with each release. Flathub publication is
prepared as far as the repository allows; the rest needs the owner.

Already in place:

- The stable metainfo
  ([`data/io.winspace.Development.metainfo.xml`](data/io.winspace.Development.metainfo.xml))
  passes `appstreamcli validate`: licences, developer, launchable, stock icon,
  categories, keywords, homepage, help, source and bug tracker links, branding
  colours, a release entry with notes for every version, and an OARS 1.1
  content rating of social-info mild (the update check contacts GitHub's API
  for the latest release); no other attribute applies (no violence, drugs,
  sex, language, social chat, purchases or ads).
- The icon is scalable SVG, and the build is offline: every crate comes from
  [`flatpak/cargo-sources.json`](flatpak/cargo-sources.json).
- Every permission is explained above ("Flatpak permissions") and below.

Still to do:

- **Screenshots** (TODO in both metainfo files). Flathub requires at least one
  `<screenshot>` served over HTTPS. The website's screenshots
  (`apps/web/public/assets/screenshots/`) show the 1.x web interface with a
  "1.1.4 preview" label, so they are not used. Capture the native window on
  fictional files with the snapshot hook (`OPENXPLORER_SNAPSHOT`, following
  `docs/PRIVACY.md`), publish the images on the website, and add them with
  their captions.
- **Sources of the Flathub manifest.** Flathub builds the manifest from its
  `flathub/<app-id>` repository, where the `dir` source above must be the
  release's source archive (`openxplorer-<version>.tar.gz` from
  `native/tools/source_archive.py`) or a git tag, with its checksum.
- **Linter exceptions.** Run `flatpak run --command=flatpak-builder-lint
  org.flatpak.Builder manifest native/packaging/flatpak/io.winspace.Development.yml`
  and `... repo repo`, and ask for an exception for each finding in the
  submission, with the reasons below.

Only the owner can:

- **Verify the application ID.** Flathub accepts `io.winspace.Development` only
  from whoever controls `winspace.io`, which does not resolve, by serving a
  token at `https://winspace.io/.well-known/org.flathub.VerifiedApps.txt`.
  The alternative is an ID under a domain the project controls, such as
  `app.openxplorer.OpenXplorer` for `openxplorer.app`; the ID is a
  compatibility contract (AGENTS.md: the desktop entry, AppStream ID, D-Bus
  name and Flatpak ID), so a new ID needs `build.rs`, the packaging data and
  the migration of existing Flatpak users (`~/.var/app/<id>/`) changed
  together, and an `<id>` rename entry (`<provides>`/`<replaces>`) in the
  metainfo.
- Open the submission pull request on `flathub/flathub`, accept Flathub's
  terms, and maintain the `flathub/<app-id>` repository it creates.
- Publish the screenshots on `openxplorer.app` (the website deploy).

#### Why each permission is needed (for the Flathub review)

- `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc`,
  `--device=dri`: the standard set for a GTK 4 window drawn with the GPU.
- `--filesystem=host`: OpenXplorer is a file manager. Users browse, copy,
  rename and delete files anywhere they have access, including other disks
  under `/media`, `/run/media` and `/mnt`, `/opt` and `/srv`; the file
  chooser portal cannot give a file manager its folders one at a time.
  `host-os` and `host-etc` are not requested.
- `--talk-name=org.gtk.vfs.*`, `--filesystem=xdg-run/gvfsd`,
  `--filesystem=xdg-run/gvfs`: SMB shares, phones (MTP), cameras (PTP), the
  Trash and the drive list come from the host's GVfs: GIO inside the
  sandbox needs the daemons' D-Bus names, their private sockets and their
  FUSE paths.
- `--talk-name=org.freedesktop.secrets`: SMB passwords the user asks to
  remember are stored in the Secret Service, under the schema the 1.x app
  used, so both find the same saved sign-ins.
- `--own-name=org.freedesktop.FileManager1`: the opt-in "Show in folder"
  integration. After the user turns it on in Settings, browsers' "Show in
  folder" requests reach OpenXplorer, as they reach Nautilus or Dolphin. The
  app claims the name only while the integration is on.
- `--talk-name=org.freedesktop.Flatpak`: `flatpak-spawn --host` for two
  explicit user actions: Open in Terminal starts the host's terminal in the
  current folder, and "Make OpenXplorer the default file manager" (opt-in)
  runs the host's `xdg-mime`, since the default applications are the host's.
- `--share=network`: "Check for updates" reads GitHub's public release list to
  say whether a newer version exists; Flatpak installs it. SMB and phones
  go through the host's GVfs, not this permission. If Flathub asks to drop
  it, the Flatpak build must hide the update check first.

## RPM

```sh
python3 native/tools/source_archive.py --vendor --output-directory ~/rpmbuild/SOURCES
rpmbuild -bb native/packaging/rpm/openxplorer.spec
rpmbuild -bb --define 'app_id io.winspace.Development' native/packaging/rpm/openxplorer.spec
```

The spec builds offline from `openxplorer-<version>.tar.gz` (the committed
sources) and `openxplorer-<version>-vendor.tar.gz` (every crate, from
`cargo vendor`), with the distribution's own Rust. It works on Fedora and
openSUSE; the GVfs backend names differ and are chosen with `%{suse_version}`.
Fedora tags the release with its version (`.fc44`); openSUSE sets no tag, so
`ci/build-package.sh` passes the distribution's ID (`.opensuse_tumbleweed`)
and the two RPMs of a release have different names.

## Arch package

```sh
python3 native/tools/source_archive.py --output-directory build
cp native/packaging/arch/PKGBUILD build/ && cd build && makepkg
```

Set `_app_id=io.winspace.Development` for the stable app. The PKGBUILD follows
the Arch Rust package guidelines: `cargo fetch` in `prepare()`, then a frozen
offline build.

## Continuous integration

[`.github/workflows/native-distros.yml`](../../.github/workflows/native-distros.yml)
runs, in Fedora, openSUSE Tumbleweed, Arch Linux, Ubuntu 24.04 and Debian 13
containers, the check driver (`tools/check.py`) against each distribution's own
GTK, GLib and GVfs, then builds and verifies the preview package of that
distribution's format and publishes it as an artifact; a separate job builds
the Flatpak bundle. The scripts in [`ci/`](ci/) are its steps:
`prepare-container.sh` installs the distribution's packages and adds the
unprivileged user the checks run as, `install-rust.sh` installs Rust 1.92.0
(the minimum supported version) and stable, `run-checks.sh` runs the driver
and `build-package.sh` builds and verifies the package.

The `native-package` job of
[`.github/workflows/checks.yml`](../../.github/workflows/checks.yml) runs the
same steps for the release on every push to `main`, building the stable app:
the `.deb` on Ubuntu 24.04, an RPM on Fedora and on openSUSE Tumbleweed, and
the Arch package; `native-flatpak` builds the stable Flatpak bundle. The
release needs the `.deb` (the 1.1.x updater installs it) and the Flatpak. An
RPM or Arch build that fails is marked as allowed to fail, so the release
goes ahead without that package, and `tools/release.py --packages` stages and
checksums the packages that were built.
