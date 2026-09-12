# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""XDG known-folder locations. No shell evaluation, symlinks, or data moves.

Network locations must resolve to an existing kernel CIFS/SMB3 mount.
A GVfs session path or an smb:// URI is not a persistent XDG directory.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import time
from urllib.parse import unquote, urlsplit
from core import normalise_location, CONTROL
from mount_support import read_mounts, resolve_smb_path, mount_for_path

FOLDERS = {
    'DESKTOP': ('Desktop', 'desktop'), 'DOWNLOAD': ('Downloads', 'downloads'),
    'DOCUMENTS': ('Documents', 'documents'), 'PICTURES': ('Pictures', 'pictures'),
    'MUSIC': ('Music', 'music'), 'VIDEOS': ('Videos', 'videos'),
    'TEMPLATES': ('Templates', 'documents'), 'PUBLICSHARE': ('Public', 'folder'),
}
LINE = re.compile(r'^\s*XDG_([A-Z]+)_DIR\s*=\s*"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$')


def read_user_dirs(path: Path, home: Path) -> dict[str, str]:
    """Parse the documented double-quoted syntax, NEVER source a shell file."""
    result = {}
    try:
        if path.stat().st_size > 128 * 1024:
            raise ValueError('The user-dirs configuration is unexpectedly large.')
        text = path.read_text(encoding='utf-8')
    except FileNotFoundError:
        return result
    for line in text.splitlines():
        match = LINE.match(line)
        if not match or match[1] not in FOLDERS:
            continue
        raw = match[2]
        if raw == '$HOME' or raw.startswith('$HOME/'):
            raw = str(home) + raw[5:]
        elif raw == '${HOME}' or raw.startswith('${HOME}/'):
            raw = str(home) + raw[7:]
        # Only shell's quoted-string escapes are interpreted. No command or
        # variable substitution. Literal escaped $ and ` remain literal paths.
        chars, i = [], 0
        valid = raw.startswith('/')
        while i < len(raw):
            char = raw[i]
            if char == '\\' and i + 1 < len(raw):
                if raw[i + 1] in '\\"$`':
                    chars.append(raw[i + 1]); i += 2; continue
                chars.append('\\'); i += 1; continue
            if char in '$`' or ord(char) < 32:
                valid = False
            chars.append(char); i += 1
        decoded = ''.join(chars)
        if valid and not CONTROL.search(decoded):
            result[match[1]] = os.path.normpath(decoded)
    return result


class FolderLocations:
    def __init__(self, directory: Path, *, home: Path | None = None,
                 config: Path | None = None, run=None, mounts=None):
        self.directory = directory
        self.home = home or Path.home()
        self.config = config or Path(os.environ.get('XDG_CONFIG_HOME', self.home / '.config'))
        self.path = self.config / 'user-dirs.dirs'
        self.history = directory / 'folder-location-history.json'
        self.run = run or self._run
        self.mounts = mounts or read_mounts
        self.lock = threading.RLock()

    @staticmethod
    def _run(args):
        try:
            result = subprocess.run(args, text=True, capture_output=True, timeout=15, check=True)
        except FileNotFoundError as exc:
            raise ValueError('Install xdg-user-dirs before changing a standard folder.') from exc
        except subprocess.CalledProcessError as exc:
            raise ValueError((exc.stderr or 'The XDG folder setting could not be changed.').strip()) from exc
        return result.stdout.strip()

    def paths(self):
        # Reading the config avoids GLib's process-lifetime special-directory
        # cache, and immediately reflects changes from other applications.
        configured = read_user_dirs(self.path, self.home)
        return {key: configured.get(key, str(self.home / label))
                for key, (label, _) in FOLDERS.items()}

    def _history(self):
        try:
            data = json.loads(self.history.read_text())
            return data if isinstance(data, dict) else {}
        except (OSError, ValueError):
            return {}

    def snapshot(self):
        history = self._history()
        return [{'key': key, 'label': FOLDERS[key][0], 'icon': FOLDERS[key][1],
                 'path': path, 'uri': Path(path).as_uri(), 'defaultPath': str(self.home / FOLDERS[key][0]),
                 'previousPath': history.get(key, {}).get('previous')}
                for key, path in self.paths().items()]

    def validate(self, key: str, value: str):
        if key not in FOLDERS:
            raise ValueError('Choose a standard folder, such as Downloads or Documents.')
        uri = normalise_location(value, home=self.home)
        mounts = self.mounts()
        if uri.startswith('smb:'):
            path = resolve_smb_path(uri, mounts)
            if path is None:
                raise ValueError('This SMB folder is not mounted at a stable Linux path. Use “Set up network mount”, or mount it with CIFS first. A sidebar bookmark alone is not enough.')
        else:
            path = unquote(urlsplit(uri).path)
        target = Path(path)
        # realpath detects an existing symlink pointing into a temporary GVfs
        # session, instead of accepting it as apparently persistent.
        real = target.resolve(strict=True)
        if any(str(real) == base or str(real).startswith(base + '/') for base in ('/run', '/tmp', '/var/tmp')):
            raise ValueError('Use a persistent location, not a temporary or per-login GVfs path.')
        mount = mount_for_path(str(real), mounts)
        if mount and 'gvfs' in mount['fstype']:
            raise ValueError('GVfs session paths cannot be used as persistent standard folders.')
        if not real.is_dir():
            raise ValueError('The new location must be an existing folder.')
        if real == self.home.resolve() or real == Path('/'):
            raise ValueError('Choose a dedicated folder, not your entire home directory or the filesystem root.')
        if not os.access(real, os.W_OK | os.X_OK):
            raise ValueError('You do not have write access to this folder.')
        return {'key': key, 'path': str(real), 'uri': real.as_uri(),
                'network': bool(mount and mount['fstype'] in ('cifs', 'smb3')),
                'source': mount.get('source') if mount else None,
                'previous': self.paths()[key]}

    def apply(self, key, value, *, confirmed=False):
        if confirmed is not True:
            raise ValueError('Confirm the new location before applying it.')
        with self.lock:
            result = self.validate(key, value)
            if result['path'] == result['previous']:
                return {**result, 'changed': False}
            self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
            backups = self.directory / 'location-backups'
            backups.mkdir(exist_ok=True, mode=0o700)
            fd, backup = tempfile.mkstemp(prefix='user-dirs-', suffix='.dirs', dir=backups)
            with os.fdopen(fd, 'wb') as out:
                os.fchmod(out.fileno(), 0o600)
                out.write(self.path.read_bytes() if self.path.exists() else b'')
                out.flush(); os.fsync(out.fileno())
            self.run(['xdg-user-dirs-update', '--set', key, result['path']])
            # The configured result is re-read, not assumed from exit status.
            if self.paths()[key] != result['path']:
                raise ValueError('The folder configuration did not retain the requested path. A backup was saved; no user files were moved.')
            history = self._history()
            history[key] = {'previous': result['previous'], 'path': result['path'],
                            'backup': str(backup), 'changedAt': time.time()}
            fd, temporary = tempfile.mkstemp(prefix='.locations-', dir=self.directory)
            try:
                with os.fdopen(fd, 'w') as out:
                    os.fchmod(out.fileno(), 0o600)
                    json.dump(history, out, indent=2); out.flush(); os.fsync(out.fileno())
                os.replace(temporary, self.history)
            finally:
                if os.path.exists(temporary): os.unlink(temporary)
            return {**result, 'changed': True, 'backup': str(backup), 'filesMoved': False}
