# Website deployment

Publish a static site linked to its public source repository.

## Configure before publication

Edit apps/web/lib/site.ts for public identity and the preferred design. The selected design is Zorin / Horizon; /concepts/windows/ and /concepts/vercel/ remain alternatives. The canonical project URL is https://openxplorer.app.

The public source repository is https://github.com/AKolenda/openxplorer. Package calls to action link to https://github.com/AKolenda/openxplorer/releases. Source links lead to the repository. The website has no direct download links. Keep the repository public whenever the website links to it.

## Prepare a local release

The release tool creates and verifies a local Debian package, corresponding-source archive and checksums under dist/. It also removes any legacy website download directories. The website itself carries no release binaries; its source links lead to the public GitHub repository and its package links lead to GitHub Releases.

```sh
pnpm check
pnpm audit --audit-level=moderate
pnpm designs
pnpm release
python3 tools/audit-public-data.py
pnpm build
```

## Cloudflare Workers Static Assets

The root wrangler.jsonc serves apps/web/out with Cloudflare Workers Static Assets. The site remains Next.js with output: export. The configured custom domain is openxplorer.app; workers.dev and preview URLs are disabled. Directory-style HTML routing and the exported 404 page are configured.

Use the project-local Wrangler and the dedicated openxplorer OAuth profile. Named profiles are experimental in the pinned CLI and keep this project separate from default-profile authentication. CLOUDFLARE_API_TOKEN takes precedence over profiles; verify the active identity before publishing. Do not place credentials in source files.

After OAuth completes, activate the profile and verify its account. The local development, dry-run and deployment scripts explicitly pass --profile openxplorer. A dry run checks packaging and does not deploy. Full setup and account/domain checks are documented in docs/CLOUDFLARE-SETUP.md.

```sh
pnpm cf:login
pnpm cf:activate
pnpm cf:profiles
pnpm cf:whoami

# After building and auditing the release above:
pnpm cf:dry-run
pnpm cf:dev
# After reviewing the export and confirming the intended account/domain:
pnpm cf:deploy
```

## Check before announcing

Run browser checks against the actual HTTP-served Next.js export. Test hydration, routes, keyboard navigation, mobile menus, search and every public-repository link. Confirm that no release binaries are emitted into the website export. Also verify Cloudflare routing/security headers and 404 behavior. The site has no application sign-in, analytics scripts, external fonts or tracking pixels.

The application preview is embedded in an iframe with only allow-scripts; it has an opaque sandbox origin. The tour bridge accepts only fixed demo commands from its parent window. The page checks message sources before showing status. The preview’s CSP blocks network requests. Never replace this with a native credential or filesystem bridge.

## Release launch checks

Dependency installation, pnpm check, the real pnpm build and production-export browser checks have passed. A dedicated Wrangler OAuth profile can deploy this site independently of the default login. Verify the current domain and deployment status before announcing a release. See TEST-REPORT.md for executed checks and docs/PUBLIC-RELEASE-CHECKLIST.md for source-publication preparation. Stable 1.0.0 is published on GitHub Releases; re-run these checks before each new release.

---

OpenXplorer 1.1.2. Project-authored documentation: AGPL-3.0-only.
