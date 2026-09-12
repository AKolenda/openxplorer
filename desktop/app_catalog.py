# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Deduplicate installed launchers without launching commands or changing defaults."""
from __future__ import annotations
import re

EDITOR_IDS = frozenset({'code.desktop', 'com.visualstudio.code.desktop',
                       'codium.desktop', 'com.vscodium.codium.desktop', 'code-insiders.desktop'})


def unique_applications(apps, preferred_id=None):
    """Prefer the default/primary launcher; omit hidden URL helpers.

    A distribution package and a Flatpak can have different desktop IDs but
    the same visible name. Offer one representative for each visible name.
    Distinct names (e.g. Code Insiders and VSCodium) remain distinct choices.
    """
    groups = {}
    for app in apps:
        identifier = app.get_id()
        if not identifier or identifier == 'io.winspace.Development.desktop':
            continue
        if not app.should_show() or re.search(r'url[-_]?handler', identifier, re.I):
            continue
        if not app.supports_files() and not app.supports_uris():
            continue
        name = ' '.join((app.get_display_name() or identifier).split()).casefold()
        rank = (identifier != preferred_id, identifier not in ('code.desktop', 'codium.desktop'), identifier)
        if name not in groups or rank < groups[name][0]:
            groups[name] = (rank, app)
    return [groups[key][1] for key in sorted(groups)]


def editor_shortcuts(apps):
    return [{'id': a.get_id(), 'name': a.get_display_name()}
            for a in unique_applications(a for a in apps if a.get_id() in EDITOR_IDS)]
