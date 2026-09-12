# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Previous versions from exposed snapshot/backup folders.

This is NOT an SMB FSCTL_SRV_ENUMERATE_SNAPSHOTS client and does not create
snapshots. Backends must expose readable snapshot directories. A separate
provider can be added later without inventing history for unsupported servers.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import re
import tempfile
import threading
from urllib.parse import quote, unquote, urlsplit
from core import normalise_location, validate_name

MARKERS = {'.snapshot', '.snapshots', '#snapshot'}


def conventional_snapshot(uri):
    parts = unquote(urlsplit(uri).path).split('/')
    return any(p in MARKERS or p.startswith('@GMT-') for p in parts) or any(
        parts[i:i+2] == ['.zfs', 'snapshot'] for i in range(len(parts)-1))


def within(uri, root):
    return uri.rstrip('/') == root.rstrip('/') or uri.startswith(root.rstrip('/') + '/')


def child_uri(uri, name):
    validate_name(name)
    return uri.rstrip('/') + '/' + quote(name, safe='')


def relative_uri(uri, root):
    if not within(uri, root):
        raise ValueError('This item is outside the live folder.')
    return uri[len(root.rstrip('/')):].lstrip('/')


class PreviousVersions:
    def __init__(self, directory: Path, provider=None):
        self.directory = directory
        self.path = directory / 'snapshot-sources.json'
        self.provider = provider
        self.lock = threading.RLock()
        self.discovered = set()

    def sources(self):
        try:
            data = json.loads(self.path.read_text())
            if not isinstance(data, list): return []
            clean = []
            for item in data[:64]:
                try:
                    live = normalise_location(item['live'])
                    snapshots = normalise_location(item['snapshots'])
                    if live != snapshots and not within(live, snapshots):
                        clean.append({'live': live, 'snapshots': snapshots, 'layout': item.get('layout', 'direct') if item.get('layout', 'direct') in ('direct', 'snapper') else 'direct'})
                except (ValueError, TypeError, KeyError): pass
            return clean
        except (OSError, ValueError): return []

    def configure(self, live, snapshots, layout='direct', remove=False):
        live, snapshots = normalise_location(live), normalise_location(snapshots)
        if layout not in ('direct', 'snapper'):
            raise ValueError('Unknown snapshot folder layout.')
        if live == snapshots or within(live, snapshots):
            raise ValueError('The snapshot folder must not contain the current live folder.')
        with self.lock:
            sources = [s for s in self.sources() if s['live'] != live]
            if not remove:
                if len(sources) >= 64: raise ValueError('At most 64 snapshot sources are supported.')
                sources.append({'live': live, 'snapshots': snapshots, 'layout': layout})
            self.directory.mkdir(mode=0o700, exist_ok=True, parents=True)
            fd, path = tempfile.mkstemp(prefix='.versions-', dir=self.directory)
            try:
                with os.fdopen(fd, 'w') as out:
                    os.fchmod(out.fileno(), 0o600)
                    json.dump(sources, out); out.flush(); os.fsync(out.fileno())
                os.replace(path, self.path)
            finally:
                if os.path.exists(path): os.unlink(path)
        return sources

    def roots(self):
        with self.lock:
            return {s['snapshots'] for s in self.sources()} | set(self.discovered)

    def annotate(self, entries):
        # One settings read per batch, not one read per directory entry.
        roots = self.roots()
        return [{**entry, 'readOnly': conventional_snapshot(entry['uri']) or
                 any(within(entry['uri'], root) for root in roots)} for entry in entries]

    def protected(self, uri):
        uri = normalise_location(uri)
        return conventional_snapshot(uri) or any(within(uri, root) for root in self.roots())

    def assert_writable(self, uri):
        if self.protected(uri):
            raise ValueError('Previous-version locations are read-only in OpenXplorer. Restore a copy to a different folder first.')
        return uri

    def candidates(self, uri, is_dir):
        configured = sorted([s for s in self.sources() if within(uri, s['live'])], key=lambda s: len(s['live']), reverse=True)
        if configured:
            return configured[:1]
        u = urlsplit(uri)
        parts = u.path.strip('/').split('/')
        if u.scheme == 'smb' and parts and parts[0]:
            live = 'smb://' + u.netloc + '/' + parts[0]
            return [{'live': live, 'snapshots': live + '/' + suffix, 'layout': layout}
                    for suffix, layout in (('.snapshot', 'direct'), ('%23snapshot', 'direct'), ('.zfs/snapshot', 'direct'), ('.snapshots', 'snapper'))]
        # Local layouts vary; a filesystem root cannot be inferred from a path.
        # Probe near the selected item only; users can configure the live root.
        base = uri if is_dir else uri.rsplit('/', 1)[0]
        return [{'live': base, 'snapshots': base + '/' + suffix, 'layout': layout}
                for suffix, layout in (('.snapshot', 'direct'), ('.zfs/snapshot', 'direct'), ('.snapshots', 'snapper'))]

    def list(self, uri, is_dir, cancel):
        if self.provider is None: raise ValueError('A filesystem provider is required.')
        uri = normalise_location(uri)
        results, warnings, scanned, truncated = [], [], [], False
        for source in self.candidates(uri, is_dir):
            cancel.check()
            try:
                entries, more = self.provider.children(source['snapshots'], cancel, limit=100)
                with self.lock:
                    self.discovered.add(source['snapshots'])
                scanned.append(source['snapshots'])
                truncated |= more
            except Exception as exc:
                cancel.check()
                warnings.append(f"{source['snapshots']}: {exc}")
                continue
            for entry in entries:
                cancel.check()
                if not entry.get('isDir') or entry.get('symlink'): continue
                try:
                    version_root = child_uri(source['snapshots'], entry['name'])
                    if source['layout'] == 'snapper': version_root += '/snapshot'
                    relative = relative_uri(uri, source['live'])
                    candidate = version_root + ('/' + relative if relative else '')
                    info = self.provider.inspect(candidate, cancel)
                    if info.get('symlink'): continue
                    results.append({**info, 'uri': candidate, 'label': entry['name'],
                                    'snapshotRoot': version_root, 'source': source['snapshots'],
                                    'snapshotModified': entry.get('modified'), 'readOnly': True})
                except Exception as exc:
                    cancel.check()
                    # Missing in this historical snapshot is normal, while
                    # permission/network failures must not become 'no history'.
                    if getattr(exc, 'code', '') != 'not-found':
                        warnings.append(f"{entry.get('name', 'Snapshot')}: {exc}")
                if len(results) >= 100:
                    truncated = True; break
            if len(results) >= 100: break
        results.sort(key=lambda e: e['label'], reverse=True)
        return {'versions': results, 'sources': scanned, 'warnings': warnings[:8],
                'truncated': truncated, 'provider': 'Exposed snapshot folders',
                'protocolEnumeration': False, 'canCreateSnapshots': False,
                'message': 'No matching previous versions were found in readable snapshot folders. This does not prove that your server has no snapshots or backups.' if not results else '',
                'configured': [s for s in self.sources() if within(uri, s['live'])]}
