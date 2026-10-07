# Development

One repository. A native app and an independent website.

## Repository layout

native/ contains the Rust GTK 4 application, its parity inventories, packaging and check driver. The Python GTK/WebKit app of 1.x has been removed from the tree: its last release is tag v1.1.4, and its final sources, which native/parity/ cites as the behavioural specification, are desktop/ at tag v2.0.0. apps/web/ contains the Next.js App Router website. docs/ mirrors the website documentation as Markdown. designs/ holds generated standalone HTML pitches.

## Run the website with pnpm

Use Node.js 22.13+ and the package manager version declared at the root. The supplied pnpm-lock.yaml records resolved dependencies. Use frozen installs and review any dependency changes together with their lockfile changes. Dependencies are not vendored.

```sh
corepack enable
corepack prepare pnpm@10.34.5 --activate
pnpm install --frozen-lockfile
pnpm audit --audit-level=moderate
pnpm dev
```

> Dependency installation, dependency-aware pnpm check and the real Next.js production export have passed. The production-export browser suite also passes, including React hydration, tour readiness and mobile iframe removal. Standalone HTML rendering is a separate design-review workflow; consult the current test report for each check actually executed.

## Build and validate

The website remains a Next.js static export. pnpm build writes apps/web/out, and pnpm check performs dependency-aware TypeScript checking. Cloudflare Workers Static Assets serves that export without a framework migration or application backend. The offline design renderer does not exercise the Next.js runtime.

The native experience roadmap and public source checklist are in docs/NATIVE-EXPERIENCE-ROADMAP.md and docs/PUBLIC-RELEASE-CHECKLIST.md. They distinguish remaining desktop work, source-publication preparation and stable-release validation.

```sh
pnpm check
pnpm build
pnpm preview

# Native checks and the release .deb (Ubuntu 24.04 build host)
python3 native/tools/check.py
python3 native/parity/check.py
python3 native/tools/build_deb.py --app-id io.winspace.Development
python3 native/tools/verify_deb.py dist/native/openxplorer_2.0.3_all.deb
```

## Make a focused contribution

Read CONTRIBUTING.md and SECURITY.md. Use disposable files for operation tests. Separate UI fixtures from native integration results, include reproducible steps, and do not publish NAS passwords or private filename inventories. Contributions are under the project license, with existing notices preserved.

## Compatibility identifiers

The visible name and public command are OpenXplorer/openxplorer. The previous io.winspace.Development desktop ID, winspace configuration paths and credential schemas remain deliberately stable. Do not bulk-rename them without a migration design.

## Regenerate the screenshots and the tour

The website screenshots and the click-through tour are pictures of the native app, taken by its snapshot hook (OPENXPLORER_SNAPSHOT) in an isolated session: bubblewrap hides the home folders, mounts and session bus and gives the app no network, Xvfb and a private D-Bus session keep it off the desktop, and a fictional demo tree is mounted as /home/demo. The tour's clickable areas are the rectangles of real controls, which the app reports. Do not replace the pictures with a separately drawn explorer mockup. Review every picture before committing it.

```sh
python3 tools/capture-screenshots.py
python3 tools/capture-native-tour.py
python3 tools/audit-public-data.py
pnpm designs
```

> Capturing needs cargo, bubblewrap (bwrap), xvfb-run and dbus-run-session; the tools build the release program with the stable application ID. The pictures show fictional files and no live SMB server; they do not replace the native tests.

## Documentation as Markdown

Copy page as Markdown copies the full guide, including headings, paragraphs, code blocks, callouts and screenshot references—not a link or rendered HTML. The content source is apps/web/lib/docs.json; tools/sync-docs.py regenerates repository documentation and the website copies.

Documentation search opens from its named button. The site does not intercept Command-K or Control-K. Keyboard navigation within the search dialog and Escape-to-close remain available.

---

OpenXplorer 2.0.3. Project-authored documentation: AGPL-3.0-only.
