# Location compatibility fixtures

These synthetic inputs cover local paths, invalid input, devices, SMB, labels,
breadcrumbs and file sizes. Expected output was captured from the Python desktop
implementation and JavaScript display helpers. No personal filesystem is read.
The Rust tests embed the JSON and do not require Python, Node, locales or `/tmp` files.

- `desktop/core.py` SHA-256: `c37655f1f93480f6789f9303e7032c87181cb25d1fb269bd458164b7cd9e8daa`
- `desktop/ui/app.js` SHA-256: `6af4c55b34135685abb2e41b318d54cbf97353189bd02f0a3bfc483cf5d1861e`

To regenerate from the repository root:

```sh
python3 native/crates/ox-core/tests/location_fixtures/generate_python.py desktop native/crates/ox-core/tests/location_fixtures/python.json
node native/crates/ox-core/tests/location_fixtures/generate_javascript.cjs desktop/ui/app.js native/crates/ox-core/tests/location_fixtures/javascript.json
```

The JavaScript capture uses line ranges from the recorded source version. Review
those ranges against the source before regenerating after JavaScript changes.
The Rust comparison deliberately normalizes trailing slashes in device parent
paths; navigation uses the canonical path without a trailing slash except at roots.
