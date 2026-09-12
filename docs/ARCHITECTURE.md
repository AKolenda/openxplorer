# One repository, two independently deployable products

## Decision

Keep OpenXplorer as a monorepo. The website embeds a build of the actual app UI;
documentation, screenshots, version metadata and AGPL corresponding-source
archives change together. Separate repositories would add cross-repository
asset publishing and synchronization without an operational benefit here.

## Boundaries

| Directory | Responsibility | Runtime dependencies |
|---|---|---|
| `desktop/` | Python native host, GIO operations, desktop UI | Distro Python GI, GTK/WebKit/GVfs |
| `apps/web/` | Next.js website and docs presentation | pnpm at build time; static export for hosting |
| `docs/` | Generated guides and handwritten policies/release review | None |
| `tools/` | Preview generation, screenshots, source/deb release assembly | Python, Node; Playwright for captures |
| `dist/` | Same-version Debian package and corresponding source | Download artifacts, not editable source |

pnpm manages the website workspace, not Python system libraries. No Turborepo,
Nx, second JavaScript desktop runtime or shared backend is needed. The installed
file manager does not depend on website availability. The website never receives
the privileged native bridge or user data. Its interactive preview is isolated
and backed by fictional fixtures; it is not instantiated on narrow screens.

## Change and release flow

Edit `desktop/ui/*` for product behavior. Edit `apps/web/lib/docs.json` for guide
content. `tools/prepare-web.cjs` builds the app preview and generated Markdown;
`tools/render-previews.cjs` creates standalone design-review pages. The latter
is not a Next.js production build. `pnpm build` produces the hosted export after
a real, locked install.

`python3 tools/release.py` builds/verifies the Debian package and corresponding
source, then stages matching downloads for the website. It never installs the
app, changes desktop defaults, modifies Brave or mounts a network share.

Keep common versions while the project is small. Debian uses `1.0.0~rc4` for
this prerelease so that final `1.0.0` upgrades it normally. UI, source and website
use semantic version `1.0.0-rc.4`. Stable internal Winspace IDs/paths remain for
compatibility. A website-only deployment can reuse the current desktop package.

## Ownership and contribution

Use one issue tracker, one review flow and one security reporting channel, with
area labels such as desktop, website, docs and integration. The repository owner
has not supplied a public repository/contact yet; no links or emails are invented.
AGPL-3.0-only applies to project-authored changes; preserve file-level exceptions
and third-party notices. Review ownership/contact and source availability before
announcing a public release.
