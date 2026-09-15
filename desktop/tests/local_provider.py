# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Test-only local provider. Never used by the shipped GTK application.

It exercises transfer orchestration against temporary real files without GI.
It cannot validate the production GIO/GVfs adapter or SMB behavior.
"""
from pathlib import Path
import ctypes
import errno
import os
import stat
from urllib.parse import urlsplit, unquote
from operations import Info, Cancelled

class Cancellation:
    def __init__(self): self.cancelled = False
    def cancel(self): self.cancelled = True
    def is_cancelled(self): return self.cancelled
    def check(self):
        if self.cancelled: raise Cancelled('Cancelled')

class LocalNode:
    def __init__(self, uri=None, path=None):
        self.p = Path(path) if path is not None else Path(unquote(urlsplit(uri).path))
        self.uri = self.p.as_uri()
        self.name = self.p.name
        self.path = str(self.p)
    def child(self, name): return type(self)(path=self.p / name)
    def parent(self): return type(self)(path=self.p.parent) if self.p != self.p.parent else None
    def exists(self, cancel=None):
        if cancel: cancel.check()
        return os.path.lexists(self.p)
    def info(self, cancel=None):
        if cancel: cancel.check()
        s = self.p.lstat()
        kind = 'directory' if stat.S_ISDIR(s.st_mode) else 'symlink' if stat.S_ISLNK(s.st_mode) else 'file' if stat.S_ISREG(s.st_mode) else 'special'
        return Info(kind, s.st_size)
    def is_directory(self, cancel=None):
        if cancel: cancel.check()
        return self.p.is_dir()
    def children(self, cancel=None):
        with os.scandir(self.p) as scan:
            for entry in scan:
                if cancel: cancel.check()
                yield type(self)(path=entry.path)
    def mkdir(self, cancel=None):
        if cancel: cancel.check()
        self.p.mkdir()
    def copy_file(self, target, cancel, progress):
        cancel.check()
        if self.p.is_symlink():
            os.symlink(os.readlink(self.p), target.p)
            return
        total = self.p.stat().st_size
        current = 0
        with self.p.open('rb') as src, target.p.open('xb') as dst:
            while True:
                cancel.check()
                data = src.read(8192)
                if not data: break
                dst.write(data)
                current += len(data)
                progress(current, total)
    def move_native(self, target, cancel=None):
        if cancel: cancel.check()
        libc = ctypes.CDLL(None, use_errno=True)
        rename = libc.renameat2
        rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        rename.restype = ctypes.c_int
        if rename(-100, os.fsencode(self.p), -100, os.fsencode(target.p), 1):
            e=ctypes.get_errno()
            raise OSError(e, os.strerror(e))
    def replace_native(self, target, cancel=None):
        if cancel: cancel.check()
        os.replace(self.p, target.p)
    def delete(self):
        if self.p.is_dir() and not self.p.is_symlink(): self.p.rmdir()
        else: self.p.unlink()
    def trash(self, cancel):
        cancel.check()
        raise NotImplementedError('Test provider deliberately does not support Trash; no delete fallback')
    def can_trash(self, cancel=None):
        return False
    def delete_tree(self, cancel):
        cancel.check()
        if self.p.is_dir() and not self.p.is_symlink():
            for child in self.children(cancel): child.delete_tree(cancel)
        self.delete()
