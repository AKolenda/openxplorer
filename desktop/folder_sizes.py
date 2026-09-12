# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Bounded, read-only logical folder-size scans, separate from filename indexing.

No content is downloaded. No links, special files, nested filesystem mounts, or
snapshot collections are followed. Report incomplete results explicitly. The
provider contract also makes traversal testable without GI or a NAS.
"""
from __future__ import annotations
import os
from pathlib import Path
import stat
import time
from urllib.parse import unquote, urlsplit
from core import normalise_location, is_smb_server

EXCLUDED = frozenset({'.zfs', '.snapshot', '#snapshot', '.snapshots'})
MAX_ENTRIES = 1_000_000
MAX_SECONDS = 300


class LocalSizeProvider:
    """Local metadata provider; does not follow symbolic links."""
    def __init__(self):
        from mount_support import read_mounts
        self.mounts = {m['path'].rstrip('/') or '/' for m in read_mounts()}

    @staticmethod
    def path(uri):
        return Path(unquote(urlsplit(uri).path))

    def inspect(self, uri, cancel):
        cancel.check()
        p = self.path(uri)
        return self.entry(p, p.lstat())

    def entry(self, p, s):
        return {'uri': p.as_uri(), 'name': p.name, 'size': s.st_size,
                'isDir': stat.S_ISDIR(s.st_mode), 'symlink': stat.S_ISLNK(s.st_mode),
                'regular': stat.S_ISREG(s.st_mode), 'filesystem': str(s.st_dev),
                'identity': f'{s.st_dev}:{s.st_ino}', 'mountpoint': str(p) in self.mounts}

    def children(self, uri, cancel):
        with os.scandir(self.path(uri)) as entries:
            for item in entries:
                cancel.check()
                try:
                    yield self.entry(Path(item.path), item.stat(follow_symlinks=False))
                except OSError:
                    yield {'unreadable': True, 'name': item.name}


class GioSizeProvider:
    """GIO metadata-only enumeration for SMB (imports GI only when used)."""
    ATTRS = 'standard::name,standard::type,standard::size,standard::is-symlink,id::filesystem'

    def __init__(self):
        from gi.repository import Gio
        from gio_backend import raw
        self.Gio, self.raw = Gio, raw

    def entry(self, file, info):
        G = self.Gio
        size = info.get_size() if info.has_attribute('standard::size') else None
        return {'uri': file.get_uri(), 'name': info.get_name(), 'size': size,
                'isDir': info.get_file_type() in (G.FileType.DIRECTORY, G.FileType.MOUNTABLE),
                'regular': info.get_file_type() == G.FileType.REGULAR,
                'symlink': info.get_is_symlink() or info.get_file_type() == G.FileType.SYMBOLIC_LINK,
                'filesystem': info.get_attribute_string('id::filesystem')}

    def inspect(self, uri, cancel):
        G = self.Gio
        file = G.File.new_for_uri(uri)
        info = file.query_info(self.ATTRS, G.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, self.raw(cancel))
        return self.entry(file, info)

    def children(self, uri, cancel):
        G = self.Gio
        file = G.File.new_for_uri(uri)
        en = file.enumerate_children(self.ATTRS, G.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, self.raw(cancel))
        try:
            while True:
                cancel.check()
                info = en.next_file(self.raw(cancel))
                if info is None:
                    break
                yield self.entry(en.get_child(info), info)
        finally:
            en.close(None)


def scan_folder(uri, cancel, progress=None, provider=None, *, max_entries=MAX_ENTRIES,
                max_seconds=MAX_SECONDS, clock=time.monotonic):
    """Return logical bytes, coverage and timestamp, never a fabricated total.

    Limits are checked between metadata operations; a blocked filesystem read
    may take longer to return. GIO requests receive the native cancellation.
    Repeated hard-link identities are counted once when reported by a provider.
    """
    uri = normalise_location(uri)
    if is_smb_server(uri):
        raise ValueError('Open or select a share first, not the whole SMB server.')
    provider = provider or (LocalSizeProvider() if uri.startswith('file:') else GioSizeProvider())
    if max_entries < 1 or max_seconds <= 0:
        raise ValueError('Scan limits must be positive.')
    root = provider.inspect(uri, cancel)  # Let NOT_MOUNTED reach the native retry.
    if not root.get('isDir') or root.get('symlink'):
        raise ValueError('Select a directory, not a file or symbolic link.')
    start = clock()
    result = {'uri': uri, 'bytes': 0, 'files': 0, 'folders': 0, 'entries': 0,
              'skipped': 0, 'errors': 0, 'status': 'scanning', 'reason': '',
              'metric': 'logical file bytes', 'updated': None}
    stack, visited, identities = [uri], set(), set()
    last = start - 1
    fs = root.get('filesystem')
    def cancelled():
        return cancel.is_cancelled()
    def publish(force=False):
        nonlocal last
        now = clock()
        if progress and (force or now-last >= .2):
            progress(dict(result))
            last = now
    publish(True)
    stopped = False
    while stack and not stopped:
        if cancelled():
            result.update(status='cancelled', reason='Cancelled by user')
            break
        if clock()-start >= max_seconds:
            result.update(status='partial', reason='Time limit reached')
            break
        current = stack.pop()
        if current in visited:
            result['skipped'] += 1
            continue
        visited.add(current)
        try:
            for item in provider.children(current, cancel):
                if cancelled():
                    result.update(status='cancelled', reason='Cancelled by user'); stopped = True; break
                if result['entries'] >= max_entries or clock()-start >= max_seconds:
                    result.update(status='partial', reason='Scan limit reached'); stopped = True; break
                result['entries'] += 1
                if item.get('unreadable'):
                    result['errors'] += 1
                    continue
                if item.get('symlink'):
                    result['skipped'] += 1
                    continue
                if item.get('isDir'):
                    if (item['name'] in EXCLUDED or item.get('mountpoint') or
                        (fs and item.get('filesystem') and item['filesystem'] != fs)):
                        result['skipped'] += 1
                        continue
                    result['folders'] += 1
                    stack.append(item['uri'])
                elif item.get('regular') and isinstance(item.get('size'), int) and item['size'] >= 0:
                    identity = item.get('identity')
                    if identity and identity in identities:
                        continue
                    if identity:
                        identities.add(identity)
                    result['bytes'] += item['size']
                    result['files'] += 1
                else:
                    result['skipped'] += 1
                publish()
        except Exception:
            if cancelled():
                result.update(status='cancelled', reason='Cancelled by user'); break
            # A share may need mounting at first enumeration, not query_info.
            # Propagate a root error to the host's native authentication/retry.
            if current == uri and result['entries'] == 0:
                raise
            # Failed subtrees are not presented as empty directories.
            result['errors'] += 1
        publish()
    if result['status'] == 'scanning':
        result['status'] = 'partial' if result['errors'] or result['skipped'] else 'complete'
        if result['status'] == 'partial':
            result['reason'] = 'Some links, mounts, snapshot collections or unreadable entries were excluded'
    result['updated'] = time.time()
    result['elapsedSeconds'] = round(clock()-start, 3)
    publish(True)
    return result
