# Public demo data

All published examples, screenshot inputs and browser-preview files are fictional. The demo uses `/home/demo`, `studio-nas`, `archive-nas`, `Shared` and `Launch planning`. They do not identify a person, a real server or a customer project.

Do not use customer screenshots or copy folder names, usernames, hostnames, share names or addresses from bug-report images into fixtures. Capture public screenshots only with `python3 tools/capture-screenshots.py` and `python3 tools/capture-website.py` in isolated browser contexts. Run `python3 tools/audit-public-data.py --private-terms /path/outside/repo/private-terms.json` before a privacy-reviewed release. Supply a JSON array of identifiers to exclude, stored outside the repository. No customer identifiers or fingerprints are embedded in the audit script. Without that argument, the audit checks packaging and image provenance but cannot check a private denylist.

The simulated preview uses its own versioned storage key. This release does not import old demo pins. This applies only to the demo; the native app keeps the user’s actual settings and files.

The release tool discards older archives and screenshot captures rather than bundling them. The audit checks text, nested ZIP/deb payloads, and image provenance. A byte scan is not OCR: screenshot safety also depends on generating images exclusively from the reviewed fixtures and visually reviewing those captures.
