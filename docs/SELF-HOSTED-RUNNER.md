# OpenXplorer release runner

The `Build, release and deploy` workflow uses a dedicated Linux x64 self-hosted
runner labelled `openxplorer`. Pushes to `main` and manual dispatches on `main`
run the checks, publish a new desktop version, and deploy the website.
Pull-request events and non-main dispatches do not execute on this host.
Keep branch protection enabled and review changes before merging them.

The runner service runs as the unprivileged `openxplorer-runner` user in
`/opt/openxplorer-runner`. It starts at boot, restarts after failure, and is
limited to four CPU cores and 8 GiB RAM. Temporary files use
`/var/tmp/openxplorer-runner` on disk. Other repository runners are independent.
Do not grant this account sudo or copy personal SSH keys, browser profiles or
Wrangler OAuth credentials into its home directory.

Provision a Debian host with Git, GitHub CLI, Python venv/GI/CairoSVG,
GTK3/WebKitGTK 4.1, Secret introspection, desktop-file-utils, Xvfb, xauth,
libXtst, dbus-x11 and the Chromium dependencies installed by Playwright.
Unprivileged user namespaces must work: the native transport check retains
WebKit's sandbox. Download the official GitHub Actions runner and verify its
published SHA-256 before registering it to this repository with the
`openxplorer` label. Install its systemd service under the dedicated user.
The workflow installs its pinned pnpm and Playwright versions without sudo.

On LXC hosts whose overlaid `/proc` blocks nested sandbox mounts, install
`tools/runner-service.sh` root-owned and mode 0755 at
`/usr/local/libexec/openxplorer-runner-service`. Override only this service:

```ini
[Service]
ExecStart=
ExecStart=+/usr/local/libexec/openxplorer-runner-service
```

The root-owned helper creates a private mount namespace, mounts a fresh procfs,
and drops to the runner user with no-new-privileges before executing runner code.
It does not change the container's shared mounts or disable WebKit's sandbox.
Check the full nested mount, not only creation of a user namespace:
`bwrap --ro-bind / / --unshare-user --unshare-pid --proc /proc -- true`.

Repository configuration:

- Secret `CLOUDFLARE_API_TOKEN`: a dedicated deployment token with Workers
  Scripts Edit and Account Settings Read for the intended account, plus
  Workers Routes Edit and Zone Read for `openxplorer.app` only.
- Variable `CLOUDFLARE_ACCOUNT_ID`: the intended Cloudflare account ID.
- GitHub's per-job token provides release publication access. No personal
  GitHub token is stored on the runner.

Bump the package, desktop and website versions together, update the changelog,
regenerate docs/designs and fictional screenshots, and merge the reviewed
change into `main`. All checks must pass before publication. A release includes
the Debian installer, matching corresponding source and `SHA256SUMS`.
Existing release assets are left unchanged. Cloudflare deployment follows
release publication; rerun the workflow after fixing a deployment failure.

For recovery, check the repository's Actions runner status and inspect the
runner's systemd service and `_diag` logs on the build host. Restart that
service if necessary; do not restart another project's runner. Rotate the
Cloudflare token by replacing the repository secret, then revoke the old token
once a deployment with the replacement succeeds.
