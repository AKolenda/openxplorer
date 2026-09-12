# SPDX-License-Identifier: AGPL-3.0-only
"""Private application state: reject symlink/non-regular/hardlinked leaves.

This is defense in depth for misplaced or tampered XDG state, not isolation
from another process running as the SAME desktop user. XDG ancestors may be
user-managed; the application's own directory must be an owned real directory.
"""
from __future__ import annotations
import os
from pathlib import Path
import stat


def private_directory(path: Path) -> None:
    path = Path(path)
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        info = os.fstat(fd)
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid():
            raise ValueError('Application state directory must be owned by this user.')
        os.fchmod(fd, 0o700)
    finally:
        os.close(fd)


def private_file(path: Path, *, create: bool = False, writable: bool = False, allow_unlinked: bool = False) -> int:
    """Caller owns returned fd. Validate BEFORE fchmod; never block on a FIFO."""
    flags = (os.O_RDWR if writable else os.O_RDONLY) | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    if create:
        flags |= os.O_CREAT
    fd = os.open(path, flags, 0o600)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_nlink > 1 or (info.st_nlink == 0 and not allow_unlinked):
            raise ValueError('Application state must be an owned regular file, not a link or device.')
        if info.st_nlink:
            os.fchmod(fd, 0o600)
        return fd
    except BaseException:
        os.close(fd)
        raise


def private_text(path: Path, limit: int = 4 * 1024 * 1024) -> str:
    fd = private_file(path)
    with os.fdopen(fd, 'rb') as stream:
        if os.fstat(stream.fileno()).st_size > limit:
            raise ValueError('Settings file exceeds the 4 MiB safety limit.')
        data = stream.read(limit + 1)
        if len(data) > limit:
            raise ValueError('Settings file exceeds the 4 MiB safety limit.')
        return data.decode('utf-8')


def validate_sqlite_files(path: Path) -> None:
    # SQLite creates/unlinks sidecars itself. An opened sidecar can legitimately
    # have nlink=0 after another connection checkpoints it; do not chmod that fd.
    # Refuse redirected/special/hardlinked files that still have directory entries.
    for candidate in (path, Path(str(path)+'-wal'), Path(str(path)+'-shm'), Path(str(path)+'-journal')):
        try:
            fd = private_file(candidate, writable=True, allow_unlinked=candidate != path)
        except FileNotFoundError:
            if candidate == path:
                raise
            continue
        os.close(fd)
