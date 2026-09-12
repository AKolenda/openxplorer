# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Private, metadata-only SQLite search. No filesystem or network traversal here.

Each root is an explicit opt-in. Scans use generations: only a fully successful
scan may prune missing records. A cancelled/offline scan preserves old results.
Readers have separate WAL connections, so SMB latency cannot block a query.
"""
from __future__ import annotations
from contextlib import contextmanager
import json
import os
from pathlib import Path
import sqlite3
import threading
import time
import unicodedata
import uuid
from urllib.parse import unquote, urlsplit
from core import normalise_location, is_smb_server
from private_storage import private_directory, private_file, validate_sqlite_files


def display_path(uri: str) -> str:
    u = urlsplit(uri)
    return ('\\\\' + u.netloc + unquote(u.path).replace('/', '\\')) if u.scheme == 'smb' else unquote(u.path)


def below(uri: str, root: str) -> bool:
    return uri == root or uri.startswith(root.rstrip('/') + '/')


def fold(value: str) -> str:
    return unicodedata.normalize('NFKC', value).casefold()


class SearchIndex:
    def __init__(self, directory: Path | None = None, *, recover: bool = True):
        self.directory = directory or Path(os.environ.get('XDG_CACHE_HOME', Path.home()/'.cache'))/'winspace'
        private_directory(self.directory)
        self.path = self.directory/'search.sqlite3'
        self.lock = threading.RLock()
        # SQLite creates the sidecars in this private directory. Pre-create the
        # database with 0600; paths can reveal private filenames even without content.
        fd = private_file(self.path, create=True, writable=True)
        os.close(fd)
        with self.connect() as db:
            db.execute('PRAGMA journal_mode=WAL')
            db.executescript('''
                CREATE TABLE IF NOT EXISTS roots(
                    uri TEXT PRIMARY KEY, label TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
                    status TEXT NOT NULL DEFAULT 'Not indexed', updated REAL NOT NULL DEFAULT 0,
                    scanned INTEGER NOT NULL DEFAULT 0, error TEXT NOT NULL DEFAULT '',
                    generation TEXT NOT NULL DEFAULT '', include_hidden INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE IF NOT EXISTS entries(
                    id INTEGER PRIMARY KEY, root TEXT NOT NULL, uri TEXT NOT NULL,
                    parent TEXT NOT NULL, name TEXT NOT NULL, search_text TEXT NOT NULL,
                    is_dir INTEGER NOT NULL, hidden INTEGER NOT NULL, size INTEGER,
                    modified REAL NOT NULL, kind TEXT NOT NULL, type TEXT NOT NULL,
                    generation TEXT NOT NULL, seen REAL NOT NULL,
                    UNIQUE(root,uri));
                CREATE INDEX IF NOT EXISTS entries_root_uri ON entries(root,uri);
                CREATE INDEX IF NOT EXISTS entries_parent ON entries(parent);
            ''')
            existing = {r[1] for r in db.execute('PRAGMA table_info(roots)')}
            for column, declaration in [('update_mode', "TEXT NOT NULL DEFAULT 'Not watching'"),
                    ('watch_count', 'INTEGER NOT NULL DEFAULT 0'), ('watch_error', "TEXT NOT NULL DEFAULT ''"),
                    ('last_event', 'REAL NOT NULL DEFAULT 0')]:
                if column not in existing:
                    db.execute('ALTER TABLE roots ADD COLUMN ' + column + ' ' + declaration)
            db.execute('CREATE TABLE IF NOT EXISTS index_requests (key TEXT PRIMARY KEY, kind TEXT, uri TEXT, created REAL)')
            try:
                db.execute("CREATE VIRTUAL TABLE IF NOT EXISTS names_fts USING fts5(search_text,content='entries',content_rowid='id',tokenize='trigram')")
                db.executescript('''
                    CREATE TRIGGER IF NOT EXISTS names_insert AFTER INSERT ON entries BEGIN
                        INSERT INTO names_fts(rowid,search_text) VALUES(new.id,new.search_text); END;
                    CREATE TRIGGER IF NOT EXISTS names_delete AFTER DELETE ON entries BEGIN
                        INSERT INTO names_fts(names_fts,rowid,search_text) VALUES('delete',old.id,old.search_text); END;
                    CREATE TRIGGER IF NOT EXISTS names_update AFTER UPDATE OF search_text ON entries WHEN old.search_text<>new.search_text BEGIN
                        INSERT INTO names_fts(names_fts,rowid,search_text) VALUES('delete',old.id,old.search_text);
                        INSERT INTO names_fts(rowid,search_text) VALUES(new.id,new.search_text); END;
                ''')
                self.fts = True
            except sqlite3.OperationalError:
                self.fts = False
            # An interrupted application must not look as though it is still scanning.
            if recover:
                db.execute("UPDATE roots SET status='Interrupted',error='Refresh to finish the interrupted scan.' WHERE status IN ('Indexing','Queued')")

    def recover_interrupted(self):
        """Only the primary GApplication may recover unfinished scan status.
        A secondary process opening a folder must not mark a live scan interrupted.
        """
        with self.connect() as db:
            db.execute("UPDATE roots SET status='Interrupted',error='Refresh to finish the interrupted scan.' WHERE status IN ('Indexing','Queued')")

    @contextmanager
    def connect(self):
        private_directory(self.directory)
        validate_sqlite_files(self.path)
        db = sqlite3.connect(self.path, timeout=8)
        db.row_factory = sqlite3.Row
        db.execute('PRAGMA busy_timeout=8000')
        try:
            with db:
                yield db
        finally:
            db.close()

    def roots(self) -> list[dict]:
        with self.connect() as db:
            return [dict(r) for r in db.execute('''SELECT r.*,
                (SELECT count(*) FROM entries e WHERE e.root=r.uri) AS count
                FROM roots r ORDER BY r.label COLLATE NOCASE''')]

    def snapshot(self) -> dict:
        roots = self.roots()
        return {'roots': roots, 'count': sum(r['count'] for r in roots if r['enabled']),
                'engine': 'SQLite FTS5 trigram' if self.fts else 'SQLite substring fallback',
                'database': str(self.path), 'metadataOnly': True}

    def configure(self, uri: str, enabled: bool, label: str = '', include_hidden: bool = False) -> dict:
        uri = normalise_location(uri)
        if is_smb_server(uri):
            raise ValueError('Open a share first. Cache a shared folder, not the server’s share list.')
        if not isinstance(enabled, bool) or not isinstance(include_hidden, bool):
            raise ValueError('Cache settings must be true or false.')
        label = (str(label).strip() or display_path(uri))[:200]
        with self.lock, self.connect() as db:
            db.execute('''INSERT INTO roots(uri,label,enabled,include_hidden) VALUES(?,?,?,?)
                ON CONFLICT(uri) DO UPDATE SET enabled=excluded.enabled,label=excluded.label,
                include_hidden=excluded.include_hidden''', (uri,label,int(enabled),int(include_hidden)))
            if not enabled:
                db.execute('DELETE FROM entries WHERE root=?', (uri,))
                db.execute("UPDATE roots SET status='Disabled',scanned=0,error='',generation='' WHERE uri=?", (uri,))
        return self.snapshot()

    def begin(self, uri: str) -> str:
        generation = uuid.uuid4().hex
        with self.lock, self.connect() as db:
            result = db.execute("UPDATE roots SET generation=?,status='Indexing',scanned=0,error='' WHERE uri=? AND enabled=1", (generation,uri))
            if result.rowcount != 1:
                raise ValueError('This folder is not enabled for caching.')
        return generation

    def put_batch(self, root: str, generation: str, entries: list[dict]) -> int:
        values=[]
        now=time.time()
        for e in entries:
            uri=normalise_location(e.get('targetUri') or e['uri'])
            if not below(uri,root) or uri == root:
                continue
            # Do not let symlinks or virtual targets point outside an opted-in tree.
            if e.get('symlink') or e.get('isVirtual'):
                continue
            name=str(e['name'])[:4096]
            parent=uri.rsplit('/',1)[0]
            if parent=='file://':
                parent='file:///'
            values.append((root,uri,parent,name,fold(name+' '+display_path(parent)),
                           int(bool(e.get('isDir'))),int(bool(e.get('hidden'))),e.get('size'),
                           float(e.get('modified') or 0),str(e.get('kind') or ('directory' if e.get('isDir') else 'file')),
                           str(e.get('type') or 'File'),generation,now))
        with self.lock, self.connect() as db:
            current=db.execute('SELECT generation,enabled FROM roots WHERE uri=?',(root,)).fetchone()
            if not current or not current['enabled'] or current['generation'] != generation:
                return 0
            db.executemany('''INSERT INTO entries(root,uri,parent,name,search_text,is_dir,hidden,size,modified,kind,type,generation,seen)
                VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(root,uri) DO UPDATE SET
                parent=excluded.parent,name=excluded.name,search_text=excluded.search_text,
                is_dir=excluded.is_dir,hidden=excluded.hidden,size=excluded.size,modified=excluded.modified,
                kind=excluded.kind,type=excluded.type,generation=excluded.generation,seen=excluded.seen''',values)
            db.execute('UPDATE roots SET scanned=scanned+? WHERE uri=?',(len(values),root))
        return len(values)

    def finish(self, root: str, generation: str, *, complete: bool, error: str = '') -> None:
        with self.lock, self.connect() as db:
            r=db.execute('SELECT generation,enabled FROM roots WHERE uri=?',(root,)).fetchone()
            if not r or not r['enabled'] or r['generation']!=generation:
                return
            if complete:
                db.execute('DELETE FROM entries WHERE root=? AND generation<>?',(root,generation))
                db.execute("UPDATE roots SET updated=?,status='Ready',error='' WHERE uri=?",(time.time(),root))
            else:
                db.execute("UPDATE roots SET status='Incomplete / offline',error=? WHERE uri=?",(error[:500],root))

    def clear(self, uri: str) -> None:
        uri=normalise_location(uri)
        with self.lock, self.connect() as db:
            db.execute('DELETE FROM entries WHERE root=?',(uri,))
            db.execute("UPDATE roots SET status='Not indexed',updated=0,scanned=0,generation='',error='' WHERE uri=?",(uri,))

    def remove(self, uri: str) -> None:
        uri=normalise_location(uri)
        with self.lock, self.connect() as db:
            db.execute('DELETE FROM entries WHERE root=?',(uri,))
            db.execute('DELETE FROM roots WHERE uri=?',(uri,))

    def search(self, text: str, scope: str | None = None, limit: int = 500, show_hidden: bool = False, cancel=None) -> dict:
        if cancel:
            cancel.check()
        if not isinstance(text,str) or len(text)>512:
            raise ValueError('Search must be at most 512 characters.')
        scope=normalise_location(scope) if scope else None
        limit=max(1,min(int(limit),2000))
        terms=list(dict.fromkeys(fold(text).split()))[:20]
        started=time.perf_counter()
        if not terms:
            return {'entries':[], 'truncated':False,'elapsedMs':0,'source':'cache'}
        clauses=['r.enabled=1'];args=[]
        long=[t for t in terms if len(t)>=3]
        if self.fts and long:
            # Bind data, and quote every term: a filename is never FTS query syntax.
            expr=' AND '.join('"'+t.replace('"','""')+'"' for t in long)
            clauses.append('e.id IN (SELECT rowid FROM names_fts WHERE names_fts MATCH ?)')
            args.append(expr)
        for term in terms:
            clauses.append('instr(e.search_text,?)>0');args.append(term)
        if scope:
            clauses.append('(e.uri=? OR substr(e.uri,1,?)=?)')
            prefix=scope.rstrip('/')+'/'
            args.extend([scope,len(prefix),prefix])
        if not show_hidden:
            clauses.append('e.hidden=0')
        sql='''SELECT e.*,r.updated,r.status FROM entries e JOIN roots r ON r.uri=e.root
            WHERE '''+' AND '.join(clauses)+''' GROUP BY e.uri
            ORDER BY e.is_dir DESC,e.name COLLATE NOCASE,e.uri LIMIT ?'''
        args.append(limit+1)
        with self.connect() as db:
            if cancel:
                db.set_progress_handler(lambda: int(cancel.is_cancelled()), 4000)
            try:
                rows=db.execute(sql,args).fetchall()
            except sqlite3.OperationalError:
                if cancel:
                    cancel.check()
                raise
        result=[]
        for r in rows[:limit]:
            result.append({'uri':r['uri'],'parentUri':r['parent'],'name':r['name'],'isDir':r['kind']=='directory' or (r['kind'] not in ('file','special','symlink') and bool(r['is_dir'])),
                           'size':r['size'],'modified':r['modified'],'type':r['type'],'kind':r['kind'],
                           'hidden':bool(r['hidden']),'isVirtual':False,'canOperate':True,
                           'path':display_path(r['uri']), 'cached':True,'cachedAt':r['seen'],
                           'rootUpdated':r['updated'],'cacheStatus':r['status']})
        return {'entries':result,'truncated':len(rows)>limit,'limit':limit,
                'elapsedMs':round((time.perf_counter()-started)*1000,2),'source':'cache'}

    def monitoring(self, root, mode, count=0, error=''):
        with self.connect() as db:
            db.execute('UPDATE roots SET update_mode=?,watch_count=?,watch_error=? WHERE uri=?',
                       (mode,int(count),str(error)[:500],root))

    def enqueue(self, kind, uri):
        with self.connect() as db:
            db.execute('INSERT OR REPLACE INTO index_requests(key,kind,uri,created) VALUES(?,?,?,?)',
                       (kind+':'+uri,kind,uri,time.time()))

    def drain_requests(self):
        with self.connect() as db:
            rows = [dict(r) for r in db.execute('SELECT * FROM index_requests ORDER BY created')]
            db.execute('DELETE FROM index_requests')
            return rows

    def directories(self, root, after='', limit=4):
        with self.connect() as db:
            return [r[0] for r in db.execute('SELECT uri FROM entries WHERE root=? AND is_dir=1 AND uri>? ORDER BY uri LIMIT ?', (root,after,limit))]

    def child_directories(self, root, parent):
        with self.connect() as db:
            return {r[0] for r in db.execute('SELECT uri FROM entries WHERE root=? AND parent=? AND is_dir=1',(root,parent))}

    def replace_directory(self, root, parent, entries):
        """Atomic direct-child reconciliation; NEVER prune after a failed read.

        New subtrees are returned to the crawler. A deleted/renamed directory
        removes its stale descendants from the index, never from the filesystem.
        """
        now = time.time()
        with self.lock, self.connect() as db:
            state = db.execute('SELECT generation,enabled FROM roots WHERE uri=?',(root,)).fetchone()
            if not state or not state['enabled']: return []
            old = {r['uri']: bool(r['is_dir']) for r in db.execute('SELECT uri,is_dir FROM entries WHERE root=? AND parent=?',(root,parent))}
            clean = {}
            for entry in entries:
                uri = normalise_location(entry['uri'])
                if below(uri,root) and uri != root and not entry.get('symlink') and not entry.get('isVirtual'):
                    actual_parent = uri.rsplit('/',1)[0]
                    if actual_parent == 'file://': actual_parent = 'file:///'
                    if actual_parent.rstrip('/') == parent.rstrip('/'): clean[uri] = entry
            for uri, was_dir in old.items():
                # If a directory becomes a file, its indexed children are stale.
                if uri not in clean or (was_dir and not clean[uri].get('isDir')):
                    prefix = uri.rstrip('/')+'/'
                    db.execute('DELETE FROM entries WHERE root=? AND (uri=? OR substr(uri,1,?)=?)', (root,uri,len(prefix),prefix))
            values = []
            for uri,e in clean.items():
                name = str(e['name'])[:4096]
                values.append((root,uri,parent,name,fold(name+' '+display_path(parent)),int(e.get('isDir') is True),
                    int(bool(e.get('hidden'))),e.get('size'),float(e.get('modified') or 0),
                    e.get('kind') or ('directory' if e.get('isDir') else 'file'),e.get('type') or 'File',state['generation'],now))
            db.executemany("""INSERT INTO entries(root,uri,parent,name,search_text,is_dir,hidden,size,modified,kind,type,generation,seen)
                VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(root,uri) DO UPDATE SET
                parent=excluded.parent,name=excluded.name,search_text=excluded.search_text,is_dir=excluded.is_dir,
                hidden=excluded.hidden,size=excluded.size,modified=excluded.modified,kind=excluded.kind,type=excluded.type,
                generation=excluded.generation,seen=excluded.seen""",values)
            db.execute("UPDATE roots SET last_event=? WHERE uri=?",(now,root))
            return [uri for uri,e in clean.items() if e.get('isDir') and not old.get(uri)]
