#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Run native checks with a private display, session bus and disposable user data."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

NATIVE = Path(__file__).resolve().parents[1]


def run(*args: str, **kwargs) -> subprocess.CompletedProcess:
    """Execute a check, propagating failures instead of silently skipping it."""
    print('+', ' '.join(args), flush=True)
    return subprocess.run(args, cwd=NATIVE, check=True, **kwargs)


def test_binaries() -> list[Path]:
    """Ask Cargo which unit/integration executables it actually compiled."""
    try:
        build = run('cargo', 'test', '--workspace', '--all-targets', '--locked', '--no-run',
                    '--message-format=json', stdout=subprocess.PIPE, text=True)
    except subprocess.CalledProcessError as error:
        for line in (error.stdout or '').splitlines():
            message = json.loads(line)
            if message.get('reason') == 'compiler-message':
                print(message['message'].get('rendered', message['message']['message']),
                      file=sys.stderr)
        raise
    binaries = set()
    for line in build.stdout.splitlines():
        message = json.loads(line)
        if (message.get('reason') == 'compiler-artifact'
                and message.get('profile', {}).get('test') and message.get('executable')):
            binaries.add(Path(message['executable']))
    if not binaries:
        raise RuntimeError('Cargo produced no test executables.')
    return sorted(binaries)


def isolated_environment(root: Path) -> dict[str, str]:
    """Keep GIO, GTK and Python compatibility tests away from the real session."""
    environment = os.environ.copy()
    # Cargo/rustup must still find the installed compiler when running doctests
    # with a disposable application HOME. These are build-tool directories.
    environment.setdefault('CARGO_HOME', str(Path.home() / '.cargo'))
    environment.setdefault('RUSTUP_HOME', str(Path.home() / '.rustup'))
    for name in ('DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS',
                 'DBUS_SESSION_BUS_PID', 'SESSION_MANAGER'):
        environment.pop(name, None)
    for name, directory in {
        'HOME': 'home', 'XDG_CONFIG_HOME': 'config', 'XDG_CACHE_HOME': 'cache',
        'XDG_DATA_HOME': 'data', 'XDG_STATE_HOME': 'state', 'XDG_RUNTIME_DIR': 'runtime',
    }.items():
        path = root / directory
        path.mkdir(mode=0o700)
        environment[name] = str(path)
    environment.update({
        'GDK_BACKEND': 'x11',
        'G_DEBUG': 'fatal-criticals',
        'GSK_RENDERER': 'cairo',
        'GSETTINGS_BACKEND': 'memory',
        'GVFS_DISABLE_FUSE': '1',
        'GVFS_REMOTE_VOLUME_MONITOR_IGNORE': '1',
    })
    return environment


def main() -> None:
    for executable in ('cargo', 'python3', 'dbus-run-session', 'xvfb-run', 'mkfifo'):
        if shutil.which(executable) is None:
            raise SystemExit(f'Required native check tool is missing: {executable}')
    run('python3', '-m', 'unittest', 'discover', '-s', 'parity', '-p', 'test_*.py')
    run('python3', 'parity/check.py')
    run('cargo', 'fmt', '--all', '--check')
    run('cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
    binaries = test_binaries()
    for binary in binaries:
        with tempfile.TemporaryDirectory(prefix='openxplorer-native-test-') as temporary:
            environment = isolated_environment(Path(temporary))
            run('xvfb-run', '-a', '-s', '-screen 0 1440x1000x24', 'dbus-run-session', '--',
                str(binary), '--test-threads=1', env=environment, timeout=180)
    with tempfile.TemporaryDirectory(prefix='openxplorer-native-doctest-') as temporary:
        environment = isolated_environment(Path(temporary))
        run('xvfb-run', '-a', 'dbus-run-session', '--', 'cargo', 'test',
            '--workspace', '--doc', '--locked', env=environment, timeout=180)
    print(f'Native checks passed ({len(binaries)} test executables plus doctests).')


if __name__ == '__main__':
    main()
