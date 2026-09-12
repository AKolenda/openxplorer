# Publishing the source repository

This checklist covers publishing reviewable source and keeping website links synchronized with the public repository. It does not change the release candidate into stable 1.0; the separate [stable-release checklist](RELEASE-CHECKLIST.md) covers target-machine validation.

## Review the source snapshot

- Keep editable Python, desktop UI, website TSX/CSS, documentation, lockfile, tests and build scripts in Git. Keep generated installers, archives, designs, dependency directories, local test captures and build output out of Git.
- Review the entire staged diff and file list, including dotfiles. `.gitignore` excludes `.wrangler/`, `.dev.vars*`, private denylist files, environment files, local databases and common credential files. Ignore rules do not remove a file that is already tracked, and they do not prove that arbitrary source or documentation is free of secrets.
- Keep credentials and private review identifiers outside the repository. Example environment files may contain only public placeholders. Review source-archive exclusions as well as `.gitignore`: the optional local corresponding-source ZIP is assembled by `tools/release.py`, not by Git or the website.
- Preserve `LICENSE`, `NOTICE`, `THIRD_PARTY_NOTICES.md`, both upstream MIT notice copies, desktop license files and existing file-level exceptions. Keep the AGPL-3.0-only grant unchanged.
- Verify the actual repository URL, website URL, maintainer/contact details and private reporting instructions. Configure the repository's private vulnerability reporting before inviting sensitive reports. Do not invent accounts or contact addresses to fill metadata.
- Keep all public examples fictional. Review screenshot provenance and the visual content under [PRIVACY.md](PRIVACY.md). Historical local test captures are not public fixtures.

Useful local review commands, after Git has been initialized:

```sh
git status --short
git diff --cached --stat
git diff --cached
git ls-files
git status --short --ignored
```

## Validate the publishable source

Use the committed pnpm lockfile and the version declared in `package.json`:

```sh
pnpm install --frozen-lockfile
pnpm audit --audit-level=moderate
pnpm test:desktop
node --test desktop/tests/*.test.cjs
pnpm security:source
pnpm check
pnpm build
python3 tools/audit-public-data.py
```

Review dependency-audit findings before changing versions. The public-data audit without an external private denylist verifies provenance and packaging, not the absence of every private identifier. Record the scope and outcome of each executed check; do not copy historical pass counts into a new release report.

The GitHub workflow runs website typechecking/build/audit and production-export browser checks for React hydration and mobile preview removal. It also covers desktop unit and JavaScript tests, source-archive exclusions, a browser UI contract, native GTK/WebKit transport into Chromium on isolated X11, package verification and a staged public-data audit. It declares its native prerequisites and uses read-only repository access with pinned action commits. It has no deployment credentials or publishing step. Hosted CI is verified only after its actual run completes.

## Keep the website pointed at public source

```sh
pnpm designs
pnpm release
python3 tools/audit-public-data.py
pnpm build
```

`pnpm release` builds and verifies the Debian package and creates a corresponding-source ZIP and checksums under local, ignored output directories. It removes legacy website download directories. Re-run it after any source change intended for a distributed build. Regenerating standalone designs is separate from the real Next.js export.

Inspect the resulting source ZIP and confirm it contains the instructions and inputs needed to reproduce the application, including the lockfile and license notices. Confirm every website project/install call to action resolves to the public GitHub repository and that the website export contains no release binaries. If release assets are published later, publish the installer, matching source ZIP and `SHA256SUMS` together on GitHub; generated binaries do not belong in the source history.

Run the public-data audit again after staging the complete release, including any newly generated captures. Use `--private-terms` with a reviewed JSON list stored outside the repository when a private-data review calls for it. Keep such lists out of Git, workflow logs and corresponding-source archives.

## Repository settings

Before linking the repository publicly, verify its actual visibility and that visitors without an account can reach the intended source. An existing private repository does not provide public corresponding-source access. Enable private vulnerability reporting and secret scanning/push protection where available. Require the actual passing `website` and `desktop` checks on the protected branch. Review Dependabot dependency and pinned-action updates. Add real maintainer ownership where appropriate; the repository does not include a fabricated `CODEOWNERS` identity.

Keep deployment credentials in the deployment platform's secret store or a protected deployment environment. Source-validation workflows should remain usable for forked pull requests without those credentials. Publishing source, releasing binaries and deploying the website are separate actions; the checks workflow performs validation only.
