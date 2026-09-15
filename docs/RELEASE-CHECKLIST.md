# 1.0 release checklist

**Current status: 1.0.0 is published; this source tree prepares a local 1.0.2 maintenance build.** A built
Debian package, source inspection and simulated browser checks are useful
evidence, but do not prove native compatibility or the absence of
vulnerabilities. Treat the gates below as the standing per-release list: the
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

## Native desktop gate (owner task on real hardware)

On Zorin under the intended Wayland session, then X11 where supported:

- Install the candidate, run `openxplorer --check`, confirm launch, no duplicate
  title/menu row and functional WebKit subprocess sandbox. Do not disable the
  sandbox to hide an installation failure.
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
SECURITY.md links to it. The Debian `Maintainer` field still carries a
placeholder address; replace it with a monitored contact. Add the actual
repository/source URL. Choose supported/tested distro versions and publish
checksums via a trusted channel; checksums alone are not a signature. Create a
signing/release process without embedding keys in the repo. Re-run current
upstream advisories/system package updates before signing.

Semantic and Debian versions are `1.0.2`. For each later release, bump both,
rebuild, verify source correspondence, and publish the installer, the
corresponding-source archive and `SHA256SUMS` together. Do not present this
checklist or a limited source sweep as independent security certification.
