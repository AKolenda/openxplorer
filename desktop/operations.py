# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Conservative transfer orchestration; production I/O is supplied by GIO.

Copies are built in a newly created, unguessable staging DIRECTORY at the
recipient. Each top-level item is renamed into its final name only when the
copy succeeds. The explicit Replace policy commits completed staged files with
the backend's overwrite operation and merges same-name directories; Skip never
touches the existing item. No source is deleted by copy. Moves explicitly
prohibit a copy/delete fallback. Trash never falls back
to permanent deletion; a permanent delete is a separate mode the user has to
confirm explicitly, and is offered where the location has no Trash at all. This is not a crash-recovery/undo or filesystem snapshot
engine. A crash can leave a .winspace-transfer-*.part directory to inspect.
"""
from __future__ import annotations
from dataclasses import dataclass, field
import os
from typing import Callable, Iterator, Protocol
from urllib.parse import unquote
import uuid
from core import new_copy_name, split_location


class Cancelled(Exception):
    pass


class ReplaceUnsupported(Exception):
    """The backend cannot atomically overwrite, but native rename may work."""
    pass


@dataclass(frozen=True)
class Info:
    kind: str  # directory, file, symlink, special
    size: int = 0
    mode: int | None = None  # Unix permissions, only when the provider exposes them


class Cancellation(Protocol):
    def check(self) -> None: ...
    def is_cancelled(self) -> bool: ...


class Node(Protocol):
    uri: str
    name: str
    path: str | None
    def child(self, name: str) -> 'Node': ...
    def parent(self) -> 'Node | None': ...
    def exists(self, cancel: Cancellation | None = None) -> bool: ...
    def info(self, cancel: Cancellation | None = None) -> Info: ...
    def is_directory(self, cancel: Cancellation | None = None) -> bool: ...
    def children(self, cancel: Cancellation | None = None) -> Iterator['Node']: ...
    def mkdir(self, cancel: Cancellation | None = None) -> None: ...
    def copy_file(self, target: 'Node', cancel: Cancellation, progress: Callable[[int, int], None]) -> None: ...
    def move_native(self, target: 'Node', cancel: Cancellation | None = None) -> None: ...
    def replace_native(self, target: 'Node', cancel: Cancellation | None = None) -> None: ...
    def delete(self) -> None: ...  # used ONLY on this engine's exclusive staging tree
    def trash(self, cancel: Cancellation) -> None: ...
    def delete_tree(self, cancel: Cancellation, assert_writable=None) -> None: ...  # explicit permanent delete


@dataclass
class Result:
    done: list[str] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    cancelled: bool = False

    def as_dict(self) -> dict:
        return {'done': self.done, 'skipped': self.skipped,
                'errors': self.errors, 'cancelled': self.cancelled}


def guard_destination(source: Node, directory: Node) -> None:
    """Reject self/descendant transfers, including local symlink aliases.

    SMB host aliases cannot be proven identical without server support. The
    staging-name recursion check below adds a second guard for such aliases.
    Case-folding SMB paths is intentionally conservative.
    """
    if source.path and directory.path:
        s, d = os.path.realpath(source.path), os.path.realpath(directory.path)
        if s == d or os.path.commonpath([s, d]) == s:
            raise ValueError('Cannot place a folder inside itself (including through a symlink).')
    a, b = split_location(source.uri), split_location(directory.uri)
    if a.scheme == b.scheme and a.netloc.lower() == b.netloc.lower():
        s, d = unquote(a.path).rstrip('/'), unquote(b.path).rstrip('/')
        if a.scheme == 'smb':
            s, d = s.casefold(), d.casefold()
        if d == s or d.startswith(s + '/'):
            raise ValueError('Cannot place a folder inside itself.')


class TransferEngine:
    def __init__(self, factory: Callable[[str], Node], emit: Callable[[dict], None] | None = None,
                 assert_writable: Callable[[str], None] | None = None):
        self.factory = factory
        self.emit = emit or (lambda _: None)
        self.assert_writable = assert_writable

    def run(self, mode: str, uris: list[str], target: str | None,
            policy: str, cancel: Cancellation) -> Result:
        if mode not in ('copy', 'move', 'trash', 'delete'):
            raise ValueError('Unknown operation.')
        if policy not in ('skip', 'keep-both', 'replace'):
            raise ValueError('Choose Skip duplicates, Keep both, or Replace existing.')
        if not isinstance(uris, list) or not uris or len(uris) > 100000:
            raise ValueError('Select between 1 and 100,000 items.')
        uris = list(dict.fromkeys(uris))
        result = Result()
        removal = mode in ('trash', 'delete')
        dest_dir = self.factory(target) if target and not removal else None
        if not removal and dest_dir is None:
            raise ValueError('Choose a destination folder.')
        if dest_dir and not dest_dir.is_directory(cancel):
            raise ValueError('The destination is not a folder.')
        for index, uri in enumerate(uris):
            stage: Node | None = None
            try:
                cancel.check()
                source = self.factory(uri)
                if source.parent() is None:
                    raise ValueError('Filesystem roots cannot be copied, moved or trashed as items.')
                info = source.info(cancel)
                # Trash/delete have no byte progress, so the batch position is
                # the only honest fraction to show for them.
                self.emit({'label': f'{("Delete" if mode == "delete" else mode.title())}: {source.name} ({index+1}/{len(uris)})',
                           'fraction': index / len(uris) if removal else 0})
                if removal:
                    self._check_write_tree(source, None, cancel, source_writable=True)
                    if mode == 'trash':
                        source.trash(cancel)
                    else:
                        source.delete_tree(cancel, self.assert_writable)
                    result.done.append(uri)
                    continue
                assert dest_dir is not None
                if info.kind == 'directory':
                    guard_destination(source, dest_dir)
                destination = dest_dir.child(source.name)
                if mode == 'move' and destination.uri == source.uri:
                    result.skipped.append(uri)
                    continue
                if destination.exists(cancel):
                    if policy == 'skip':
                        result.skipped.append(uri)
                        continue
                    if policy == 'keep-both':
                        count = 2
                        while destination.exists(cancel):
                            cancel.check()
                            destination = dest_dir.child(new_copy_name(source.name, count, info.kind == 'directory'))
                            count += 1
                            if count > 10000:
                                raise ValueError('Too many duplicate names. Rename the item before copying.')
                # Check every affected path before changing this top-level item.
                # A writable parent can contain protected backup descendants.
                self._check_write_tree(source, destination, cancel, source_writable=mode == 'move')
                if mode == 'move':
                    # Backends MUST use NO_FALLBACK_FOR_MOVE. Replace is an
                    # explicit user choice and still never degrades to copy/delete.
                    if policy == 'replace':
                        self._commit_replace(source, destination, cancel)
                    else:
                        source.move_native(destination, cancel)
                    result.done.append(uri)
                    continue
                # Reserve a private namespace. A failed mkdir never grants us
                # permission to delete that name during cleanup.
                candidate = dest_dir.child('.winspace-transfer-' + uuid.uuid4().hex + '.part')
                candidate.mkdir(cancel)
                stage = candidate
                self._secure_local_staging(stage)
                staged_item = stage.child('payload')
                directory_modes = {}
                self._copy(source, staged_item, cancel, stage.name, 0, directory_modes)
                cancel.check()
                # Native rename in the same destination directory. Replace is
                # only reached after the user explicitly chose it; other
                # policies retain the no-overwrite race guard.
                if policy == 'replace':
                    self._commit_replace(staged_item, destination, cancel, directory_modes)
                else:
                    self._publish_staged(staged_item, destination, directory_modes, cancel)
                result.done.append(uri)
                stage.delete()  # now empty; cannot recursively remove final item
                stage = None
            except Exception as exc:
                if isinstance(exc, Cancelled) or cancel.is_cancelled():
                    result.cancelled = True
                else:
                    result.errors.append(f'{self.factory(uri).name}: {exc}')
            finally:
                if stage is not None:
                    try:
                        self._clean_staging(stage)
                    except Exception as exc:
                        result.errors.append(f'Incomplete staging folder left at {stage.uri}. Inspect it before removing it. {exc}')
            if result.cancelled:
                break
        self.emit({'label': f'{len(result.done)} item(s) completed', 'fraction': 1})
        return result

    @staticmethod
    def _secure_local_staging(stage: Node) -> None:
        """Harden real local staging without applying Unix modes to GVfs.

        MTP, AFC and many SMB backends expose a FUSE path but do not implement
        chmod. Their random staging namespace remains private to the connected
        session. Local paths use an opened, no-follow directory descriptor so
        a path swap cannot redirect the permission change.
        """
        TransferEngine._set_local_directory_mode(stage, 0o700)

    @staticmethod
    def _set_local_directory_mode(node: Node, mode: int) -> None:
        path = getattr(node, 'path', None)
        if not path or not node.uri.startswith('file:'):
            return
        flags = os.O_RDONLY | getattr(os, 'O_DIRECTORY', 0) | getattr(os, 'O_NOFOLLOW', 0)
        descriptor = os.open(path, flags)
        try:
            os.fchmod(descriptor, mode)
        finally:
            os.close(descriptor)

    def _check_write_tree(self, source: Node, destination: Node | None,
                          cancel: Cancellation, source_writable: bool = False,
                          depth: int = 0) -> None:
        """Preflight without following links or modifying selected directories."""
        if self.assert_writable is None:
            return
        cancel.check()
        if depth > 128:
            raise ValueError('Folder nesting exceeds this build’s safety limit (128).')
        if source_writable:
            self.assert_writable(source.uri)
        if destination is not None:
            self.assert_writable(destination.uri)
        if source.info(cancel).kind == 'directory':
            for child in source.children(cancel):
                self._check_write_tree(child, destination.child(child.name) if destination else None,
                                       cancel, source_writable, depth + 1)

    @staticmethod
    def _restore_directory_modes(source: Node, modes: dict, cancel: Cancellation) -> None:
        # Restore children before parents, and only immediately before publishing
        # a complete subtree. Restrictive source modes must not prevent building,
        # merging or cleaning an exclusively owned staging directory.
        cancel.check()
        for child in source.children(cancel):
            if child.uri in modes:
                TransferEngine._restore_directory_modes(child, modes, cancel)
        pending = modes.pop(source.uri, None)
        if pending is not None:
            TransferEngine._set_local_directory_mode(*pending)

    @staticmethod
    def _publish_staged(source: Node, destination: Node, modes: dict, cancel: Cancellation) -> None:
        # Linux requires owner write access when moving a directory between
        # parents. Retain it just for the rename, then restore the exact mode
        # through the already-open descriptor. Group/other permissions are
        # restricted before publication, so private contents are never exposed.
        root = modes.pop(source.uri, None)
        descriptor = None
        published = False
        if root is not None:
            flags = os.O_RDONLY | getattr(os, 'O_DIRECTORY', 0) | getattr(os, 'O_NOFOLLOW', 0)
            descriptor = os.open(source.path, flags)
        try:
            if root is not None and modes:
                TransferEngine._restore_directory_modes(source, modes, cancel)
            if descriptor is not None:
                os.fchmod(descriptor, root[1] | 0o700)
            source.move_native(destination, cancel)
            published = True
        finally:
            if descriptor is not None:
                try:
                    try:
                        os.fchmod(descriptor, root[1])
                    except OSError as exc:
                        if published:
                            raise ValueError(f'The copied folder exists at {destination.uri}, but its final permissions could not be restored. {exc}') from exc
                        raise
                finally:
                    os.close(descriptor)

    def _commit_replace(self, source: Node, destination: Node,
                        cancel: Cancellation, directory_modes: dict | None = None) -> None:
        """Commit one completed item using Windows-like replace semantics.

        Same-name directories merge recursively and keep destination-only
        children. Files and symlinks use the backend's explicit overwrite move.
        A file/folder type mismatch is left untouched instead of deleting a
        directory tree as a side effect of a batch choice.
        """
        cancel.check()
        if self.assert_writable is not None:
            self.assert_writable(destination.uri)
        if not destination.exists(cancel):
            if directory_modes is not None:
                self._publish_staged(source, destination, directory_modes, cancel)
            else:
                source.move_native(destination, cancel)
            return
        incoming, existing = source.info(cancel), destination.info(cancel)
        if incoming.kind == 'directory' and existing.kind == 'directory':
            for child in list(source.children(cancel)):
                self._commit_replace(child, destination.child(child.name), cancel, directory_modes)
            source.delete()
            if directory_modes is not None:
                directory_modes.pop(source.uri, None)
            return
        if 'directory' in (incoming.kind, existing.kind):
            raise ValueError('A file and folder have the same name. Rename or remove one of them, then try again.')
        if incoming.kind not in ('file', 'symlink') or existing.kind not in ('file', 'symlink'):
            raise ValueError('This item type cannot be replaced automatically.')
        try:
            source.replace_native(destination, cancel)
        except ReplaceUnsupported:
            self._replace_via_backup(source, destination, cancel)

    @staticmethod
    def _replace_via_backup(source: Node, destination: Node,
                            cancel: Cancellation) -> None:
        """Replace a file on backends such as MTP using reversible renames.

        The old destination is retained under an unguessable sibling name until
        the completed incoming file is installed. If installation fails, the
        old name is restored. No copy/delete fallback is used for a move.
        """
        parent = destination.parent()
        if parent is None:
            raise ValueError('Filesystem roots cannot be replaced.')
        backup = None
        for _ in range(100):
            candidate = parent.child('.winspace-replaced-' + uuid.uuid4().hex + '.backup')
            if not candidate.exists(cancel):
                backup = candidate
                break
        if backup is None:
            raise ValueError('Could not reserve a temporary replacement name.')
        destination.move_native(backup, cancel)
        try:
            # Once the old file moved aside, finish the tiny commit step even
            # if cancellation arrives; stopping here would unnecessarily leave
            # the public destination name empty.
            source.move_native(destination, None)
        except Exception as install_error:
            try:
                backup.move_native(destination, None)
            except Exception as restore_error:
                raise ValueError(
                    f'Replacement failed and the original remains at {backup.uri}. '
                    f'Restore it manually before retrying. {restore_error}') from install_error
            raise
        try:
            backup.delete()
        except Exception as cleanup_error:
            raise ValueError(
                f'Replacement completed, but the prior file remains at {backup.uri}. '
                f'Remove that backup after checking the new file. {cleanup_error}') from cleanup_error

    def _copy(self, source: Node, target: Node, cancel: Cancellation,
              own_stage_name: str, depth: int, directory_modes: dict) -> None:
        cancel.check()
        if depth > 128:
            raise ValueError('Folder nesting exceeds this build’s safety limit (128).')
        if source.name == own_stage_name:
            raise ValueError('The destination resolves inside the source through an alias. Copy stopped.')
        info = source.info(cancel)  # lstat/NOFOLLOW_SYMLINKS, never traverse links
        if info.kind == 'directory':
            target.mkdir(cancel)
            if target.path and target.uri.startswith('file:'):
                mode = info.mode if info.mode is not None else target.info(cancel).mode
                directory_modes[target.uri] = (target, mode if mode is not None else 0o700)
                self._secure_local_staging(target)
            for child in source.children(cancel):
                self._copy(child, target.child(child.name), cancel, own_stage_name, depth + 1, directory_modes)
        elif info.kind in ('file', 'symlink'):
            def progress(current: int, total: int) -> None:
                cancel.check()
                self.emit({'label': f'Copying {source.name} · {current:,} / {total:,} bytes',
                           'fraction': current / total if total else 0})
            source.copy_file(target, cancel, progress)
        else:
            raise ValueError('Sockets, devices and other special files are not copied.')

    @staticmethod
    def _clean_staging(node: Node) -> None:
        # This root was exclusively created by us. Children are inspected with
        # NOFOLLOW_SYMLINKS. Never call this on a user-selected path.
        if node.info().kind == 'directory':
            TransferEngine._secure_local_staging(node)
            for child in node.children():
                TransferEngine._clean_staging(child)
        node.delete()
