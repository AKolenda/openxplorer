# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""GIO stream and MIME launching, without round-tripping SMB files to OpenXplorer."""
from __future__ import annotations
from contextlib import contextmanager
import io
import os
from pathlib import Path
from urllib.parse import unquote,urlsplit
import gi
gi.require_version('Gio','2.0')
from gi.repository import Gio,GLib
from activation import choose_application,SELF_IDS
from core import normalise_location
from gio_backend import raw,inspect
from mount_support import resolve_smb_path,read_mounts


def local_path(uri):
    file=Gio.File.new_for_uri(normalise_location(uri))
    direct=file.get_path()
    if direct: return direct
    if not uri.startswith('smb:'): return None
    try:
        mounted=resolve_smb_path(uri,read_mounts())
        if mounted: return mounted
    except (ValueError,OSError): pass
    # GNOME's optional gvfs-fuse export is local to this logged-in user.
    u=urlsplit(uri); parts=unquote(u.path).strip('/').split('/')
    if not parts or not parts[0]: return None
    root=Path(GLib.get_user_runtime_dir())/'gvfs'
    try:
        for mount in root.iterdir():
            if not mount.name.startswith('smb-share:'): continue
            attrs=dict(x.split('=',1) for x in mount.name[len('smb-share:'):].split(',') if '=' in x)
            if unquote(attrs.get('server','')).lower()!=u.hostname.lower(): continue
            if unquote(attrs.get('share','')).casefold()!=parts[0].casefold(): continue
            if str(u.port or 445)!=attrs.get('port','445'): continue
            return str(mount.joinpath(*parts[1:]))
    except OSError: pass
    return None


def prepare_default(uri,cancel=None):
    entry=inspect(uri,cancel)
    if entry['isDir']: raise ValueError('This item is a folder.')
    if entry['kind'] not in ('file',): raise ValueError('Cannot open a special filesystem object.')
    content=entry.get('contentType') or 'application/octet-stream'
    # IMPORTANT: querying by URI would prefer x-scheme-handler/smb and could
    # relaunch OpenXplorer on a PDF/MP4, resulting in "Not a directory".
    app=choose_application(Gio.AppInfo.get_all_for_type(content),Gio.AppInfo.get_default_for_type(content,False))
    path=local_path(uri)
    if not path and not app.supports_uris():
        raise ValueError('This application needs a local path. Install gvfs-fuse or mount the share with CIFS, then reopen it; or choose a URI-capable application with Open with…')
    file=Gio.File.new_for_path(path) if path else Gio.File.new_for_uri(uri)
    return app,file,entry


class GioReader(io.RawIOBase):
    """A seekable archive stream; GIO short reads are not treated as EOF."""
    def __init__(self,file,cancel):
        super().__init__()
        self.cancel=cancel
        self.stream=None
        try:
            self.stream=file.read(raw(cancel))
            self.size=file.query_info('standard::size',Gio.FileQueryInfoFlags.NONE,raw(cancel)).get_size()
            if not self.stream.can_seek():
                raise ValueError('This share does not support seekable ZIP reading. Mount it locally or use an archive manager.')
        except Exception:
            self.close()
            raise
    def read(self,n=-1):
        if self.closed: raise ValueError('Read from a closed ZIP stream.')
        if self.cancel: self.cancel.check()
        if n is None or n<0: n=max(0,self.size-self.tell())
        data=bytearray()
        while len(data)<n:
            if self.cancel: self.cancel.check()
            block=bytes(self.stream.read_bytes(min(64*1024,n-len(data)),raw(self.cancel)).get_data())
            if not block: break
            data.extend(block)
        return bytes(data)
    def seek(self,offset,whence=0):
        if self.closed: raise ValueError('Seek on a closed ZIP stream.')
        if whence not in (0,1,2): raise ValueError('Invalid ZIP seek mode.')
        pos=offset+(self.tell() if whence==1 else self.size if whence==2 else 0)
        if pos<0: raise ValueError('Invalid ZIP offset.')
        if self.cancel: self.cancel.check()
        self.stream.seek(pos,GLib.SeekType.SET,raw(self.cancel)); return pos
    def tell(self): return self.stream.tell()
    def readable(self): return True
    def seekable(self): return True
    def close(self):
        try:
            if not self.closed and self.stream is not None: self.stream.close(None)
        finally: super().close()


@contextmanager
def archive_stream(uri,cancel=None):
    path=local_path(uri)
    if path:
        with open(path,'rb') as source: yield source
    else:
        with GioReader(Gio.File.new_for_uri(normalise_location(uri)),cancel) as source: yield source
