# SPDX-License-Identifier: AGPL-3.0-only
"""Transactional ZIP extraction into a NEW folder, never into existing content.

Sources may be local or seekable GIO streams. Destination I/O is supplied by
GIO in production and a disposable local provider in tests. We preflight ALL
members before creating a staging folder, stream with cancellation/byte caps,
then publish using a non-overwriting native rename on the destination volume.
Archive permissions, symlinks, executables bits and ownership are not applied.
"""
from __future__ import annotations
from dataclasses import dataclass
import os
import stat
import unicodedata
import uuid
import zipfile
from core import validate_name
from operations import TransferEngine


@dataclass(frozen=True)
class Limits:
    entries: int = 100_000
    paths: int = 200_000   # includes implied parent directories
    depth: int = 128
    total_bytes: int = 20 * 1024**3
    member_bytes: int = 8 * 1024**3
    ratio: int = 1000


def suggested_name(filename: str) -> str:
    value = str(filename)
    if value.lower().endswith('.zip'):
        value = value[:-4]
    value = value.rstrip(' .') or 'Extracted files'
    return validate_name(value)


def member_parts(info: zipfile.ZipInfo, limits: Limits) -> tuple[str, ...]:
    name = info.filename
    # ZipInfo.filename truncates embedded NUL, whereas orig_filename retains it.
    if name != info.orig_filename or not name or len(name) > 4096:
        raise ValueError('ZIP contains an invalid or overlong member name.')
    if name.startswith('/') or '\\' in name or any(ord(c) < 32 or ord(c) == 127 for c in name):
        raise ValueError('ZIP contains an unsafe member path. Nothing was extracted.')
    parts = tuple(name.rstrip('/').split('/'))
    if len(parts) > limits.depth:
        raise ValueError('ZIP nesting exceeds the 128-level safety limit.')
    for part in parts:
        if (part in ('', '.', '..') or ':' in part or part.endswith((' ', '.'))
                or len(part.encode('utf-8')) > 255):
            raise ValueError('ZIP contains a path unsafe for local/SMB extraction. Nothing was extracted.')
        base = part.split('.')[0].casefold()
        if base in {'con', 'prn', 'aux', 'nul', *(f'com{i}' for i in range(1, 10)), *(f'lpt{i}' for i in range(1, 10))}:
            raise ValueError('ZIP contains a reserved device filename. Use an archive manager to inspect it.')
    mode = stat.S_IFMT(info.external_attr >> 16)
    if mode not in (0, stat.S_IFREG, stat.S_IFDIR) or (mode == stat.S_IFDIR and not info.is_dir()):
        raise ValueError('ZIP contains a symbolic link or special file. Nothing was extracted.')
    if info.flag_bits & 1:
        raise ValueError('Password-protected ZIPs need an external archive manager in this release.')
    if info.compress_type not in {zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED, zipfile.ZIP_BZIP2, zipfile.ZIP_LZMA}:
        raise ValueError('This ZIP compression method needs an external archive manager.')
    if info.file_size < 0 or info.compress_size < 0 or (info.is_dir() and info.file_size):
        raise ValueError('ZIP contains inconsistent size metadata.')
    if info.file_size > limits.member_bytes or info.file_size > max(1, info.compress_size) * limits.ratio:
        raise ValueError('ZIP exceeds the per-file decompression safety limit. Use an archive manager.')
    return parts


def plan(archive: zipfile.ZipFile, cancel, limits: Limits = Limits()) -> tuple[list, dict]:
    """Validate duplicates and file/directory clashes on case-insensitive shares."""
    members = archive.infolist()
    if len(members) > limits.entries:
        raise ValueError('ZIP has too many entries for the built-in extractor.')
    known, explicit, result = {}, set(), []
    total, files = 0, 0
    for item in members:
        cancel.check()
        parts = member_parts(item, limits)
        key = tuple(unicodedata.normalize('NFC', s).casefold() for s in parts)
        if key in explicit:
            raise ValueError('ZIP contains duplicate filenames. Nothing was extracted.')
        explicit.add(key)
        for depth in range(1, len(parts) + 1):
            sub, spelling = key[:depth], parts[:depth]
            kind = 'directory' if depth < len(parts) or item.is_dir() else 'file'
            if sub in known and known[sub] != (kind, spelling):
                raise ValueError('ZIP has conflicting or case-ambiguous paths. Nothing was extracted.')
            known[sub] = (kind, spelling)
        if len(known) > limits.paths:
            raise ValueError('ZIP has too many paths for the built-in extractor.')
        if not item.is_dir():
            total += item.file_size
            files += 1
            if total > limits.total_bytes:
                raise ValueError('ZIP exceeds the 20 GiB extraction limit. Use an archive manager.')
        result.append((item, parts))
    return result, {'files': files, 'folders': sum(k == 'directory' for k, _ in known.values()),
                    'bytes': total, 'entries': len(members)}


class ZipExtractor:
    def __init__(self, archives, factory, writer, emit=None, limits=Limits()):
        self.archives, self.factory, self.writer = archives, factory, writer
        self.emit = emit or (lambda value: None)
        self.limits = limits

    def inspect(self, uri, cancel):
        with self.archives.opened(uri, cancel) as archive:
            _, summary = plan(archive, cancel, self.limits)
        return {**summary, 'uri': uri}

    def extract(self, uri, target, name, cancel):
        name = validate_name(name)
        directory = self.factory(target)
        if directory.info(cancel).kind != 'directory':
            raise ValueError('Choose a real destination folder, not a link or server listing.')
        final = directory.child(name)
        if final.exists(cancel):
            raise FileExistsError('The destination already exists. Choose a new folder name; existing files are never overwritten.')
        stage = None
        published = None
        try:
            self.emit({'label': 'Checking ZIP contents…', 'fraction': 0})
            with self.archives.opened(uri, cancel) as archive:
                members, summary = plan(archive, cancel, self.limits)
                cancel.check()
                candidate = directory.child('.openxplorer-extract-' + uuid.uuid4().hex + '.part')
                candidate.mkdir(cancel)
                stage = candidate  # Only a successful, exclusive mkdir grants cleanup ownership.
                if stage.path:
                    os.chmod(stage.path, 0o700, follow_symlinks=False)
                dirs = {(): stage}
                count, total = 0, 0
                for item, parts in members:
                    cancel.check()
                    depth_limit = len(parts) if item.is_dir() else len(parts) - 1
                    for depth in range(1, depth_limit + 1):
                        key = parts[:depth]
                        if key not in dirs:
                            node = dirs[key[:-1]].child(key[-1])
                            node.mkdir(cancel)
                            dirs[key] = node
                    if item.is_dir():
                        continue
                    output = dirs[parts[:-1]].child(parts[-1])
                    written = 0
                    with archive.open(item, 'r') as source, self.writer(output, cancel) as destination:
                        while True:
                            cancel.check()
                            block = source.read(64 * 1024)
                            if not block:
                                break
                            written += len(block)
                            total += len(block)
                            if written > item.file_size or written > self.limits.member_bytes or total > self.limits.total_bytes:
                                raise ValueError('ZIP exceeded its declared size or the extraction safety limit.')
                            result = destination.write(block)
                            if result is not None and result != len(block):
                                raise OSError('The destination did not accept all extracted bytes.')
                            self.emit({'label': f'Extracting {item.filename} · {count + 1}/{summary["files"]} files',
                                       'fraction': min(.99, total / max(1, summary['bytes']))})
                    if written != item.file_size:
                        raise ValueError('ZIP member has a truncated size. Extraction stopped.')
                    count += 1
                cancel.check()
                # Same-volume non-overwrite rename. A competing name cannot be replaced.
                stage.move_native(final, cancel)
                published, stage = final.uri, None
            value = {**summary, 'uri': published, 'name': name, 'source': uri, 'target': target}
            self.emit({'label': f'Extracted {summary["files"]} files into {name}', 'fraction': 1})
            return value
        except Exception as exc:
            if stage is not None:
                try:
                    TransferEngine._clean_staging(stage)
                except Exception as cleanup:
                    raise RuntimeError(f'{exc}\nIncomplete extraction remains at {stage.uri}. Inspect it before removing it. {cleanup}') from exc
            # Close errors after publishing must never cause a success folder to be deleted.
            if published:
                raise RuntimeError(f'Extracted folder exists at {published}, but closing the source failed: {exc}') from exc
            raise
