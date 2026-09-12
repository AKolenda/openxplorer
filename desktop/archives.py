# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Read-only ZIP browsing. Listing never extracts file contents.

ZIP input can be a local file or a seekable GIO stream. Only an explicitly
selected member is decompressed, to a private temporary *copy*. No extractall,
no archive paths on disk, no executing templates/scripts, no write-back.
"""
from __future__ import annotations
from contextlib import contextmanager
from datetime import datetime
import io
import os
from pathlib import Path, PurePosixPath
import stat
import tempfile
import zipfile

MAX_DIRECTORY = 32 * 1024**2
MAX_MEMBERS = 100000
MAX_MEMBER = 256 * 1024**2
SAFE_VIEW_EXT = {'.txt','.md','.csv','.json','.html','.htm','.pdf','.doc','.docx','.odt','.xls','.xlsx','.ods','.ppt','.pptx','.odp','.png','.jpg','.jpeg','.webp','.gif','.bmp','.svg','.mp4','.mkv','.webm','.mp3','.wav','.ogg','.flac'}


def safe_member(name):
    if not isinstance(name,str) or not name or any(ord(c)<32 or ord(c)==127 for c in name) or '\\' in name or len(name)>4096:
        return False
    p = PurePosixPath(name)
    return not p.is_absolute() and ':' not in p.parts[0] and all(v not in ('..','.','') for v in name.rstrip('/').split('/'))


class BoundedReader:
    """Cap a malicious central-directory allocation before zipfile parses it."""
    def __init__(self, stream): self.stream = stream
    def read(self,n=-1):
        if n<0:
            position=self.stream.tell();self.stream.seek(0,2);n=self.stream.tell()-position;self.stream.seek(position)
        if n>MAX_DIRECTORY: raise ValueError('ZIP directory is too large for the built-in viewer. Use an archive manager.')
        return self.stream.read(n)
    def seek(self,*args): return self.stream.seek(*args)
    def tell(self): return self.stream.tell()
    def seekable(self): return True
    def readable(self): return True


class Archives:
    def __init__(self, opener, temp_root=None):
        self.opener = opener
        self.temp_root = Path(temp_root) if temp_root else None

    @contextmanager
    def opened(self, uri, cancel=None):
        with self.opener(uri,cancel) as stream:
            with zipfile.ZipFile(BoundedReader(stream),'r') as archive:
                if len(archive.infolist())>MAX_MEMBERS:
                    raise ValueError('ZIP has more than 100,000 members. Use an archive manager.')
                yield archive

    def list(self, uri, prefix='', cancel=None):
        if prefix and not safe_member(prefix): raise ValueError('Invalid archive folder.')
        prefix = prefix.rstrip('/')+'/' if prefix else ''
        rows, skipped = {},0
        with self.opened(uri,cancel) as archive:
            for entry in archive.infolist():
                if cancel: cancel.check()
                name = entry.filename
                if not safe_member(name) or entry.orig_filename != name or stat.S_IFMT(entry.external_attr>>16) not in (0,stat.S_IFREG,stat.S_IFDIR):
                    skipped += 1; continue
                if not name.startswith(prefix) or name == prefix: continue
                relative = name[len(prefix):]
                leaf = relative.rstrip('/').split('/')[0]
                folder = '/' in relative.rstrip('/') or entry.is_dir()
                member = prefix+leaf+('/' if folder else '')
                if member in rows: continue
                try: modified = datetime(*entry.date_time).timestamp()
                except (ValueError,OverflowError,OSError): modified=0
                rows[member]={'name':leaf,'member':member,'isDir':folder,'kind':'directory' if folder else 'file',
                    'size':None if folder else entry.file_size,'compressedSize':None if folder else entry.compress_size,
                    'modified':modified,'encrypted':bool(entry.flag_bits&1), 'readOnly':True,
                    'canOpen':not folder and not entry.flag_bits&1 and entry.file_size<=MAX_MEMBER and Path(leaf).suffix.lower() in SAFE_VIEW_EXT}
                if len(rows)>=5000: break
        return {'archiveUri':uri,'prefix':prefix,'entries':sorted(rows.values(),key=lambda e:(not e['isDir'],e['name'].casefold())),
            'readOnly':True,'skippedUnsafe':skipped,'truncated':len(rows)>=5000,'contentsExtracted':False}

    def preview_member(self,uri,member,cancel=None):
        if not safe_member(member) or member.endswith('/'):
            raise ValueError('Choose a regular archive member.')
        basename = PurePosixPath(member).name
        if Path(basename).suffix.lower() not in SAFE_VIEW_EXT:
            raise ValueError('This file type cannot be previewed safely. Use an archive manager to extract it intentionally.')
        with self.opened(uri,cancel) as archive:
            matches = [e for e in archive.infolist() if e.filename == member]
            if len(matches)!=1: raise ValueError('ZIP member is missing or duplicated. Use an archive manager.')
            item = matches[0]
            if item.flag_bits&1: raise ValueError('Encrypted ZIP members require an archive manager.')
            if item.orig_filename != item.filename or stat.S_IFMT(item.external_attr>>16) not in (0,stat.S_IFREG) or item.file_size>MAX_MEMBER or item.file_size>max(1,item.compress_size)*1000:
                raise ValueError('ZIP member is a link, too large, or exceeds the decompression safety limit.')
            if self.temp_root: self.temp_root.mkdir(parents=True,exist_ok=True,mode=0o700)
            directory = Path(tempfile.mkdtemp(prefix='winspace-zip-',dir=self.temp_root))
            os.chmod(directory,0o700)
            target = directory / basename
            try:
                total=0
                with target.open('xb') as out, archive.open(item,'r') as source:
                    os.fchmod(out.fileno(),0o600)
                    while True:
                        if cancel: cancel.check()
                        block = source.read(65536)
                        if not block: break
                        total+=len(block)
                        if total>MAX_MEMBER: raise ValueError('Decompression safety limit reached.')
                        out.write(block)
                os.chmod(target,0o400)
                return {'uri':target.as_uri(),'temporary':True,'member':member,
                        'warning':'Opened a read-only temporary copy. Changes are NOT saved back into the ZIP.'}
            except Exception:
                target.unlink(missing_ok=True); directory.rmdir(); raise
