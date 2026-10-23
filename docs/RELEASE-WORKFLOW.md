# OpenXplorer release workflow

The `Build, release and deploy` workflow (`.github/workflows/checks.yml`) runs
on GitHub-hosted runners. Pushes to `main` and manual dispatches on `main` run
the checks, publish a new desktop version, and deploy the website; every job
is guarded by `if: github.ref == 'refs/heads/main'`. Keep branch protection
enabled and review changes before merging them.

The native packages are built in distribution containers (`native-package` and
`native-flatpak` jobs). The `release` job downloads, verifies and publishes
them; it installs `desktop-file-utils` and `appstream` for the package checks.
The `website` job installs its pinned pnpm and Playwright versions, checks the
site and deploys it to Cloudflare.

`.github/CODEOWNERS` names the workflows, tools, package manifests, lockfile
and deployment configuration; the `Owner review for CI and deploy files`
ruleset on `main` requires the owner's approval for pull requests that change
them.

Repository configuration:

- Secret `CLOUDFLARE_API_TOKEN` in the `production` environment, which only
  `main` may deploy from (never a repository-level secret): a dedicated deployment token with Workers
  Scripts Edit and Account Settings Read for the intended account, plus
  Workers Routes Edit and Zone Read for `openxplorer.app` only.
- Variable `CLOUDFLARE_ACCOUNT_ID`: the intended Cloudflare account ID.
- GitHub's per-job token provides release publication access. No personal
  GitHub token is stored in the repository.

Bump the package, desktop and website versions together, update the changelog,
regenerate docs/designs and fictional screenshots, and merge the reviewed
change into `main`. All checks must pass before publication. A release includes
the native Debian installer, the RPMs, Arch package and Flatpak bundle,
matching corresponding source and `SHA256SUMS`.
Existing release assets are left unchanged. Cloudflare deployment follows
release publication. `Retry release and deploy` reruns the failed jobs of a
`main` run up to twice, which covers transient runner or network failures;
rerun the workflow by hand after fixing any other deployment failure.

For recovery, read the failed job's log in the repository's Actions tab. Rotate
the Cloudflare token by replacing the `production` environment secret, then
revoke the old token once a deployment with the replacement succeeds.
