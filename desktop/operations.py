# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Conservative transfer orchestration; production I/O is supplied by GIO.

Copies are built in a newly created, unguessable staging DIRECTORY at the
recipient. Each top-level item is renamed into its final name only when the
copy succeeds. Existing names are never overwritten. No source is deleted by
copy. Moves explicitly prohibit a copy/delete fallback. Trash never falls back
to permanent deletion; a permanent delete is a separate mode the user has to
confirm explicitly, and is offered where the location has no Trash at all. This is not a crash-recovery/undo or filesystem snapshot
engine. A crash can leave a .winspace-transfer-*.part directory to inspect.
"""
from __future__ import annotations
from dataclasses import dataclass, field
import os
from typing import Callable, Iterator, Protocol
from urllib.parse import urlsplit, unquote
import uuid
from core import new_copy_name


class Cancelled(Exception):
    pass


@dataclass(frozen=True)
class Info:
    kind: str  # directory, file, symlink, special
    size: int = 0


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
    def delete(self) -> None: ...  # used ONLY on this engine's exclusive staging tree
    def trash(self, cancel: Cancellation) -> None: ...
    def delete_tree(self, cancel: Cancellation) -> None: ...  # explicit permanent delete


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
    a, b = urlsplit(source.uri), urlsplit(directory.uri)
    if a.scheme == b.scheme and a.netloc.lower() == b.netloc.lower():
        s, d = unquote(a.path).rstrip('/'), unquote(b.path).rstrip('/')
        if a.scheme == 'smb':
            s, d = s.casefold(), d.casefold()
        if d == s or d.startswith(s + '/'):
            raise ValueError('Cannot place a folder inside itself.')


class TransferEngine:
    def __init__(self, factory: Callable[[str], Node], emit: Callable[[dict], None] | None = None):
        self.factory = factory
        self.emit = emit or (lambda _: None)

    def run(self, mode: str, uris: list[str], target: str | None,
            policy: str, cancel: Cancellation) -> Result:
        if mode not in ('copy', 'move', 'trash', 'delete'):
            raise ValueError('Unknown operation.')
        if policy not in ('skip', 'keep-both'):
            raise ValueError('Only Skip duplicates or Keep both is supported. Overwrite is disabled.')
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
                    if mode == 'trash':
                        source.trash(cancel)
                    else:
                        source.delete_tree(cancel)
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
                    count = 2
                    while destination.exists(cancel):
                        cancel.check()
                        destination = dest_dir.child(new_copy_name(source.name, count, info.kind == 'directory'))
                        count += 1
                        if count > 10000:
                            raise ValueError('Too many duplicate names. Rename the item before copying.')
                if mode == 'move':
                    # Backends MUST use NO_FALLBACK_FOR_MOVE and never OVERWRITE.
                    source.move_native(destination, cancel)
                    result.done.append(uri)
                    continue
                # Reserve a private namespace. A failed mkdir never grants us
                # permission to delete that name during cleanup.
                candidate = dest_dir.child('.winspace-transfer-' + uuid.uuid4().hex + '.part')
                candidate.mkdir(cancel)
                stage = candidate
                if getattr(stage, 'path', None):
                    os.chmod(stage.path, 0o700, follow_symlinks=False)
                staged_item = stage.child('payload')
                self._copy(source, staged_item, cancel, stage.name, 0)
                cancel.check()
                # Native rename in the same destination directory, no overwrite.
                # A racing conflicting name therefore fails, preserving it.
                staged_item.move_native(destination, cancel)
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

    def _copy(self, source: Node, target: Node, cancel: Cancellation,
              own_stage_name: str, depth: int) -> None:
        cancel.check()
        if depth > 128:
            raise ValueError('Folder nesting exceeds this build’s safety limit (128).')
        if source.name == own_stage_name:
            raise ValueError('The destination resolves inside the source through an alias. Copy stopped.')
        info = source.info(cancel)  # lstat/NOFOLLOW_SYMLINKS, never traverse links
        if info.kind == 'directory':
            target.mkdir(cancel)
            for child in source.children(cancel):
                self._copy(child, target.child(child.name), cancel, own_stage_name, depth + 1)
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
            for child in node.children():
                TransferEngine._clean_staging(child)
        node.delete()
