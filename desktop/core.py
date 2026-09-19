# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Pure-Python validation and settings. No shell commands or stored credentials."""
from __future__ import annotations
import json
from private_storage import private_directory, private_file, private_text
import fcntl
from functools import wraps
import os
from pathlib import Path
import posixpath
import re
import tempfile
import threading
from urllib.parse import quote, unquote, SplitResult, urlsplit, urlunsplit

VERSION = '1.1.1'
DEBIAN_VERSION = '1.1.1'
CONTROL = re.compile(r'[\x00-\x1f\x7f]')
DEVICE_SCHEMES = frozenset({'mtp', 'gphoto2', 'afc'})
DEVICE_URI = re.compile(r'^([A-Za-z][A-Za-z0-9+.-]*)://([^/?#]+)(/[^?#]*)?$')


def split_location(value: str) -> SplitResult:
    """Split a validated location, including GVfs's non-RFC USB authorities.

    MTP and gphoto2 roots look like ``mtp://[usb:001,002]/``. Python's URL
    parser treats the bracketed bus identifier as malformed IPv6, although it
    is the URI format produced and consumed by GIO. Keep this narrow parser for
    the three portable-device backends; ordinary file/SMB parsing stays on the
    standard library implementation.
    """
    match = DEVICE_URI.fullmatch(value) if isinstance(value, str) else None
    if match and match.group(1).lower() in DEVICE_SCHEMES:
        return SplitResult(match.group(1).lower(), match.group(2), match.group(3) or '/', '', '')
    return urlsplit(value)


def is_device_location(value: str) -> bool:
    try:
        return split_location(value).scheme.lower() in DEVICE_SCHEMES
    except (TypeError, ValueError):
        return False


def _normalise_device_location(value: str, scheme: str) -> str:
    match = DEVICE_URI.fullmatch(value)
    if not match or match.group(1).lower() != scheme:
        raise ValueError('A connected-device address must include a device identifier and path.')
    authority, encoded_path = match.group(2), match.group(3) or '/'
    if (len(authority) > 512 or '@' in authority or '%' in authority
            or any(c.isspace() for c in authority) or CONTROL.search(authority)):
        raise ValueError('Invalid connected-device identifier.')
    if ('[' in authority or ']' in authority) and not (
            authority.startswith('[') and authority.endswith(']')
            and '[' not in authority[1:-1] and ']' not in authority[1:-1]):
        raise ValueError('Invalid connected-device identifier.')
    decoded = unquote(encoded_path, errors='strict')
    if CONTROL.search(decoded):
        raise ValueError('Encoded control characters are not allowed.')
    path = posixpath.normpath('/' + decoded.lstrip('/'))
    return f'{scheme}://{authority}' + quote(path, safe='/')


def validate_name(name: str) -> str:
    if not isinstance(name, str) or not name or name in ('.', '..'):
        raise ValueError('Enter a non-empty file name, not “.” or “..”.')
    if '/' in name or '\\' in name or CONTROL.search(name):
        raise ValueError('A name cannot contain slashes or control characters.')
    if len(os.fsencode(name)) > 255:
        raise ValueError('This name is longer than 255 bytes.')
    return name


def normalise_location(value: str, base: str | None = None, home: Path | None = None) -> str:
    """Accept Linux paths, SMB URLs, UNC paths and connected-device URIs.

    URL path components are canonicalised once, preserving escaped #, ? and %.
    Local paths (not URLs) may contain these characters literally. We never run
    a shell, expand environment variables or infer a Windows C: mapping.
    """
    if not isinstance(value, str) or not value.strip():
        raise ValueError('Enter a local folder path or an SMB address.')
    value = value.strip()
    if CONTROL.search(value):
        raise ValueError('Control characters are not allowed in an address.')
    home = home or Path.home()
    if value.startswith('\\\\') or value.startswith('//'):
        parts = value.replace('\\', '/').lstrip('/').split('/')
        server, *parts = parts
        if not server or '@' in server or ':' in server or CONTROL.search(server):
            raise ValueError('Use a server name without credentials, for example \\\\nas\\share.')
        value = 'smb://' + server + '/' + '/'.join(quote(p, safe='') for p in parts)
    if re.match(r'^[A-Za-z]:[\\/]', value):
        raise ValueError('Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share.')
    if not re.match(r'^[A-Za-z][A-Za-z0-9+.-]*:', value):
        if value == '~':
            value = str(home)
        elif value.startswith('~/'):
            value = str(home / value[2:])
        if not value.startswith('/'):
            base_scheme = ''
            if base:
                try:
                    base_scheme = split_location(base).scheme.lower()
                except (TypeError, ValueError):
                    pass
            if base and base_scheme in ({'smb'} | DEVICE_SCHEMES):
                return normalise_location(base.rstrip('/') + '/' + quote(value, safe='/'), home=home)
            base_path = str(home)
            if base and base.startswith('file:'):
                base_path = unquote(urlsplit(base).path)
            value = os.path.join(base_path, value)
        value = os.path.abspath(os.path.normpath(value))
        return Path(value).as_uri()
    scheme_match = re.match(r'^([A-Za-z][A-Za-z0-9+.-]*):', value)
    scheme = scheme_match.group(1).lower() if scheme_match else ''
    if scheme in DEVICE_SCHEMES:
        return _normalise_device_location(value, scheme)
    u = urlsplit(value)
    if u.scheme.lower() not in ('file', 'smb'):
        raise ValueError('Only local paths, smb:// locations and connected devices are supported in this build.')
    if u.username is not None or u.password is not None:
        raise ValueError('Do not put a username or password in the address. Use the OpenXplorer sign-in dialog.')
    if u.query or u.fragment:
        raise ValueError('In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path.')
    decoded = unquote(u.path, errors='strict')
    if CONTROL.search(decoded):
        raise ValueError('Encoded control characters are not allowed.')
    if u.scheme.lower() == 'file':
        if u.netloc and u.netloc.lower() != 'localhost':
            raise ValueError('For network folders, use smb://server/share rather than file://server/…')
        if not decoded.startswith('/'):
            raise ValueError('A file URL must contain an absolute path.')
        return Path(os.path.normpath(decoded)).as_uri()
    if '%' in u.netloc or CONTROL.search(u.netloc):
        raise ValueError('Use an unescaped server name without credentials or control characters.')
    if not u.hostname or any(c.isspace() for c in u.hostname):
        raise ValueError('Enter an SMB server name, for example smb://nas/Projects.')
    try:
        port = u.port
    except ValueError as exc:
        raise ValueError('Invalid SMB port.') from exc
    host = u.hostname.lower()
    if ':' in host:
        host = '[' + host + ']'
    if port is not None:
        host += ':' + str(port)
    # SMB uses / in a URI; literal backslashes are treated as path separators.
    path = posixpath.normpath('/' + decoded.replace('\\', '/').lstrip('/'))
    return urlunsplit(('smb', host, quote(path, safe='/'), '', ''))


def require_share(value: str) -> str:
    uri = normalise_location(value)
    u = split_location(uri)
    if u.scheme != 'smb' or not u.path.strip('/'):
        raise ValueError('Enter a shared folder such as \\\\nas\\Projects, not only the server name.')
    return uri


def new_copy_name(name: str, number: int, is_directory: bool) -> str:
    validate_name(name)
    if is_directory or '.' not in name.lstrip('.'):
        stem, suffix = name, ''
    else:
        stem, suffix = name.rsplit('.', 1)
        suffix = '.' + suffix
    marker = f' (copy {number})'
    # Respect common NAME_MAX without corrupting UTF-8. Long extensions are
    # rejected rather than silently renamed beyond recognition.
    while len((stem + marker + suffix).encode()) > 255 and stem:
        stem = stem[:-1]
    if not stem:
        raise ValueError('This file name is too long to generate a duplicate name.')
    return stem + marker + suffix


def safe_label(value: str, fallback: str) -> str:
    if not isinstance(value, str) or not value.strip():
        return fallback
    value = value.strip()
    if CONTROL.search(value) or len(value) > 120:
        raise ValueError('A sidebar label must be at most 120 characters and contain no control characters.')
    return value


def settings_mutation(function):
    """Serialize read/modify/write across separately opened application windows."""
    @wraps(function)
    def wrapped(self, *args, **kwargs):
        if function.__name__ == 'update_preferences' and kwargs.get('save') is False:
            return function(self, *args, **kwargs)
        with self.lock:
            private_directory(self.directory)
            fd = private_file(self.directory/'settings.lock', create=True, writable=True)
            with os.fdopen(fd, 'r+b') as lockfile:
                fcntl.flock(lockfile.fileno(), fcntl.LOCK_EX)
                self.reload()
                self._mutating = True
                try:
                    return function(self, *args, **kwargs)
                finally:
                    self._mutating = False
    return wrapped


class Settings:
    """Atomically written, private JSON. A whitelist excludes credentials."""
    def __init__(self, directory: Path | None = None):
        self.directory = directory or Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config')) / 'winspace'
        self.path = self.directory / 'settings.json'
        self.lock = threading.RLock()
        self.data = {'version': 2, 'pins': [], 'shares': [], 'hiddenQuick': [], 'quickOrder': [], 'recent': [],
                     'preferences': {'theme': 'system', 'view': 'details', 'details': True, 'showHidden': False, 'autoIndex': True, 'contextMenu':'win10', 'networkInterval':60, 'textSize':100}}
        self.warning = ''
        self._mutating = False
        try:
            if self.directory.exists() or self.directory.is_symlink():
                private_directory(self.directory)
            source = json.loads(private_text(self.path))
            if not isinstance(source, dict):
                raise ValueError('Settings must be a JSON object.')
            for kind in ('pins', 'shares'):
                for item in source.get(kind, [])[:200]:
                    try:
                        uri = normalise_location(item['uri'])
                        if kind == 'shares':
                            uri = require_share(uri)
                        label = safe_label(item.get('label', ''), unquote(split_location(uri).path).split('/')[-1] or 'Folder')
                        self.data[kind].append({'uri': uri, 'label': label})
                    except (ValueError, KeyError, TypeError):
                        continue
            for item in source.get('recent', [])[:30]:
                try:
                    self.data['recent'].append({'uri': normalise_location(item['uri']), 'name': str(item['name'])[:512],
                                               'type': str(item.get('type', 'File'))[:200], 'isDir': False,
                                               'size': max(0, int(item.get('size') or 0)),
                                               'modified': max(0, int(item.get('modified') or 0))})
                except (ValueError, KeyError, TypeError):
                    continue
            for uri in source.get('hiddenQuick', [])[:200]:
                try:
                    self.data['hiddenQuick'].append(normalise_location(uri))
                except (ValueError, TypeError):
                    pass
            for uri in source.get('quickOrder', [])[:400]:
                try:
                    uri = normalise_location(uri)
                    if uri not in self.data['quickOrder']:
                        self.data['quickOrder'].append(uri)
                except (ValueError, TypeError):
                    continue
            self.update_preferences(source.get('preferences', {}), save=False)
        except FileNotFoundError:
            pass
        except (ValueError, OSError, TypeError) as exc:
            self.warning = 'Could not fully read settings; using safe defaults. ' + str(exc)

    def reload(self):
        # A new instance uses the same validated whitelist. Atomic replace means
        # readers see either a complete old or complete new configuration.
        if self.path.exists():
            other = Settings(self.directory)
            self.data, self.warning = other.data, other.warning

    def save(self) -> None:
        with self.lock:
            private_directory(self.directory)
            try:
                checked = private_file(self.path)
            except FileNotFoundError:
                pass
            else:
                os.close(checked)
            fd, name = tempfile.mkstemp(prefix='.settings-', dir=self.directory)
            try:
                with os.fdopen(fd, 'w', encoding='utf-8') as stream:
                    os.fchmod(stream.fileno(), 0o600)
                    json.dump(self.data, stream, ensure_ascii=False, indent=2)
                    stream.write('\n')
                    stream.flush()
                    os.fsync(stream.fileno())
                os.replace(name, self.path)
            finally:
                try:
                    os.unlink(name)
                except FileNotFoundError:
                    pass

    def snapshot(self) -> dict:
        with self.lock:
            if not self._mutating: self.reload()
            return json.loads(json.dumps(self.data))

    @settings_mutation
    def update_preferences(self, values: dict, save: bool = True) -> dict:
        if not isinstance(values, dict):
            raise ValueError('Preferences must be an object.')
        with self.lock:
            prefs = self.data['preferences']
            for key in ('details', 'showHidden', 'autoIndex'):
                if key in values and isinstance(values[key], bool):
                    prefs[key] = values[key]
            size = values.get('textSize')
            if type(size) is int and size in (80, 90, 100, 110, 125, 150, 175, 200):
                prefs['textSize'] = size
            # Persist only bounded layout values; never arbitrary CSS or paths.
            width = values.get('sidebarWidth')
            if isinstance(width, (int, float)) and not isinstance(width, bool) and 140 <= width <= 560:
                prefs['sidebarWidth'] = round(width)
            columns = values.get('columnWidths')
            if isinstance(columns, dict):
                limits = {'name': (140, 1600), 'modified': (100, 1000),
                          'parentUri': (140, 1600), 'type': (80, 1000), 'size': (70, 600)}
                cleaned = {}
                for key, (low, high) in limits.items():
                    value = columns.get(key)
                    if isinstance(value, (int, float)) and not isinstance(value, bool) and low <= value <= high:
                        cleaned[key] = round(value)
                prefs['columnWidths'] = cleaned
            if values.get('contextMenu') in ('win10','win11'):
                prefs['contextMenu']=values['contextMenu']
            if values.get('networkInterval') in (30,60,300):
                prefs['networkInterval']=values['networkInterval']
            if values.get('theme') in ('light', 'dark', 'system'):
                prefs['theme'] = values['theme']
            if values.get('view') in ('details', 'grid'):
                prefs['view'] = values['view']
            if save:
                self.save()
            return dict(prefs)

    @settings_mutation
    def bookmark(self, action: str, kind: str, uri: str, label: str = '') -> None:
        uri = require_share(uri) if kind == 'share' else normalise_location(uri)
        if kind not in ('share', 'pin') or action not in ('add', 'remove'):
            raise ValueError('Invalid bookmark action.')
        with self.lock:
            key = 'shares' if kind == 'share' else 'pins'
            items = self.data[key]
            self.data[key] = [p for p in items if p['uri'] != uri]
            if action == 'add':
                self.data[key].append({'uri': uri, 'label': safe_label(label, unquote(split_location(uri).path).split('/')[-1] or 'Folder')})
                if kind == 'pin':
                    self.data['hiddenQuick'] = [u for u in self.data['hiddenQuick'] if u != uri]
            elif kind == 'pin' and uri not in self.data['hiddenQuick']:
                self.data['hiddenQuick'].append(uri)
            if action == 'remove' and kind == 'pin':
                self.data['quickOrder'] = [u for u in self.data['quickOrder'] if u != uri]
            self.save()

    @settings_mutation
    def pin_many(self, items: list[dict], *, before: str | None = None,
                 quick_order: list[str] | None = None) -> list[dict]:
        """Add/reorder shortcut records only. This method performs no file I/O
        other than saving settings. Validate real folders in the GIO worker.
        Validate the entire batch before mutating or saving any settings.
        """
        if not isinstance(items, list) or not 1 <= len(items) <= 200:
            raise ValueError('Drag between 1 and 200 folders at a time.')
        clean, seen = [], set()
        for item in items:
            if not isinstance(item, dict):
                raise ValueError('Invalid folder shortcut.')
            uri = normalise_location(item.get('uri'))
            parts = split_location(uri)
            host = parts.hostname if parts.scheme == 'smb' else None
            label = safe_label(item.get('label', ''), unquote(parts.path).rstrip('/').split('/')[-1] or host or parts.netloc or 'Folder')
            if uri not in seen:
                clean.append({'uri': uri, 'label': label})
                seen.add(uri)
        if before is not None:
            before = normalise_location(before)
        if quick_order is not None and (not isinstance(quick_order, list) or len(quick_order) > 400):
            raise ValueError('Invalid sidebar order.')
        order = list(dict.fromkeys(normalise_location(u) for u in (quick_order or [])))
        with self.lock:
            old = self.snapshot()
            existing = {p['uri']: p for p in self.data['pins']}
            existing.update({p['uri']: p for p in clean})
            if len(existing) > 200:
                raise ValueError('Quick access supports up to 200 custom pins.')
            if not order:
                order = list(dict.fromkeys(self.data['quickOrder'] + list(existing)))
            # A drop on the dragged entry itself preserves its place.
            if before not in seen:
                order = [u for u in order if u not in seen]
                index = order.index(before) if before in order else len(order)
                order[index:index] = [p['uri'] for p in clean]
            else:
                order += [p['uri'] for p in clean if p['uri'] not in order]
            self.data['pins'] = list(existing.values())
            self.data['quickOrder'] = order
            self.data['hiddenQuick'] = [u for u in self.data['hiddenQuick'] if u not in seen]
            try:
                self.save()
            except OSError:
                self.data = old
                raise
            return clean

    @settings_mutation
    def remember_open(self, entry: dict) -> None:
        with self.lock:
            clean = {k: entry.get(k) for k in ('uri', 'name', 'size', 'modified', 'type', 'isDir')}
            clean['uri'] = normalise_location(clean['uri'])
            self.data['recent'] = [clean] + [e for e in self.data['recent'] if e['uri'] != clean['uri']][:29]
            self.save()


def is_smb_server(uri: str) -> bool:
    """A server listing holds shares, not files users can create/delete."""
    u = split_location(normalise_location(uri))
    return u.scheme == 'smb' and not u.path.strip('/')


def require_item_uri(uri: str) -> str:
    """Do not rename, move, trash, or transfer a whole SMB server/share root."""
    uri = normalise_location(uri)
    u = split_location(uri)
    if u.scheme == 'smb' and len([p for p in u.path.split('/') if p]) <= 1:
        raise ValueError('Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.')
    if u.scheme in DEVICE_SCHEMES and not u.path.strip('/'):
        raise ValueError('Open the device storage first, then select files or folders inside it. The device itself cannot be moved or copied.')
    return uri
