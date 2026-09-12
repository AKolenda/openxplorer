# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Merge saved locations, active mounts and this application's visited servers.
No network I/O and no automatic persistent bookmarks. Port 445 is canonicalized;
host aliases are not guessed (nor used to share credentials).
"""
from urllib.parse import urlsplit, unquote
from pathlib import Path
from core import normalise_location


def network_key(uri):
    uri = normalise_location(uri)
    u = urlsplit(uri)
    if u.scheme != 'smb':
        return uri
    return ('smb', u.hostname.lower(), u.port or 445, unquote(u.path).rstrip('/').casefold())


def merge_network_locations(saved, mounts, stable=(), visited=()):
    result = {}
    def add(item, saved=False, connected=False, kind='share'):
        try:
            uri = normalise_location(item['uri'])
            if not uri.startswith('smb:') and kind != 'mount': return
            key = network_key(uri)
            u = urlsplit(uri)
            label = item.get('label') or unquote(u.path).rstrip('/').split('/')[-1] or u.hostname
            value = result.setdefault(key, {'uri':uri, 'label':label, 'saved':False,
                                           'connected':False, 'kind':kind, 'isShared':True})
            if saved: value.update(label=label, uri=uri)
            value['saved'] |= saved
            value['connected'] |= connected
        except (KeyError, TypeError, ValueError):
            return
    for item in saved: add(item, saved=True, connected=item.get('connected',False))
    for item in mounts:
        if item.get('mounted') and item.get('uri','').startswith('smb:'):
            add(item, connected=True, kind='server' if urlsplit(item['uri']).path in ('','/') else 'share')
    for mount in stable:
        if mount.get('fstype') in ('cifs','smb3') and mount.get('path'):
            add({'uri':Path(mount['path']).as_uri(),'label':mount.get('label') or Path(mount['path']).name}, connected=True, kind='mount')
    # A successfully browsed host remains visible during this application session,
    # even if the browse backend has not produced a GMount yet.
    for item in visited: add(item, kind='server' if urlsplit(item['uri']).path in ('','/') else 'share')
    return list(result.values())
