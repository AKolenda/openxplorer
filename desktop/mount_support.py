# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Read mountinfo and prepare constrained, reviewable SMB mount plans.
No mounts or privileged writes occur in this module.
"""
from __future__ import annotations
import hashlib
import os
from pathlib import Path
import re
import shlex
from urllib.parse import quote, unquote, urlsplit
from core import normalise_location, require_share


def unescape_mount(value):
    return re.sub(r'\\([0-7]{3})', lambda m: chr(int(m[1], 8)), value)


def parse_mounts(text):
    rows = []
    for line in text.splitlines():
        try:
            before, after = line.split(' - ', 1)
            first, last = before.split(), after.split()
            rows.append({'root': unescape_mount(first[3]), 'path': unescape_mount(first[4]),
                         'fstype': last[0], 'source': unescape_mount(last[1]),
                         'options': last[2] if len(last) > 2 else ''})
        except (ValueError, IndexError):
            continue
    return rows


def read_mounts():
    return parse_mounts(Path('/proc/self/mountinfo').read_text())


def is_below(path, root):
    return path == root or path.startswith(root.rstrip('/') + '/')


def mount_for_path(path, mounts):
    candidates = [m for m in mounts if is_below(path, m['path'])]
    return max(candidates, key=lambda m: len(m['path']), default=None)


def remote_root(mount):
    if mount['fstype'] not in ('cifs', 'smb3'):
        return None
    try:
        uri = require_share(mount['source'])
        # A bind of a subdirectory has a non-root mountinfo root.
        if mount.get('root', '/') != '/':
            uri = normalise_location(uri.rstrip('/') + '/' + quote(mount['root'].lstrip('/'), safe='/'))
        return uri
    except (ValueError, TypeError):
        return None


def resolve_smb_path(uri, mounts):
    uri = require_share(uri).rstrip('/')
    candidates = []
    for mount in mounts:
        root = remote_root(mount)
        if not root:
            continue
        root = root.rstrip('/')
        # Host and share names case-insensitive; preserve path case beneath it.
        a, b = urlsplit(uri), urlsplit(root)
        ap, bp = unquote(a.path).strip('/').split('/'), unquote(b.path).strip('/').split('/')
        if a.netloc.lower() != b.netloc.lower() or ap[0].casefold() != bp[0].casefold():
            continue
        if len(ap) < len(bp) or ap[1:len(bp)] != bp[1:]:
            continue
        candidates.append((len(bp), os.path.join(mount['path'], *ap[len(bp):])))
    return max(candidates, default=(0, None), key=lambda x: x[0])[1]


def mount_plan(value, uid, gid):
    uri = require_share(value)
    u = urlsplit(uri)
    parts = unquote(u.path).strip('/').split('/')
    server, share = u.hostname, parts[0]
    # Intentionally narrower than general SMB addressing: unit files and mount
    # options get no user-provided syntax, credentials or arbitrary mount path.
    if u.port is not None or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9.-]{0,252}', server or ''):
        raise ValueError('The persistent mount assistant supports a hostname or IPv4 address without a port.')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9 ._$-]{0,79}', share):
        raise ValueError('This share name needs manual mounting. The assistant allows letters, numbers, spaces, dots, _, $, and hyphens.')
    if any(p in ('', '.', '..') for p in parts) or not (isinstance(uid, int) and isinstance(gid, int) and uid > 0 and gid >= 0):
        raise ValueError('Invalid mount destination or user identity.')
    identity = hashlib.sha256(f'{server.casefold()}/{share.casefold()}'.encode()).hexdigest()[:10]
    key = f'u{uid}s{identity}'
    mountpoint = f'/mnt/winspace/{key}'
    unit = 'mnt-winspace-' + key
    credentials = f'/etc/winspace/mount-credentials/{key}'
    source = f'//{server}/{share}'
    # Hash-based paths avoid systemd unit escaping and option injection.
    options = f'credentials={credentials},uid={uid},gid={gid},file_mode=0600,dir_mode=0700,forceuid,forcegid,nosuid,nodev,noexec,vers=3.0,_netdev'
    mount = f'''# Managed by OpenXplorer's explicit mount setup tool.
[Unit]
Description=OpenXplorer SMB mount {key}
[Mount]
What={source}
Where={mountpoint}
Type=cifs
Options={options}
TimeoutSec=20
'''
    automount = f'''# Managed by OpenXplorer's explicit mount setup tool.
[Unit]
Description=OpenXplorer on-demand SMB mount {key}
[Automount]
Where={mountpoint}
TimeoutIdleSec=300
[Install]
WantedBy=multi-user.target
'''
    command = shlex.join(['sudo', '/usr/bin/openxplorer-mount-share', '--share', source])
    return {'share': source, 'key': key, 'mountpoint': mountpoint,
            'targetPath': os.path.join(mountpoint, *parts[1:]), 'unit': unit,
            'credentials': credentials, 'mountUnit': mount, 'automountUnit': automount,
            'command': command,
            'removeCommand': shlex.join(['sudo', '/usr/bin/openxplorer-mount-share', '--share', source, '--remove'])}
