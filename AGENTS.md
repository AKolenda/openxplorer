# Working on this repository

Use pnpm for the website. Never replace the desktop with a web-only mock. Keep all native actions opt-in, especially file-manager defaults, browser profile writes, folder relocation and privileged mount setup.

Do not bulk-rename persisted `winspace` paths, DBus application IDs, keyring schemas or MIME handlers. Those are compatibility contracts, not visible branding.

Preserve the root AGPL-3.0-only grant, upstream MIT notice and file-level exceptions. Keep the corresponding-source archive and visible website source link synchronized with actual modifications.

For UI work, preserve semantic keyboard controls, reduced-motion support and responsive layout. Update shared TSX/CSS sources, then regenerate designs; do not hand-edit generated pitches. Keep docs JSON and Markdown consistent. Use fictional paths/shares in examples.

Report only tests you ran. In particular, offline TSX transpilation is not a Next.js build, and Chromium simulation is not native WebKit/SMB validation.

Public examples must follow `docs/PRIVACY.md`. Never copy customer names, addresses or screenshots into fixtures. Run the public-data audit after captures and after staging the complete release. Mobile preview gating must remove the iframe, not only conceal it with CSS.
