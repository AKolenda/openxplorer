#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Create an offline single-file preview from the SAME desktop UI sources."""
from pathlib import Path
import base64
import hashlib
ROOT = Path(__file__).resolve().parents[1]
html = (ROOT / 'ui/index.html').read_text()
css = (ROOT / 'ui/style.css').read_text()
hashes = []
for filename in ('bootstrap.js', 'text-size.js', 'type-select.js', 'snapshot-meta.js', 'app.js'):
    js = (ROOT / 'ui' / filename).read_text()
    digest = base64.b64encode(hashlib.sha256(js.encode()).digest()).decode()
    hashes.append("'sha256-" + digest + "'")
    html = html.replace('<script src="' + filename + '"></script>', '<script>' + js + '</script>')
# Guided website walkthrough is preview-only, never loaded by the native app.
showcase = (ROOT / 'demo/showcase.js').read_text()
digest = base64.b64encode(hashlib.sha256(showcase.encode()).digest()).decode()
hashes.append("'sha256-" + digest + "'")
html = html.replace('</body>', '<script>' + showcase + '</script></body>')
html = html.replace("script-src 'self'", 'script-src ' + ' '.join(hashes))
html = html.replace('<link rel="stylesheet" href="style.css">', '<style>' + css + '</style>')
(ROOT / 'preview.html').write_text(html)
print(ROOT / 'preview.html')
