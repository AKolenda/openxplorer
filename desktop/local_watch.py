# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Unprivileged Linux inotify. One watch per directory, bounded, no polling.

Watch exhaustion and queue overflow are reported to the caller, never silently
advertised as complete live coverage. No filesystem contents are read here.
"""
from __future__ import annotations
import ctypes
import ctypes.util
import os
import select
import struct
import threading
from urllib.parse import unquote, quote, urlsplit

MODIFY=0x2; ATTRIB=0x4; CLOSE_WRITE=0x8; MOVED_FROM=0x40; MOVED_TO=0x80
CREATE=0x100; DELETE=0x200; DELETE_SELF=0x400; MOVE_SELF=0x800
OVERFLOW=0x4000; IGNORED=0x8000; ISDIR=0x40000000
MASK=MODIFY|ATTRIB|CLOSE_WRITE|MOVED_FROM|MOVED_TO|CREATE|DELETE|DELETE_SELF|MOVE_SELF


class LocalWatch:
    def __init__(self, changed, overflow, max_watches=8192):
        self.changed, self.overflow = changed, overflow
        self.max_watches = max_watches
        self.lock = threading.RLock()
        self.watches, self.paths = {}, {}
        self.closed = False
        self.lib = ctypes.CDLL(ctypes.util.find_library('c') or 'libc.so.6', use_errno=True)
        self.lib.inotify_init1.argtypes = [ctypes.c_int]; self.lib.inotify_init1.restype = ctypes.c_int
        self.lib.inotify_add_watch.argtypes = [ctypes.c_int,ctypes.c_char_p,ctypes.c_uint32]; self.lib.inotify_add_watch.restype = ctypes.c_int
        self.lib.inotify_rm_watch.argtypes = [ctypes.c_int,ctypes.c_int]; self.lib.inotify_rm_watch.restype = ctypes.c_int
        self.fd = self.lib.inotify_init1(os.O_NONBLOCK|os.O_CLOEXEC)
        if self.fd < 0: raise OSError(ctypes.get_errno(), 'inotify is unavailable')
        self.thread = threading.Thread(target=self._loop,name='winspace-inotify',daemon=True)
        self.thread.start()

    def add(self, root, uri):
        path = unquote(urlsplit(uri).path)
        with self.lock:
            if path in self.paths:
                self.watches[self.paths[path]][1].add((root,uri)); return
            if len(self.watches) >= self.max_watches:
                raise OSError('Live watch limit reached (8192 directories). Unwatched directories use timed checks.')
            wd = self.lib.inotify_add_watch(self.fd, os.fsencode(path), MASK | 0x01000000 | 0x02000000) # ONLYDIR, DONT_FOLLOW
            if wd < 0: raise OSError(ctypes.get_errno(), 'Could not watch a directory')
            self.paths[path] = wd
            # inotify can return the same wd for alias/bind paths.
            if wd in self.watches: self.watches[wd][1].add((root,uri))
            else: self.watches[wd] = (path,{(root,uri)})

    def count(self, root):
        with self.lock: return sum(any(r==root for r,u in refs) for p,refs in self.watches.values())

    def remove(self, root, prefix=None):
        with self.lock:
            for wd,(path,refs) in list(self.watches.items()):
                refs.difference_update({(r,u) for r,u in refs if r==root and (not prefix or u==prefix or u.startswith(prefix.rstrip('/')+'/'))})
                if not refs:
                    self.watches.pop(wd,None)
                    for p,v in list(self.paths.items()):
                        if v==wd: self.paths.pop(p,None)
                    self.lib.inotify_rm_watch(self.fd,wd)

    def _loop(self):
        while not self.closed:
            try:
                if not select.select([self.fd],[],[],0.2)[0]: continue
                payload = os.read(self.fd,262144)
            except (OSError,ValueError): continue
            at = 0
            while at+16 <= len(payload):
                wd,mask,cookie,length = struct.unpack_from('iIII',payload,at)
                name = payload[at+16:at+16+length].split(b'\0',1)[0]
                at += 16+length
                if mask & OVERFLOW:
                    self.overflow(); continue
                with self.lock:
                    pair = self.watches.get(wd)
                    refs = list(pair[1]) if pair else []
                    if mask & IGNORED:
                        self.watches.pop(wd,None)
                        for p,v in list(self.paths.items()):
                            if v == wd: self.paths.pop(p,None)
                for root,uri in refs:
                    if mask & (DELETE|MOVED_FROM) and mask & ISDIR and name:
                        self.remove(root,uri.rstrip('/')+'/'+quote(os.fsdecode(name),safe=''))
                    if mask & (DELETE_SELF|MOVE_SELF):
                        self.changed(root,uri.rsplit('/',1)[0] or root)
                    elif not mask & IGNORED:
                        self.changed(root,uri)

    def close(self):
        self.closed = True
        self.thread.join(timeout=.6)
        os.close(self.fd)
