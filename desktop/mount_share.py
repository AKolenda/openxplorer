#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Explicit administrator-only setup for a persistent, on-demand SMB3 mount.

NOT called automatically by the GUI. Review the printed plan; this tool prompts
in the terminal. It never edits fstab or moves Downloads/Documents. Passwords
are stored in a root-only credentials file for mount.cifs, NOT the user keyring.
"""
from __future__ import annotations
import argparse
import getpass
import os
from pathlib import Path
import pwd
import shutil
import stat
import subprocess
import sys
# The installed helper uses Python isolated mode, importing only its root-owned
# application directory in addition to the standard library.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from mount_support import mount_plan

MARKER = '# Managed by OpenXplorer\'s explicit mount setup tool.'


def secure_dir(path, mode):
    """Only use root-owned non-symlink parents not writable by other users."""
    path = Path(path)
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('Administrative paths must be absolute, without parent traversal.')
    if path != path.parent:
        secure_dir(path.parent, 0o755)
    try:
        info = path.lstat()
    except FileNotFoundError:
        path.mkdir(mode=mode)
        info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022:
        raise ValueError(f'Refusing unsafe directory: {path}')


def exclusive_write(path, data, mode):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    with os.fdopen(fd, 'w') as out:
        out.write(data); out.flush(); os.fsync(out.fileno())


def systemctl(*args):
    return subprocess.run(['/usr/bin/systemctl', *args], check=True, timeout=55)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--share', required=True, help='//server/share (no password)')
    parser.add_argument('--remove', action='store_true', help='Remove this managed mount, not its files')
    parser.add_argument('--plan', action='store_true', help='Print the plan without making changes')
    args = parser.parse_args()
    uid = int(os.environ.get('SUDO_UID', os.getuid()))
    if uid <= 0:
        raise ValueError('Run with sudo from your normal desktop account, not a root login.')
    user = pwd.getpwuid(uid)
    plan = mount_plan(args.share, uid, user.pw_gid)
    print(f"Network share: {plan['share']}\nLinux path:    {plan['mountpoint']}\nDesktop user:  {user.pw_name}\n")
    if args.plan:
        print(plan['mountUnit'] + '\n' + plan['automountUnit']); return 0
    if os.geteuid() != 0:
        raise ValueError('This operation requires sudo. The GUI itself must stay unprivileged.')
    if not sys.stdin.isatty():
        raise ValueError('Run in a terminal so you can review and confirm the change.')
    mountpoint = Path(plan['mountpoint'])
    mountfile = Path('/etc/systemd/system') / (plan['unit'] + '.mount')
    autofile = mountfile.with_suffix('.automount')
    credential = Path(plan['credentials'])
    secure_dir(mountfile.parent, 0o755)
    secure_dir(credential.parent, 0o700)
    secure_dir(Path('/mnt/winspace'), 0o755)
    if args.remove:
        print('First point Downloads/Documents elsewhere. Close files using this mount.')
        for file in (mountfile, autofile, credential):
            if file.is_symlink() or not file.is_file() or file.stat().st_uid != 0:
                raise ValueError('Missing or unsafe managed files; refusing automatic removal.')
        if mountfile.read_text() != plan['mountUnit'] or autofile.read_text() != plan['automountUnit']:
            raise ValueError('Unit files have been edited. Remove them manually after review.')
        if input('Type REMOVE to disconnect this mount and forget its credential: ') != 'REMOVE':
            print('Cancelled.'); return 1
        # Fail closed: if busy, retain all config and credentials for recovery.
        systemctl('stop', plan['unit'] + '.mount', plan['unit'] + '.automount')
        systemctl('disable', plan['unit'] + '.automount')
        for file in (mountfile, autofile, credential): file.unlink()
        systemctl('daemon-reload')
        try: mountpoint.rmdir()
        except OSError: pass  # Never recursively delete anything.
        print('Mount configuration removed. No shared or local files deleted.'); return 0
    if shutil.which('mount.cifs') is None:
        raise ValueError('Install cifs-utils first: sudo apt install cifs-utils')
    if any(p.exists() or p.is_symlink() for p in (mountfile, autofile, credential, mountpoint)):
        raise ValueError('This mount already exists or a path is in use. Nothing was overwritten.\nRemoval command: ' + plan['removeCommand'])
    print('Creates two systemd units and a root-only credentials file. Uses SMB 3.0;')
    print('there is no SMB1 fallback. Does not change fstab or move existing files.')
    print('The credential file contains your SMB password in plaintext, readable by root.')
    print('Share access is intended for this Linux user. Administrators can still access it.')
    if input('Type SETUP to continue: ') != 'SETUP':
        print('Cancelled.'); return 1
    username = input('SMB username (DOMAIN\\username is optional): ').strip()
    password = getpass.getpass('SMB password: ')
    if not username or any(c in username + password for c in '\r\n\x00'):
        raise ValueError('Invalid credentials. Newlines and NUL characters are not supported.')
    domain = ''
    if '\\' in username:
        domain, username = username.split('\\', 1)
        if not username: raise ValueError('Enter a username after the domain.')
    data = f'username={username}\npassword={password}\n' + (f'domain={domain}\n' if domain else '')
    created = []
    try:
        mountpoint.mkdir(mode=0o555)  # No silent local writes when not mounted.
        for file, content, mode in ((credential, data, 0o600), (mountfile, plan['mountUnit'], 0o644), (autofile, plan['automountUnit'], 0o644)):
            exclusive_write(file, content, mode); created.append(file)
        password = data = ''
        systemctl('daemon-reload')
        systemctl('enable', '--now', plan['unit'] + '.automount')
        # Validate authentication now, not at the first browser download.
        systemctl('start', plan['unit'] + '.mount')
    except Exception:
        print('Setup failed. Trying to stop and roll back the new mount configuration.', file=sys.stderr)
        try:
            systemctl('stop', plan['unit'] + '.mount', plan['unit'] + '.automount')
            subprocess.run(['/usr/bin/systemctl', 'disable', plan['unit'] + '.automount'], timeout=20, check=False)
        except Exception:
            print('Could not stop safely. Configuration retained for manual recovery: ' + plan['removeCommand'], file=sys.stderr)
            raise
        for file in reversed(created): file.unlink(missing_ok=True)
        try: mountpoint.rmdir()
        except OSError: pass
        systemctl('daemon-reload')
        raise
    print(f"\nMounted at {plan['mountpoint']}\nIn OpenXplorer: Downloads → Properties → Location → enter this path → Check → Apply.")
    print('The server must be online for new downloads. Browser-specific download settings may also need updating.')
    print('Removal (after restoring folder locations): ' + plan['removeCommand'])
    return 0


if __name__ == '__main__':
    try: sys.exit(main())
    except (ValueError, OSError, subprocess.SubprocessError) as exc:
        print('Mount setup: ' + str(exc), file=sys.stderr); sys.exit(1)
