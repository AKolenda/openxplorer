# Security review: OpenXplorer 1.0.0-rc.4

Review date: 2026-09-07. Scope: editable desktop runtime and native bridge,
file/archive operations, credentials, indexing, root mount helper, desktop and
Brave integration, website/demo boundary, direct dependency declarations,
packaging and CI. This is a first-party source review with focused regression
checks, **not an independent penetration test or a vulnerability-free guarantee**.

## Decision

Keep the combined monorepo. Deliver a 1.0 release candidate, not stable 1.0.
No known unfixed application-level exploit was demonstrated in the reviewed
paths. That is not a claim that none exist: transitive dependencies, the full
native runtime and real deployment remain unverified. See RELEASE-CHECKLIST.md.

## Findings and changes

| Area | Finding / classification | Change and evidence |
|---|---|---|
| Website build tooling | **Confirmed affected version:** pnpm 10.11.0 falls in upstream advisory ranges for install-time path traversal. This affects developers/build systems, not the desktop package's Python runtime. | Pin 10.34.5, a patched 10.x version for the cited advisories. No claim that a full resolved dependency tree is clean. |
| Settings/index storage | Existing state/lock/database opens could follow symlinks and chmod a redirected target. Defense in depth against tampered or misconfigured local state; not a demonstrated remote privilege escalation. | Owned private directories, no-follow/nonblocking regular-file opens, hardlink rejection, bounded settings reads and SQLite sidecar checks. Tests prove target contents/permissions survive refused links. |
| ZIP preview | Preview listing was less strict than full extraction for control/ambiguous names and special-type entries. | Reject empty path components, controls, truncated names, device/FIFO/symlink entries. Existing extraction traversal/overwrites/resource limits remain. Real ZIP regressions cover the new checks. |
| Credential sign-out | A queued/in-flight application credential save could race sign-out across windows. | Server-scoped generations and serialized keyring writes/deletes reject stale saves and order deletion after an in-flight write. Deterministic simulated-keyring tests cover both orderings. GVfs/system keyring native behavior remains a target-system gate. |
| Root mount helper | Existing leaf validation did not consistently walk existing parents. Proactive hardening; no reachable unprivileged exploit demonstrated. | Validate every parent as a real root-owned, non-group/world-writable directory before writing. The optional helper is not automatically invoked. |
| Copy staging | Partially copied local directory trees used filesystem default directory permissions. | Explicit 0700 on owned local staging directories. GIO creates private files; final no-overwrite publishing and cancellation cleanup retained. |
| Native web renderer | Subprocess sandbox state was implicit. | Explicitly enable the WebKit sandbox before WebView creation and expose only the packaged UI path read-only. CSP, exact local navigation policy, denied permissions/downloads and no-root rule retained. Actual WebKit execution remains untested here. |
| Terminal launch | New privileged bridge action could introduce command injection if implemented with shell strings. | URI-only API, fresh metadata, local mounted path, snapshot guard, fixed supported terminal CLIs, trusted system search path, literal argv/cwd and `shell=False`. A real benign recorder process verifies literal metacharacter paths; no graphical terminal was exercised. |
| Web deployment/CI | Production used unfrozen installs and default workflow permissions. | Frozen lock gate, moderate-or-higher audit gate, real typecheck/build steps, read-only token, no persisted checkout credentials, Dependabot config and hosting security headers. No automatic publish or secret-bearing PR workflow. |

## Source and test evidence

`tools/security_sweep.py` scans runtime Python AST for selected dangerous APIs
and checks project-specific bridge, CSP, iframe, tool-version and CI invariants.
It records a SHA-256 inventory of runtime Python files in
`test-results/security-sweep.json`. It is not taint analysis or a replacement for
Bandit/Semgrep, dependency auditing or manual review. `tools/audit-public-data.py`
checks known private-name fingerprints, recursively inspected archives and public
screenshot provenance. It is not a universal secret detector.

`desktop/tests/test_terminal_security.py` exercises real temporary state files,
symlinks, FIFOs, ZIP data, a benign process recorder and delayed fake keyring I/O.
`desktop/tests/ui_terminal.py` clicks the actual interface in Chromium, including
folder/file/background/sidebar/SMB actions and protected/virtual contexts. The
public transport reports simulation and cannot launch a process.

Read TEST-REPORT.md for counts from this delivery. Test doubles and Chromium are
not the full GTK/WebKit/GVfs application. No real user folders were used.

## Dependency review and unresolved gates

Direct pins inspected: Next.js 16.3.4, React/React DOM 19.2.8, pnpm changed to
10.34.5. The pinned Next version is newer than the 16.3.3 patch release discussed
in the August 2026 upstream advisory. The site is configured for static export
and unoptimized images, not a hosted Next API/image server. These facts do not
eliminate transitive or future vulnerabilities.

Container registry requests failed DNS resolution; Corepack could not fetch the
pinned pnpm. There is no fabricated lockfile, successful `pnpm audit`, dependency-
aware typecheck, or Next production build. CI and hosting require a real reviewed
lockfile before deployment. Distro-managed GTK, WebKit, GIO, libsecret, GVfs and
SMB client libraries must be kept patched with the target OS; they were not
installed/audited here. The package verification does not execute maintainer
scripts or establish apt repository provenance.

The hosting CSP adds base/object/frame restrictions but is **not a strict script
CSP**. A nonce/hash script policy needs the actual production export reviewed.
External files opened in viewers, terminals and editors execute outside the
OpenXplorer renderer sandbox. Filesystem/SMB races and a compromised same-user
process or administrator are not isolated by this application. Clipboard and
FileManager1 are desktop-session integrations, not barriers against that user.

The optional mount helper deliberately uses a root-only plaintext CIFS credential
file. Search caches contain private names/paths, not file contents. Do not upload
these, browser profiles, keyring exports or raw user logs. Password removal in a
Python process cannot guarantee zeroization of every memory copy.

## Primary upstream references checked

- pnpm workspaces: https://pnpm.io/workspaces
- pnpm tarball manifest traversal, fixed 10.34.5:
  https://github.com/pnpm/pnpm/security/advisories/GHSA-vq4v-j7r6-jq4m
- pnpm lockfile virtual-store traversal, fixed 10.34.5:
  https://github.com/pnpm/pnpm/security/advisories/GHSA-c59q-g84q-2gj5
- Next.js August 2026 security release:
  https://nextjs.org/blog/august-2026-security-release
- WebKit sandbox must be set before any web process:
  https://webkitgtk.org/reference/webkit2gtk/2.40.1/method.WebContext.set_sandbox_enabled.html
- pnpm audit (run against the actual resolved graph): https://pnpm.io/cli/audit

## Publication blockers not silently invented

The package maintainer address remains a marked `example.invalid` placeholder;
the owner must supply a real address/private report channel. The owner supplied openxplorer.app and it is now configured. No repository,
support SLA, code-signing key, successful deployment or production safety claim
is invented. Checksums establish artifact correspondence, not publisher
identity. All project and upstream license notices remain in the combined source.


## RC2 targeted review

The update guard addresses mixed-version native/UI processes without forcing active writes.
New remote ZIP reads accumulate short GIO reads with cancellation; the extraction policy
and safety limits are not weakened. AppStream catalog and desktop search updates do not
add a package repository or execute network installation commands. The extraction dialog
can explicitly delegate to the system archive application. See HOTFIX-RC2.md and the
current executed test report; this is not an independent penetration test.
