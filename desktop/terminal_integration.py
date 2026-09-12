# SPDX-License-Identifier: AGPL-3.0-only
"""Open a local terminal at a verified directory; never run filename-derived code.

The native host supplies fresh GIO metadata, a local/CIFS/GVfs-FUSE resolver and
its snapshot guard. No URI, executable, terminal arguments, script, environment
or command string is accepted from the web interface other than the target URI.
SMB is a LOCAL shell at an already-mounted path, not an SSH session on a NAS.
"""
from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import shutil
import stat
import subprocess
import threading
from typing import Callable
from urllib.parse import urlsplit, urlunsplit

from core import normalise_location, is_smb_server
from previous_versions import conventional_snapshot

# Never search the selected directory, a share, $TERMINAL or user-supplied PATH.
SYSTEM_PATH = '/usr/bin:/bin:/usr/local/bin'
TERMINALS = {
    'gnome-terminal': ('GNOME Terminal', '--working-directory='),
    'kgx': ('Console', '--working-directory='),
    'xfce4-terminal': ('Xfce Terminal', '--working-directory='),
    'konsole': ('Konsole', '--workdir='),
    'xterm': ('XTerm', None),
    'uxterm': ('UXTerm', None),
}


@dataclass(frozen=True)
class Terminal:
    executable: str
    kind: str

    @property
    def label(self) -> str:
        return TERMINALS[self.kind][0]


def find_terminal() -> Terminal:
    """Honor Debian's terminal alternative if its target has a known CLI.

    Otherwise prefer the native GNOME terminal on Zorin. Only system-installed
    allowlisted implementations are used; a custom terminal needs manual setup.
    """
    alternative = shutil.which('x-terminal-emulator', path=SYSTEM_PATH)
    if alternative:
        resolved = Path(alternative).resolve()
        # gnome-terminal.wrapper is the Debian alternative's option translator;
        # call its real CLI so --working-directory cannot be misinterpreted.
        name = resolved.name.removesuffix('.wrapper')
        if name in TERMINALS:
            executable = shutil.which(name, path=SYSTEM_PATH)
            if executable:
                return Terminal(executable, name)
    for name in TERMINALS:
        executable = shutil.which(name, path=SYSTEM_PATH)
        if executable:
            return Terminal(executable, name)
    raise ValueError('No supported terminal is installed. On Zorin, install GNOME Terminal with: sudo apt install gnome-terminal')


def checked_directory(value: str) -> str:
    """Resolve aliases before checking the target; no URL becomes a shell word."""
    if (not isinstance(value, str) or not value.startswith('/') or len(value) > 16384
            or any(ord(c) < 32 or ord(c) == 127 for c in value)):
        raise ValueError('The terminal requires a valid absolute local directory.')
    path = Path(value).resolve(strict=True)
    if not stat.S_ISDIR(path.stat().st_mode):
        raise ValueError('The terminal destination is not a directory.')
    if not os.access(path, os.X_OK):
        raise PermissionError('You do not have permission to enter this directory.')
    return str(path)


def prepare_directory(uri: str, inspect_entry: Callable, resolve_path: Callable,
                      assert_writable: Callable, cancel=None) -> dict:
    """Query reality, not a cached row's isDir label. Files use their parent."""
    uri = normalise_location(uri)
    if is_smb_server(uri):
        raise ValueError('Open a network share first. A server listing is not a terminal directory.')
    assert_writable(uri)
    if cancel:
        cancel.check()
    entry = inspect_entry(uri, cancel)
    if entry.get('symlink') or entry.get('kind') in ('symlink', 'special', 'unknown'):
        raise ValueError('Open the real folder first; links and special files are not terminal destinations.')
    if entry.get('isDir') is True:
        directory_uri = normalise_location(entry.get('targetUri') or uri)
    elif entry.get('kind') == 'file':
        parts = urlsplit(uri)
        parent = parts.path.rstrip('/').rsplit('/', 1)[0] or '/'
        directory_uri = normalise_location(urlunsplit((parts.scheme, parts.netloc, parent, '', '')))
    else:
        raise ValueError('Select a regular file or a directory.')
    if is_smb_server(directory_uri):
        raise ValueError('Open a network share before opening a terminal.')
    assert_writable(directory_uri)
    if conventional_snapshot(directory_uri):
        raise ValueError('Previous-version locations cannot be opened in Terminal. Restore a copy first.')
    local = resolve_path(directory_uri)
    if not local:
        raise ValueError('This SMB folder needs a local mount before Terminal can use it. Connect to the share and install gvfs-fuse, or use a persistent CIFS mount. This does not open an SSH session.')
    path = checked_directory(local)
    # Also check the resolved alias, including a symlink INTO a snapshot.
    local_uri = Path(path).as_uri()
    assert_writable(local_uri)
    if conventional_snapshot(local_uri):
        raise ValueError('Previous-version locations cannot be opened in Terminal. Restore a copy first.')
    if cancel:
        cancel.check()
    return {'uri': directory_uri, 'path': path, 'network': directory_uri.startswith('smb:')}


def terminal_argv(terminal: Terminal, directory: str) -> list[str]:
    """No -c/-e, eval, shell=True, chdir command, or quoted script string."""
    if terminal.kind not in TERMINALS or not os.path.isabs(terminal.executable):
        raise ValueError('Unsupported terminal executable.')
    directory = checked_directory(directory)
    option = TERMINALS[terminal.kind][1]
    return [terminal.executable] + ([option + directory] if option else [])


def launch_terminal(prepared: dict, terminal: Terminal | None = None) -> dict:
    terminal = terminal or find_terminal()
    directory = checked_directory(prepared['path'])
    argv = terminal_argv(terminal, directory)
    env = dict(os.environ, PWD=directory)
    # Treat this as a new terminal, not a child tab of any terminal used to start
    # OpenXplorer. Do not copy any SMB password into arguments or environment.
    for key in ('GNOME_TERMINAL_SCREEN', 'GNOME_TERMINAL_SERVICE'):
        env.pop(key, None)
    process = subprocess.Popen(argv, cwd=directory, env=env, shell=False,
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, close_fds=True,
                               start_new_session=True)
    try:
        code = process.wait(timeout=0.25)
        if code:
            raise RuntimeError(f'{terminal.label} could not start (exit {code}). Check your terminal installation and desktop session.')
    except subprocess.TimeoutExpired:
        # GUI emulators can stay alive for the shell lifetime. Reap after exit.
        threading.Thread(target=process.wait, daemon=True, name='openxplorer-terminal').start()
    return {'opened': True, 'terminal': terminal.label, 'path': directory,
            'uri': prepared['uri'], 'localShell': True}
