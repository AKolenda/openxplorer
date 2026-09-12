# OpenXplorer

Public source repository: **https://github.com/AKolenda/openxplorer-public**. The companion website is **https://openxplorer.app**; Cloudflare setup is in progress and a live deployment is not yet claimed.

[Current update and troubleshooting](docs/UPDATE-RC4.md)

**Windows File Explorer-inspired file manager for Linux.** Built for Zorin OS, with compatible Ubuntu and Debian systems in mind.

Browse local folders and SMB shares in one workspace. Keep the familiar tabs, clickable paths and pinned sidebar; keep control of the source.

![OpenXplorer’s actual HTML interface, browsing a sample NAS in light mode](apps/web/public/assets/screenshots/explorer-light.png)

*Actual application HTML and icons, captured in Chromium using fictional sample files. This is not a native desktop or live-NAS test.*

**[Installation](docs/installation.md)** · **[Network shares](docs/network-shares.md)** · **[Website setup](apps/web/README.md)** · **[Contributing](CONTRIBUTING.md)** · **[License](LICENSE)**

## This candidate

Middle-click folders and locations to open background tabs; use Shift+middle-click
to switch immediately. Tear tabs out into new windows again, while retaining
cross-window merging and acknowledgement-based handoffs. The native drag source
now supports GTK's desktop-drop handshake and an explicit tear-out area below
the source tab strip. See [rc4 update notes](docs/UPDATE-RC4.md).

The installed Software card retains the icon and AGPL license. A generic
pre-install local-package card is not fixed merely by embedding another icon;
a first-install catalog experience needs pre-indexed AppStream distribution.

## 1.0 release candidate 4

**This is a release candidate, not a signed-off production 1.0.** The desktop
installer is built separately from the website. Website dependencies and the
pnpm lockfile are now present; dependency-aware `pnpm check` and the real Next.js
`pnpm build` have passed. Production-export browser and hydration checks have also passed.
Target Zorin/Wayland, live SMB and installed-application interoperability checks
remain part of the stable-release validation.
See the [security review](docs/SECURITY-REVIEW.md), [release checklist](docs/RELEASE-CHECKLIST.md)
and [executed test report](TEST-REPORT.md).

The [native experience roadmap](docs/NATIVE-EXPERIENCE-ROADMAP.md) separates
remaining interaction improvements from larger features. The
[public source checklist](docs/PUBLIC-RELEASE-CHECKLIST.md) covers repository
publication and keeping the website pointed at the public source.

Right-click a folder, sidebar pin, mounted share, or the file-list background →
**Open in Terminal**. For a regular file, **Open containing folder in Terminal**
opens its parent. The app resolves current metadata before opening anything.
SMB needs an existing local CIFS/GVfs-FUSE path: this starts your **local shell**,
not SSH on the NAS. Server listings, ZIP interiors and protected snapshot views
are not terminal directories. No filenames become shell commands.

The terminal selection honors Debian's system alternative when its target is
recognized, then falls back to installed GNOME Terminal, Console, Xfce Terminal,
Konsole or XTerm. It does not evaluate `$TERMINAL` or search a folder's PATH.
On Zorin, `sudo apt install gnome-terminal` provides the normal fallback.

Earlier ZIP extraction, larger text, mobile documentation, fictional previews,
resizable layout and GTK menu-row fixes are retained. Finish file operations
and run `openxplorer --quit` before upgrading; a background service may survive
closing the last window.

## One repository, two deliverables

The Python application stays in `desktop/`; the pnpm/Next.js site stays in
`apps/web/`. They share documentation, release versions and the actual app preview.
There is no runtime dependency from the installed app to Node, pnpm or the website.
Changes to the site can be deployed without reinstalling the desktop package.
See [monorepo architecture](docs/ARCHITECTURE.md).

## Try the interface

The website embeds `desktop/preview.html`, built from the **same HTML, CSS, JavaScript and icons as the desktop application**. It uses a simulated filesystem adapter, not a separate mockup.

The Zorin-inspired homepage includes an interactive preview and a user-triggered walkthrough: type `\\studio-nas\Projects`, open the sample share, drag **Design** to the pinned sidebar, then open the new pin. There is no real network access. Reduced-motion preferences are respected; Stop, Reset and direct interaction interrupt the tour.

Generate the single-file preview:

```sh
python3 desktop/tools/build_preview.py
```

Open `desktop/preview.html` in a browser. The `designs/` directory in a complete release contains a self-contained website preview, with the app embedded in a script-only sandbox and screenshots inlined. Serve the folder to follow its documentation and repository links reliably:

```sh
python3 -m http.server 3000 --directory designs
```

Open `http://localhost:3000`. The native app connects to your storage; the browser preview does not access your files, credentials, keyring, defaults, or NAS.

## ZIP files and larger text

Right-click a ZIP and choose **Extract all…**, then choose an existing destination and a **new** output-folder name. The ZIP is unchanged and existing files are not overwritten. A yellow folder-and-zipper icon distinguishes archives from ordinary folders. Double-click still opens the read-only ZIP browser.

Use **Ctrl +** (also Ctrl =), **Ctrl −**, and **Ctrl 0** to enlarge, reduce, or reset text. **Settings → Appearance & layout → Text size** provides the same saved 80%–200% setting.

## Install the desktop release candidate

Finish file operations and close all existing OpenXplorer / Winspace windows and services. From the download directory:

```sh
openxplorer --quit
sudo apt install ./openxplorer_1.0.0~rc4_all.deb
openxplorer --check
openxplorer --restart
```

The package depends on distribution-provided Python GI, GTK 3, WebKitGTK 4.1, GIO/GVfs and desktop integration tools. Zorin is the primary target; other Debian-family distributions need compatible packages. No blanket compatibility or production-readiness claim is made.

The installer does **not** change your default file manager, Brave preferences, download folder, credentials or mounts. Those features require explicit actions in Settings. Legacy Winspace configuration and desktop IDs remain compatible; `winspace` is an alias.

For the initial blank-window rendering issue, close the application and try `openxplorer --software-rendering`. Read the [Zorin setup guide](desktop/ZORIN-SETUP.md) for defaults, portals, Brave and taskbar integration.

## What is in the app

| Workflow | Implementation and boundaries |
|---|---|
| Local files and SMB | GIO/GVfs storage; Windows-style UNC and `smb://` addresses; custom authentication UI; keyring-backed credential preferences. Native behavior needs target-machine testing. |
| Navigation | Tabs, clickable ancestor paths, type-to-select, side-button navigation, draggable pins, resizable sidebar and columns, light/dark appearances. |
| Cached search | Opt-in filename/path indexing, local change monitoring, incremental network checks. This is metadata, not offline file content or instant SMB push notifications. |
| Opening and file operations | Metadata-based file activation, app chooser, desktop file clipboard, progress/cancellation, conservative duplicate handling, read-only ZIP browsing and extraction into a new folder. Cross-device cut/move is limited. |
| Desktop integration | Optional folder handlers, FileManager1 service, known-folder locations and reviewed Brave download updates. Desktop portals still control some routes. |
| Previous versions | Browse existing exposed snapshot/backup directories and restore a copy. No SMB shadow-copy protocol enumeration, snapshot creation or in-place restore. |

### Previous versions you can recognize

![Previous versions with dates in the right-hand column](apps/web/public/assets/screenshots/previous-versions.png)

Dates come from recognizable snapshot names, such as `auto-2026-09-04_16-30`. The interface labels their source. `@GMT` names show UTC; other name-encoded times are not timezone-converted. Unrecognized names show **Date unavailable**. Folder modification times are not treated as snapshot creation times.

![A historical folder in a clearly marked Previous version tab](apps/web/public/assets/screenshots/snapshot-tab.png)

Historical tabs have an amber edge, a **Previous version** badge and a banner. SMB tabs also keep their green network marker. Browse opens a new tab; the original tab retains its Previous versions dialog. Historical views are read-only inside OpenXplorer, not a system-wide permission guarantee.

### Network folders where you need them

![Actual pinned network folder with green indicator](apps/web/public/assets/screenshots/pinned-sidebar.png)

Use backslashes in a Windows-style path, **not backticks**:

```text
\\studio-nas\Projects
smb://studio-nas/Projects
```

Pinning a share or subfolder creates a bookmark. It does not copy or move files. Network discovery depends on the installed providers and what your devices advertise.

## Next.js website with pnpm

```sh
corepack enable
corepack prepare pnpm@10.34.5 --activate
pnpm install --frozen-lockfile
pnpm audit --audit-level=moderate
pnpm dev
```

Open `http://localhost:3000`. Node.js 22.13+ is the configured baseline. The app-router site lives in `apps/web/` and defaults to the selected **Zorin / Horizon** design. Windows and Blueprint alternatives remain at `/concepts/windows/` and `/concepts/vercel/`.

```sh
pnpm check
pnpm build
pnpm preview
```

Next.js exports to `apps/web/out`. Use the supplied `pnpm-lock.yaml` with frozen
installs; review dependency and lockfile changes together. The real typecheck
and production export have passed. Standalone HTML rendering remains a separate
preview workflow, and production hydration has its own browser checks.

## Cloudflare website hosting

The site remains a Next.js static export. `wrangler.jsonc` serves `apps/web/out`
through Cloudflare Workers Static Assets, with `openxplorer.app` as the canonical
domain. No framework migration or application backend is needed. Follow
[Cloudflare setup](docs/CLOUDFLARE-SETUP.md) for account and domain verification.

Use the project-local Wrangler and the dedicated `openxplorer` OAuth profile:

```sh
pnpm cf:login
pnpm cf:activate
pnpm cf:profiles
pnpm cf:whoami
```

Named profiles are experimental in the pinned Wrangler version. They keep this
project's OAuth identity separate from the default profile; an environment
`CLOUDFLARE_API_TOKEN` takes precedence, so verify the active identity before
publishing. The project profile is authenticated separately from the default login.

Build local release artifacts, audit the public data and validate the export before deploying:

```sh
pnpm check
pnpm audit --audit-level=moderate
pnpm designs
pnpm release
python3 tools/audit-public-data.py
pnpm build
pnpm cf:dry-run
pnpm cf:dev
# After reviewing the local export and confirming the intended account/domain:
pnpm cf:deploy
```

The website does not host the installer, source ZIP or checksums. Its project
calls to action lead to the public GitHub repository. The dry run validates
local packaging; it does not publish the site or prove that the account/domain
is configured. Rebuild and rerun the release audit after any further source or
public-asset changes.

## Documentation and screenshots

The documentation uses a topic sidebar, readable article column and “On this page” navigation. Each page offers **Copy page as Markdown** and a `.md` download. Search opens from its named button; the site does not intercept Command-K or Control-K.

`apps/web/lib/docs.json` is the content source. Markdown, the search index and clipboard content are generated from it:

```sh
python3 tools/sync-docs.py
node tools/prepare-web.cjs
pnpm designs
```

Screenshots are generated from the real preview, not a marketing recreation:

```sh
python3 desktop/tools/build_preview.py
python3 tools/capture-screenshots.py
```

The capture script requires Playwright and Chromium. Set `CHROMIUM` to use another installed Chromium executable. No external font files are bundled. See the [screenshot manifest](apps/web/public/assets/screenshots/manifest.json).

## Repository layout

```text
openxplorer/
  desktop/                 Python GTK/WebKit host, GIO storage and actual UI
    ui/                    Desktop HTML, CSS, JS and icons
    demo/showcase.js       Preview-only NAS and pinning walkthrough
    tests/                 Storage, browser, snapshot and selector checks
    tools/                 Preview / Debian package builders and verifier
  apps/web/                Next.js application and website README
    components/            Shared site and real-preview components
    lib/docs.json          Canonical guide content
    public/                Preview, screenshots and generated Markdown
  docs/                    Generated, repository-friendly Markdown guides
  tools/                   Website preparation, screenshots and release tooling
  designs/                 Generated standalone HTML in complete releases
  dist/                    Local installer, corresponding source and checksums
  LICENSE                  AGPL-3.0-only
```

## Build and verify

Building the desktop package does not require root or network access:

```sh
python3 desktop/tools/build_deb.py --output dist/openxplorer_1.0.0~rc4_all.deb
python3 desktop/tools/verify_deb.py dist/openxplorer_1.0.0~rc4_all.deb
```

Run automated checks:

```sh
(cd desktop && python3 -m unittest discover -s tests -p 'test_*.py')
node desktop/tests/type_select.test.cjs
node desktop/tests/snapshot_meta.test.cjs
python3 desktop/tests/ui_v09.py
python3 tests/test_website.py
```

Browser checks require Python Playwright and Chromium. Tests use disposable local files or simulated storage. Consult **[TEST-REPORT.md](TEST-REPORT.md)** for current counts and explicit native-runtime / Next.js validation boundaries. No script should be used against important files as its first native test.

## Contribute and license

Read [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md). The canonical
domain is `openxplorer.app`; the public repository link is intentionally unset
while the source repository is private. The corresponding-source ZIP remains
available to public visitors. Set the public repository URL in
`apps/web/lib/site.ts` if the repository is made public, and follow the
[public source checklist](docs/PUBLIC-RELEASE-CHECKLIST.md). Do not invent contact
details, statistics or hosting claims.

OpenXplorer project changes, its website and project-authored documentation are licensed **AGPL-3.0-only**. The complete license and corresponding-source build tools are included. Original Winspace MIT notices and third-party notices are preserved in `NOTICE`, `THIRD_PARTY_NOTICES.md` and `desktop/licenses/`.

Independent project. Not affiliated with Microsoft, Zorin, Canonical, Debian or Vercel. No warranty.
