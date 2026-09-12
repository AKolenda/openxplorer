# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""GIO/GVfs provider. All filesystem I/O here is called on bounded workers."""
from __future__ import annotations
import os
from pathlib import Path
import time
from typing import Callable
import gi
gi.require_version('Gio', '2.0')
from gi.repository import Gio, GLib
from core import normalise_location, validate_name, require_item_uri, is_smb_server
from entry_model import classify_entry
from operations import Cancelled, Info

ATTRIBUTES = 'standard::name,standard::display-name,standard::type,standard::is-hidden,standard::is-symlink,standard::size,standard::content-type,standard::target-uri,standard::is-virtual,time::modified'
NOFOLLOW = Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS
COPY_FLAGS = Gio.FileCopyFlags.NOFOLLOW_SYMLINKS
MOVE_FLAGS = COPY_FLAGS | Gio.FileCopyFlags.NO_FALLBACK_FOR_MOVE


class GioCancellation:
    def __init__(self):
        self.raw = Gio.Cancellable()
    def cancel(self) -> None:
        self.raw.cancel()
    def is_cancelled(self) -> bool:
        return self.raw.is_cancelled()
    def check(self) -> None:
        if self.raw.is_cancelled():
            raise Cancelled('Operation cancelled.')


def raw(cancel):
    return cancel.raw if cancel else None


def error_payload(exc: Exception) -> dict:
    code = 'error'
    if isinstance(exc, Cancelled):
        code = 'cancelled'
    if isinstance(exc, GLib.Error):
        for name, out in [('NOT_MOUNTED','not-mounted'), ('CANCELLED','cancelled'),
                          ('NOT_FOUND','not-found'), ('PERMISSION_DENIED','permission-denied'),
                          ('EXISTS','exists'), ('NOT_SUPPORTED','not-supported'), ('NOT_DIRECTORY','not-directory')]:
            if exc.matches(Gio.io_error_quark(), getattr(Gio.IOErrorEnum, name)):
                code = out
                break
    return {'code': code, 'message': str(exc)}


class GioNode:
    def __init__(self, uri: str | None = None, gfile=None):
        self.file = gfile if gfile is not None else Gio.File.new_for_uri(normalise_location(uri))
        self.uri = self.file.get_uri()
        self.name = self.file.get_basename() or self.uri
        self.path = self.file.get_path()

    def child(self, name):
        # Generated staging names and source names can include backslashes on
        # POSIX. User-entered names are validated separately at the UI boundary.
        if not name or name in ('.', '..') or '/' in name or '\x00' in name:
            raise ValueError('Invalid child name.')
        return GioNode(gfile=self.file.get_child(name))

    def parent(self):
        p = self.file.get_parent()
        return GioNode(gfile=p) if p else None

    def exists(self, cancel=None):
        return self.file.query_exists(raw(cancel))

    def info(self, cancel=None):
        i = self.file.query_info('standard::type,standard::size', NOFOLLOW, raw(cancel))
        kind = {Gio.FileType.DIRECTORY: 'directory', Gio.FileType.REGULAR: 'file',
                Gio.FileType.SYMBOLIC_LINK: 'symlink'}.get(i.get_file_type(), 'special')
        return Info(kind, i.get_size())

    def is_directory(self, cancel=None):
        info = self.file.query_info('standard::type', Gio.FileQueryInfoFlags.NONE, raw(cancel))
        return info.get_file_type() == Gio.FileType.DIRECTORY

    def children(self, cancel=None):
        en = self.file.enumerate_children('standard::name', NOFOLLOW, raw(cancel))
        try:
            while True:
                if cancel:
                    cancel.check()
                info = en.next_file(raw(cancel))
                if info is None:
                    break
                yield GioNode(gfile=en.get_child(info))
        finally:
            en.close(None)

    def mkdir(self, cancel=None):
        self.file.make_directory(raw(cancel))

    def copy_file(self, target, cancel, progress):
        def callback(current, total, *unused):
            progress(current, total)
        self.file.copy(target.file, COPY_FLAGS, raw(cancel), callback, None)

    def move_native(self, target, cancel=None):
        require_item_uri(self.uri)
        try:
            self.file.move(target.file, MOVE_FLAGS, raw(cancel), None, None)
        except GLib.Error as exc:
            if exc.matches(Gio.io_error_quark(), Gio.IOErrorEnum.NOT_SUPPORTED) or exc.matches(Gio.io_error_quark(), Gio.IOErrorEnum.WOULD_RECURSE):
                raise ValueError('A native move is not supported here. Cross-filesystem/cross-share moves are deliberately disabled. Copy, verify, then trash the source separately.') from exc
            raise

    def delete(self):
        self.file.delete(None)

    def trash(self, cancel):
        require_item_uri(self.uri)
        try:
            self.file.trash(raw(cancel))
        except GLib.Error as exc:
            if exc.matches(Gio.io_error_quark(), Gio.IOErrorEnum.NOT_SUPPORTED):
                raise ValueError('Trash is not supported at this location. The original item was not permanently deleted. Delete it permanently instead, or use the server’s recycle-bin policy.') from exc
            raise

    def can_trash(self, cancel=None) -> bool:
        """Ask GIO whether this location has a usable Trash. Shares and most
        remote backends answer no; the UI then offers an explicit permanent
        delete instead of a Trash move that can only fail."""
        try:
            info = self.file.query_info('access::can-trash', NOFOLLOW, raw(cancel))
        except GLib.Error as exc:
            # Let a not-mounted share reach the caller so the usual mount retry runs.
            if exc.matches(Gio.io_error_quark(), Gio.IOErrorEnum.NOT_MOUNTED):
                raise
            return False
        return bool(info.get_attribute_boolean('access::can-trash'))

    def delete_tree(self, cancel):
        """Permanent, unrecoverable delete of a user-selected item. Only ever
        reached when the user confirmed a permanent delete in the UI. Symlinks
        are removed as links; their targets are never traversed."""
        require_item_uri(self.uri)
        self._delete_recursive(cancel, 0)

    def _delete_recursive(self, cancel, depth):
        cancel.check()
        if depth > 128:
            raise ValueError('Folder nesting exceeds this build’s safety limit (128).')
        info = self.file.query_info('standard::type', NOFOLLOW, raw(cancel))
        if info.get_file_type() == Gio.FileType.DIRECTORY:
            for child in self.children(cancel):
                child._delete_recursive(cancel, depth + 1)
        self.file.delete(raw(cancel))


def entry_from_info(gfile, info) -> dict:
    name = info.get_display_name() or info.get_name() or gfile.get_basename() or gfile.get_uri()
    kind = {Gio.FileType.DIRECTORY: 'directory', Gio.FileType.REGULAR: 'file',
            Gio.FileType.MOUNTABLE: 'mountable', Gio.FileType.SHORTCUT: 'shortcut',
            Gio.FileType.SYMBOLIC_LINK: 'symlink', Gio.FileType.SPECIAL: 'special'}.get(info.get_file_type(), 'unknown')
    content_type = info.get_content_type()
    model = classify_entry(kind, gfile.get_uri(), content_type,
                           info.get_attribute_string('standard::target-uri'),
                           info.get_attribute_boolean('standard::is-virtual') if info.has_attribute('standard::is-virtual') else False)
    description = model.pop('folderType') or (Gio.content_type_get_description(content_type) if content_type else 'File')
    return {'uri': gfile.get_uri(), 'name': name, **model,
            'size': None if model['isDir'] or not info.has_attribute('standard::size') else info.get_size(),
            'type': description or 'File', 'contentType': content_type,
            'modified': info.get_attribute_uint64('time::modified') if info.has_attribute('time::modified') else 0,
            'hidden': bool(info.get_is_hidden()), 'symlink': bool(info.get_is_symlink())}


def trash_support(uri: str, cancel=None) -> dict:
    """Report whether a folder (or item) supports a Trash move."""
    return {'canTrash': GioNode(uri).can_trash(cancel)}


def verify_pin(uri: str, label: str = '', cancel=None) -> dict:
    """Inspect only the dropped item, not its children. A share need not be
    mounted to pin its validated target URI from the server listing.
    """
    entry = inspect(uri, cancel)
    if not entry['isDir']:
        raise ValueError('Only folders and network shares can be pinned to Quick access.')
    return {'uri': normalise_location(entry.get('targetUri') or entry['uri']),
            'label': label or entry['name']}


def enumerate_folder(uri: str, show_hidden: bool, cancel: GioCancellation,
                     emit: Callable[[list[dict]], None]) -> dict:
    file = Gio.File.new_for_uri(normalise_location(uri))
    # GIO owns backend enumeration. We never perform a Python os.stat per row.
    en = file.enumerate_children(ATTRIBUTES, Gio.FileQueryInfoFlags.NONE, cancel.raw)
    batch = []
    count = 0
    try:
        while True:
            cancel.check()
            info = en.next_file(cancel.raw)
            if info is None:
                break
            if not show_hidden and info.get_is_hidden():
                continue
            batch.append(entry_from_info(en.get_child(info), info))
            count += 1
            if len(batch) >= 128:
                emit(batch)
                batch = []
        if batch:
            emit(batch)
    finally:
        en.close(None)
    return {'uri': file.get_uri(), 'count': count}


def inspect(uri: str, cancel=None) -> dict:
    file = Gio.File.new_for_uri(normalise_location(uri))
    return entry_from_info(file, file.query_info(ATTRIBUTES, Gio.FileQueryInfoFlags.NONE, raw(cancel)))


def verify_folder(uri: str, cancel=None) -> str:
    file = Gio.File.new_for_uri(normalise_location(uri))
    info = file.query_info('standard::type', Gio.FileQueryInfoFlags.NONE, raw(cancel))
    if info.get_file_type() != Gio.FileType.DIRECTORY:
        raise ValueError('This location is not a folder.')
    return file.get_uri()


def create_item(uri: str, name: str, kind: str, cancel=None) -> dict:
    validate_name(name)
    if is_smb_server(uri):
        raise ValueError('Open a network share before creating files or folders.')
    parent = Gio.File.new_for_uri(normalise_location(uri))
    child = parent.get_child(name)
    if kind == 'folder':
        child.make_directory(raw(cancel))
    elif kind == 'file':
        stream = child.create(Gio.FileCreateFlags.NONE, raw(cancel))
        stream.close(raw(cancel))
    else:
        raise ValueError('Unknown item kind.')
    return {'uri': child.get_uri()}


def rename_item(uri: str, name: str, cancel=None) -> dict:
    validate_name(name)
    src = GioNode(require_item_uri(uri))
    parent = src.parent()
    if parent is None:
        raise ValueError('Cannot rename a filesystem root.')
    dest = parent.child(name)
    if src.uri == dest.uri:
        return {'uri': src.uri}
    src.move_native(dest, cancel)
    return {'uri': dest.uri}


def index_directory(uri: str, include_hidden: bool, cancel: GioCancellation):
    """Stream metadata without following symlinks, opening contents or mounting.
    An unmounted SMB share raises NOT_MOUNTED: the background crawler stops.
    """
    file=Gio.File.new_for_uri(normalise_location(uri))
    en=file.enumerate_children(ATTRIBUTES,NOFOLLOW,cancel.raw)
    batch=[]
    try:
        while True:
            cancel.check()
            info=en.next_file(cancel.raw)
            if info is None:
                break
            if not include_hidden and info.get_is_hidden():
                continue
            e=entry_from_info(en.get_child(info),info)
            if e['symlink'] or e['kind']=='symlink' or e['isVirtual']:
                continue
            batch.append(e)
            if len(batch)>=256:
                yield batch
                batch=[]
        if batch:
            yield batch
    finally:
        en.close(None)


def discover_servers(cancel: GioCancellation) -> dict:
    """Read GVfs's combined DNS-SD / WS-Discovery / SMB browser. No port scan,
    no passwords, and no enumeration of any discovered server's share contents.
    The host mounts network:/// silently before calling this worker.
    """
    from urllib.parse import urlsplit
    servers={}
    warnings=[]
    sources=['network:///']
    for source in sources:
        en=None
        try:
            en=Gio.File.new_for_uri(source).enumerate_children(ATTRIBUTES,NOFOLLOW,cancel.raw)
            for _ in range(500):
                cancel.check()
                info=en.next_file(cancel.raw)
                if info is None:
                    break
                target=info.get_attribute_string('standard::target-uri')
                if not target:
                    continue
                try:
                    u=urlsplit(normalise_location(target))
                except (ValueError,TypeError):
                    continue
                if u.scheme!='smb' or not u.hostname:
                    continue
                uri='smb://'+u.netloc+'/'
                servers[uri]={'uri':uri,'label':info.get_display_name() or u.hostname,
                              'host':u.hostname,'source':'Network discovery','connected':False}
        except Exception as exc:
            cancel.check()
            warnings.append(str(exc))
        finally:
            if en:
                en.close(None)
    return {'servers':sorted(servers.values(),key=lambda s:s['label'].casefold()),
            'warnings':warnings,'method':'GVfs network discovery',
            'note':'Finds servers advertising on this network. Firewalls, VLANs, disabled discovery or missing GVfs services can hide devices. Enter a server address manually when needed.'}


from contextlib import contextmanager


@contextmanager
def exclusive_output(node, cancel):
    """New regular file, no replacement or archived executable/ownership bits."""
    stream = node.file.create(Gio.FileCreateFlags.PRIVATE, raw(cancel))
    class Writer:
        def write(self, block):
            cancel.check()
            ok, written = stream.write_all(block, raw(cancel))
            if not ok or written != len(block):
                raise OSError('Incomplete extraction write.')
            return written
    try:
        yield Writer()
    except BaseException:
        try:
            stream.close(None)
        except Exception:
            pass
        raise
    else:
        # Close with no cancelled GCancellable to flush/release the handle.
        stream.close(None)
