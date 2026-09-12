# Cloudflare setup

OpenXplorer uses **openxplorer.app**. The website is a Next.js static export,
served by Workers Static Assets from `apps/web/out`. The desktop remains an
independent native application. No database, server-side Next.js adapter or
runtime application secret is needed for this site.

## Keep two Wrangler logins

The pinned Wrangler version supports named OAuth profiles. Cloudflare currently
labels this feature experimental. Create the project profile once, signing in
to the account that owns the domain:

```sh
pnpm install --frozen-lockfile
pnpm cf:login
pnpm cf:activate
pnpm cf:profiles
pnpm cf:whoami
```

`cf:login` runs `wrangler auth create openxplorer`; `cf:activate` binds that
profile to this repository directory. It does not log out or replace `default`.
Other project directories continue using their existing bindings or default
profile. Do not run plain `wrangler login` to create this second identity.
For a CLI that cannot open a browser, append `--browser=false` to `pnpm cf:login`.
Authorize the link while the CLI is waiting; expired requests must be restarted.

The dev, dry-run and deploy scripts also pass `--profile openxplorer` explicitly.
Wrangler's auth commands and `whoami` do not accept `--profile`; `cf:whoami`
uses the directory binding. OAuth credentials and directory bindings are local
Wrangler state outside the published source, not project environment variables.

An exported `CLOUDFLARE_API_TOKEN` takes priority over OAuth profiles. An
`account_id` configuration or `CLOUDFLARE_ACCOUNT_ID` also overrides account
selection. Check the intended identity before deploying and do not combine
unrelated account credentials with these project scripts. CI should use its
own scoped token in the CI secret store, not a copied local OAuth profile.

See [Cloudflare authentication profiles](https://developers.cloudflare.com/workers/wrangler/profiles/).

## Build and deploy

`wrangler.jsonc` declares the `openxplorer-site` Worker and the root custom
domain. Automatic trailing-slash handling matches directory-index pages while
also serving the standalone HTML preview. Missing routes use the exported 404
page. `_headers` provides the site's security headers and long-lived caching
only for fingerprinted Next assets. Release downloads are not hosted by the website.
The `no-transform` directive also prevents Cloudflare's automatic analytics
beacon injection, preserving the site's no-analytics behavior and the demo's
hashed-script CSP. Cloudflare documents this behavior in its
[Web Analytics setup guide](https://developers.cloudflare.com/web-analytics/get-started/).

```sh
pnpm check
pnpm audit --audit-level=moderate
pnpm designs
pnpm release
python3 tools/audit-public-data.py
pnpm build
pnpm cf:dry-run
pnpm cf:dev
```

Check the local Worker URL, desktop preview, mobile iframe removal, documentation,
404 response and every download before deployment. Python browser tests need
Playwright and Chromium; `tests/test_launch_production.py` checks the actual
HTTP-served Next export and React hydration. The standalone renderer alone
does not validate these behaviors.

Before the first deploy, verify the authenticated account owns the active
`openxplorer.app` zone and inspect existing DNS/Worker mappings. Resolve an
existing site or CNAME deliberately rather than overwriting it implicitly.
The deploy command publishes the built site and attaches the declared hostname:

```sh
pnpm cf:deploy
```

Cloudflare Custom Domains creates the domain mapping and certificate. The
configuration disables `workers.dev` and version preview URLs. The `www`
hostname is not configured by this root-domain deployment. Verify final HTTPS
responses, keyboard/mobile behavior, iframe readiness, download hashes and
the visible corresponding-source offer before announcing the release.

See [Static Assets](https://developers.cloudflare.com/workers/static-assets/)
and [Custom Domains](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/).

## Agent tools are separate from Wrangler

Cloudflare's [official agent setup](https://developers.cloudflare.com/agent-setup/prompt.md)
installs the `cloudflare/skills` package and registers these Codex MCP servers:

| Server | URL |
| --- | --- |
| cloudflare | https://mcp.cloudflare.com/mcp |
| cloudflare-docs | https://docs.mcp.cloudflare.com/mcp |
| cloudflare-bindings | https://bindings.mcp.cloudflare.com/mcp |
| cloudflare-builds | https://builds.mcp.cloudflare.com/mcp |
| cloudflare-observability | https://observability.mcp.cloudflare.com/mcp |

`codex mcp login cloudflare` authenticates the general API MCP server. This does
not authenticate Wrangler or replace any Wrangler profile. The docs server is
public; other MCP servers can request their own OAuth consent when first used.
Restart the coding agent after registration to load newly configured servers.

Keep all account credentials, local logs and `.wrangler` state out of Git and
the corresponding-source archive. Consult the [public-release checklist](PUBLIC-RELEASE-CHECKLIST.md)
for source review and the [native roadmap](NATIVE-EXPERIENCE-ROADMAP.md) for
remaining desktop work. Hosting a release candidate does not mark it stable.
