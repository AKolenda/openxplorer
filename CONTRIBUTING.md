# Contributing to OpenXplorer

## Before a change

Read [README.md](README.md), [AGENTS.md](AGENTS.md), [the development guide](docs/development.md) and [SECURITY.md](SECURITY.md). Keep a change focused and explain the observable problem before changing implementation details. A visual improvement must preserve file-operation and desktop integration safeguards.

## Local development

The desktop uses distribution-managed Python GI, GTK 3, WebKitGTK 4.1 and GIO/GVfs. Follow [desktop/README.md](desktop/README.md) for native prerequisites. Do not pip-install an unrelated package named `gi` or run the file manager as root. Node and pnpm are website/build tooling; the installed desktop app does not depend on them.

Use Node.js 22.13+ and the repository's pinned pnpm version:

```sh
corepack enable
corepack prepare pnpm@10.34.5 --activate
pnpm install --frozen-lockfile
pnpm dev
```

Commit dependency changes together with the updated `pnpm-lock.yaml`. A normal checkout uses the committed lock; do not regenerate it merely to bypass an installation failure. Keep local environment values and Cloudflare credentials outside source control. `.env.example` and `.dev.vars.example`, when present, may contain only public placeholder values.

## Checks before a pull request

Run the checks that cover the change and include their actual outcomes:

```sh
pnpm test:desktop
node --test desktop/tests/*.test.cjs
pnpm security:source
pnpm check
pnpm build
```

For desktop UI changes, generate the shared preview and run the appropriate `desktop/tests/ui_*.py` suites. Those suites require Python Playwright and Chromium; set `CHROMIUM` to the browser executable. They exercise simulated filesystem/bridge data and do not establish native WebKit or SMB behavior.

```sh
python3 desktop/tools/build_preview.py
python3 desktop/tests/ui_file_drag.py
```

`desktop/tests/native_file_transport.py` exercises the production GTK/WebKit transports using synthetic HTML and disposable files. Run it only on an isolated X11 display because it moves the pointer and presses keys. With Playwright and `CHROMIUM` configured, it also verifies real files arriving in a separate Chromium process. The [CI workflow](.github/workflows/checks.yml) declares the native prerequisites and command. This does not test the entire installed app, T3 Code, Wayland, live SMB, or the sandbox file-transfer portal.

The package builder needs `dpkg-deb` and CairoSVG or a GdkPixbuf SVG loader. Build and inspect the installer without installing it:

```sh
python3 desktop/tools/build_deb.py --output /tmp/openxplorer-candidate.deb
python3 desktop/tools/verify_deb.py /tmp/openxplorer-candidate.deb
```

GitHub checks use a read-only token and do not publish packages, create releases, deploy the website or modify desktop defaults. A workflow file existing in the repository is not evidence that its hosted run passed.

## File-operation changes

Use disposable directory trees and non-critical shares. Test collisions, cancellation, partial copies, offline servers, read-only snapshots and stale cached metadata. Never substitute permanent deletion when Trash is unsupported. Do not overwrite a live file during Previous versions restore. Keep passwords out of logs, fixtures, screenshots and source archives.

## UI and documentation

Use existing tokens and semantic controls. Preserve keyboard access, responsive layout and reduced-motion behavior. Change shared TSX/CSS sources, then regenerate designs; do not hand-edit generated pitches. `apps/web/lib/docs.json` is the canonical guide source. Regenerate its Markdown with `python3 tools/sync-docs.py`; use `pnpm designs` to rebuild standalone designs. An offline TSX render is not a Next.js production build.

Use only fictional names, paths and shares in examples and screenshots, following [docs/PRIVACY.md](docs/PRIVACY.md). Use the capture scripts for public images, then run `python3 tools/audit-public-data.py`. Keep private denylist files outside the repository. The default audit verifies packaging and provenance; private identifiers require the optional external denylist and visual review.

Keep documentation and release claims synchronized with behavior and tests. Generated installers, archives, previews, local logs and `test-results/` stay out of Git. Preserve the corresponding-source build tools and visible website source link; see the [public-release checklist](docs/PUBLIC-RELEASE-CHECKLIST.md).

## Compatibility and license

The `winspace` configuration paths, `io.winspace.Development` application/desktop identity and credential schemas are stable compatibility contracts. Propose an explicit migration before renaming them. Preserve existing notices.

Contributions are submitted under AGPL-3.0-only unless a documented file-level exception applies. Preserve the root [LICENSE](LICENSE), [NOTICE](NOTICE), upstream [MIT notice](licenses/Winspace-MIT.txt), desktop copies and [third-party notices](THIRD_PARTY_NOTICES.md). Include provenance for copied code/assets and ensure you are authorized to contribute them. No CLA is required by this repository.

## Suggested pull-request summary

Describe the problem, the resulting behavior, tests actually run, known limitations and any migration impact. Use the pull-request template. For screenshots, use synthetic filenames and shares. If you did not run native tests, state that plainly. Report security-sensitive findings through the process in [SECURITY.md](SECURITY.md), without exposing credentials or private filesystem data in a public issue.
