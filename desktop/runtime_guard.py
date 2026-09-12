# SPDX-License-Identifier: AGPL-3.0-only
"""Detect an old single-instance service after an in-place package upgrade.

No process-name matching, signals, root actions, shell commands or session files.
D-Bus requests are sent to the exact current unique owner, never to every process
named Python. A running file operation may refuse the existing quit action.
"""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import time

APP_ID = 'io.winspace.Development'
OBJECT_PATH = '/io/winspace/Development'
PROTOCOL = 1


def identity(root: Path, version: str) -> dict:
    digest = hashlib.sha256()
    inputs = sorted(root.glob('*.py')) + sorted((root/'ui').glob('*'))
    for path in inputs:
        if path.is_file() and path.suffix in ('.py', '.html', '.js', '.css', '.svg'):
            digest.update(path.relative_to(root).as_posix().encode() + b'\0')
            digest.update(path.read_bytes())
    return {'version': version, 'protocol': PROTOCOL, 'build': digest.hexdigest()}


def same_build(installed: dict, running: dict | None) -> bool:
    return bool(isinstance(running, dict) and all(running.get(k) == installed.get(k)
                                               for k in ('version', 'protocol', 'build')))


class Session:
    def __init__(self, Gio, GLib, connection=None):
        self.Gio, self.GLib = Gio, GLib
        self.connection = connection or Gio.bus_get_sync(Gio.BusType.SESSION, None)

    def call(self, dest, path, interface, method, signature, values):
        result = self.connection.call_sync(dest, path, interface, method,
            self.GLib.Variant(signature, values), None,
            self.Gio.DBusCallFlags.NO_AUTO_START, 3000, None)
        return result.unpack()

    def owner(self) -> str | None:
        try:
            return self.call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                             'org.freedesktop.DBus', 'GetNameOwner', '(s)', (APP_ID,))[0]
        except self.GLib.Error as exc:
            remote = self.Gio.DBusError.get_remote_error(exc)
            if remote == 'org.freedesktop.DBus.Error.NameHasNoOwner':
                return None
            raise

    def running(self, owner: str) -> dict | None:
        try:
            response = self.call(owner, OBJECT_PATH, 'org.gtk.Actions', 'Describe',
                                 '(s)', ('runtime-info',))
            # Describe -> (enabled, parameter signature, optional state array).
            description = response[0]
            value = description[2][0]
            if hasattr(value, 'unpack'): value = value.unpack()
            result = json.loads(value)
            if isinstance(result, dict) and isinstance(result.get('version'), str):
                return result
        except (self.GLib.Error, ValueError, TypeError, IndexError, KeyError):
            pass  # Older releases did not expose runtime identity.
        return None

    def stop(self, owner: str, timeout: float = 6, sleep=time.sleep,
             monotonic=time.monotonic) -> None:
        """Request safe quit and wait for its name to disappear; never force it."""
        try:
            self.call(owner, OBJECT_PATH, 'org.gtk.Actions', 'Activate', '(sava{sv})',
                      ('quit', [], {}))
        except self.GLib.Error:
            if self.owner() == owner: raise
        deadline = monotonic() + timeout
        while True:
            present = self.owner()
            if present is None: return
            if present != owner:
                raise RuntimeError('Another OpenXplorer process started during restart. No process was killed. Retry after closing it.')
            if monotonic() >= deadline:
                raise RuntimeError('OpenXplorer is still running. Finish or cancel active file operations, then run openxplorer --restart again. No process was killed.')
            sleep(.08)

    def status(self, installed: dict) -> dict:
        owner = self.owner()
        running = self.running(owner) if owner else None
        return {'installed': installed, 'owner': owner, 'running': running,
                'matches': same_build(installed, running) if owner else None,
                'legacyProcess': bool(owner and running is None)}


def require_current(session: Session, installed: dict, *, restart=False,
                    confirm=None) -> dict:
    """Return only when normal activation may safely continue.

    confirm receives the comparison and is deliberately supplied by the GUI,
    not hardwired into this testable module. Service mode never asks at login.
    """
    status = session.status(installed)
    owner = status['owner']
    if owner and (restart or not status['matches']):
        if not restart and (confirm is None or confirm(status) is not True):
            raise RuntimeError('A different or older OpenXplorer process is running. Finish file operations and run openxplorer --restart.')
        session.stop(owner)
    return status
