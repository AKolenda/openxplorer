# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""File-opening policy; intentionally independent of GTK and of cached labels."""
from __future__ import annotations

SELF_IDS = {'io.winspace.Development.desktop', 'io.winspace.OpenXplorer.desktop'}
ZIP_TYPES = {'application/zip', 'application/x-zip', 'application/x-zip-compressed'}


def activation_kind(entry: dict) -> str:
    # The caller must supply freshly queried metadata, not an old search row.
    kind = entry.get('kind')
    if kind == 'directory' or (kind not in ('file', 'special', 'symlink') and entry.get('isDir') is True):
        return 'directory'
    if kind in ('special', 'unknown', 'symlink'):
        raise ValueError('This item is not a regular file or a readable folder.')
    if entry.get('contentType') in ZIP_TYPES or str(entry.get('name', '')).lower().endswith('.zip'):
        return 'archive'
    return 'file'


def choose_application(apps, default=None):
    """Never launch our own SMB scheme handler while opening a regular file."""
    candidates = ([default] if default else []) + list(apps)
    for app in candidates:
        if app and app.get_id() not in SELF_IDS and (app.supports_files() or app.supports_uris()):
            return app
    raise ValueError('No application is installed for this file type. Use Open with… to choose one.')
