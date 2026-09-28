# Third-party notices and licensing scope

## Original Winspace material

The desktop is based on the Winspace 0.7.0 source supplied in this conversation. Its original MIT copyright, permission and disclaimer notice is preserved without editing in `licenses/Winspace-MIT.txt` and `desktop/licenses/Winspace-MIT.txt`. The modified combined application is offered under AGPL-3.0-only; the original permissive grant is not erased.

## Distribution runtime libraries

GTK, GLib/GIO, WebKitGTK, GVfs, libsecret and other APT-provided libraries are not vendored or relicensed by this repository. Obtain their source/notices from the relevant distribution packages. The package declares the libraries it needs.

## Website dependencies

Next.js and React/React DOM are MIT-licensed upstream projects; TypeScript is Apache-2.0; DefinitelyTyped type packages use their own included notices. pnpm is build tooling, not a project-authored dependency. They are installed separately through the package manager, not copied into this delivery. Preserve their package notices in deployments/distributions as required. Lock and review the actual dependency graph on the connected build machine; this document is not a generated complete transitive software bill of materials.

## Icons of the native app

The native preview (`native/crates/ox-app`) bundles unmodified SVG icons from two Microsoft sets, both under the MIT licence. Their copyright and permission notices are preserved without editing in `licenses/`:

- Fluent UI System Icons, npm package `@fluentui/svg-icons` 1.1.343 (https://github.com/microsoft/fluentui-system-icons): `licenses/Fluent-UI-System-Icons-MIT.txt`.
- Fluent Emoji, commit 1ffb34c752ecf5d402f04cfb4b392c77f57c54bc (https://github.com/microsoft/fluentui-emoji): `licenses/Fluent-Emoji-MIT.txt`.

`native/crates/ox-app/resources/icons/SOURCES.md` lists every bundled file with its upstream name and SHA-256. The files are renamed with an `ox-` prefix but their contents are byte-identical to upstream. No Office, Windows or other product logos are included.

## Art and documentation

The desktop app's folder icon comes from the original project (the native app's is Fluent Emoji, see above). Website inline line icons and CSS geometry were created for this project. There are no bundled Windows/Zorin/Vercel logos, proprietary typefaces, stock photographs or copied third-party screenshots. Website UI illustrations use explicitly fictional sample files. AppStream metadata continues to use CC0-1.0 as declared in the file. GNU license text is reproduced for its stated purpose and must not be changed.

## Reference sources

These informed the scaffolding and licensing; they are not application dependencies:

- Next.js installation: https://nextjs.org/docs/app/getting-started/installation
- Next.js static exports: https://nextjs.org/docs/app/guides/static-exports
- Next.js/React package metadata: https://registry.npmjs.org/next/latest and https://registry.npmjs.org/react/latest (versions selected on 2026-09-06).
- pnpm setup: https://pnpm.io/installation
- GNU AGPLv3 text: https://www.gnu.org/licenses/agpl-3.0.html
- SPDX AGPL-3.0-only text: https://raw.githubusercontent.com/spdx/license-list-data/main/text/AGPL-3.0-only.txt
