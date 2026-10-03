# Release checklist

**Current status: this source tree prepares 2.0.1 of the native GTK 4 app
(`native/`) that replaces the retired Python app (last released as tag
v1.1.4, no longer in the tree or shipped).** The remaining parity items, known gaps and owed hardware
acceptance are in [native/BACKLOG.md](../native/BACKLOG.md). Built packages,
source inspection and isolated tests are useful evidence, but do not prove
native compatibility or the absence of vulnerabilities. Treat the gates below as the standing per-release list: the
website and packaging gates run in CI and locally, while the native desktop
gate stays an owner task on real Zorin hardware.

## Website dependency gate

On a connected development machine with the pinned pnpm:

```sh
corepack enable
corepack prepare pnpm@10.34.5 --activate
pnpm install --lockfile-only --ignore-scripts
# Review all resolved sources/integrities and commit pnpm-lock.yaml.
pnpm install --frozen-lockfile
pnpm audit --audit-level=moderate
pnpm check
pnpm build
pnpm preview
```

Review audit findings rather than automatically forcing upgrades. Do not change
production to an unfrozen install to make CI green. Verify Next hydration,
exported routes, demo sandbox, mobile docs, real download/source links and host
security headers. CI and Vercel deliberately require a lockfile. Pin CI actions
to reviewed immutable commits before enabling a privileged release workflow.

## Native app gate (runs locally and in CI)

```sh
cd native && cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings   # workspace lints in native/Cargo.toml
cd .. && python3 native/tools/check.py        # private Xvfb display, D-Bus and HOME
python3 native/parity/check.py                # inventories; --gate replace lists the backlog
python3 tools/release.py [--flatpak]           # stable .deb (+ Flatpak), source ZIP, SHA256SUMS in dist/
python3 native/tools/verify_upgrade.py --python openxplorer_1.1.4_all.deb \
    --stable dist/openxplorer_<version>_all.deb
python3 tools/audit-public-data.py
```

The versions in `native/Cargo.toml`, `native/packaging/rpm/openxplorer.spec`,
`native/packaging/arch/PKGBUILD`, the newest `<release>` of both metainfo files
in `native/packaging/data/`, `package.json`, `apps/web/package.json`,
`apps/web/lib/site.ts` and `tools/sync-docs.py` must agree, and `CHANGELOG.md`
starts with the release's entry (the workflow publishes its first section as
the release notes). On `main`, `checks.yml` builds the stable `.deb` (Ubuntu
24.04), the RPMs (Fedora, openSUSE), the Arch package and the Flatpak bundle on
hosted runners, and the self-hosted release job verifies and publishes them.

## Native desktop gate (owner task on real hardware)

On Zorin under the intended Wayland session, then X11 where supported:

- Install the candidate over 1.1.4 through **Check for updates** and from the
  file, run `openxplorer --version`, confirm launch and no duplicate
  title/menu row. Install the Flatpak bundle on a second distribution.
- Middle-click local/SMB folders and sidebar locations. Check background vs
  Shift foreground, tab closing, tear-out onto the desktop and source body,
  merge-back, reorder, Escape, busy/closed destination and scaled displays.
- Test Open in Terminal on a local folder with spaces/shell metacharacters,
  ordinary file, CIFS mount and GVfs-FUSE share. Check actual cwd, no unintended
  commands, and actionable errors for an unmounted share/missing terminal.
- Use disposable data to test copy/cut/paste between windows, collisions,
  cancellation, permission failures, disk full, disconnect/reconnect and stale
  search rows. Verify original/source data survives failed operations.
- Check session and persistent keyring credentials across shares/windows,
  rejected credentials, cancelled prompts and sign-out while prompts are active.
- Test ZIP extraction, invalid/encrypted/oversized archives, snapshot read-only
  guards and restore-a-copy. External viewers/terminals are not sandboxed by us.
- Verify indexing/privacy permissions, default-folder handlers, FileManager1
  and Brave/portal behavior. Browser-profile edits require explicit opt-in.
- Review the optional root mount helper in a VM before testing actual systemd
  mount setup/removal. It keeps credentials in a root-readable plaintext file.

## Publication gate (owner action required)

Private vulnerability reporting is enabled on the GitHub repository and
SECURITY.md links to it. The packages name "OpenXplorer contributors
<openxplorer@users.noreply.github.com>", the project's commit identity, as
maintainer; security reports go through GitHub's private reporting. Add the actual
repository/source URL. Choose supported/tested distro versions and publish
checksums via a trusted channel; checksums alone are not a signature. Create a
signing/release process without embedding keys in the repo. Re-run current
upstream advisories/system package updates before signing.

The release version is `2.0.1`. For each later release, bump every version
listed above, rebuild, verify source correspondence, and publish the packages,
the corresponding-source archive and `SHA256SUMS` together. Do not present this
checklist or a limited source sweep as independent security certification.
