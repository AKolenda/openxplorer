# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Metadata index coordinator: inotify locally, incremental checks remotely.

One elected process owns the crawler/watchers for all OpenXplorer windows. SQLite
is shared; other windows send explicit scan requests through its command table.
Neither event handling nor the crawler opens file contents or requests mounts.
"""
from __future__ import annotations
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import fcntl
import json
import os
import threading
import time
from urllib.parse import urlsplit, unquote
from search_index import SearchIndex, below
from local_watch import LocalWatch
from mount_support import read_mounts, mount_for_path

NETWORK_FS = {'cifs','smb3','nfs','nfs4','sshfs','fuse.sshfs','fuse.gvfsd-fuse'}
EXCLUDED = ('/proc','/sys','/dev','/run','/tmp','/var/tmp')


class IndexService:
    def __init__(self, index: SearchIndex, list_directory, cancellation_factory, notify=lambda: None):
        self.index, self.list_directory = index, list_directory
        self.cancellation_factory, self.notify = cancellation_factory, notify
        self.executor = ThreadPoolExecutor(max_workers=1,thread_name_prefix='winspace-index')
        self.lock = threading.RLock()
        self.jobs, self.deltas, self.dirty = {}, set(), {}
        self.delta_cancels = {}
        self._enabled_before = self.enabled if hasattr(self, "enabled") else True
        self.paused_hosts, self.failed_watches = set(), {}
        self.closed = False
        self.last_requested = {}
        self.enabled = True
        self.network_interval = 60
        self.poll_state = {}
        self.started = set()
        self.force_full = set()
        self.watcher = None
        self.leader = False
        self.owner_fd = os.open(index.directory/'index-owner.lock',os.O_CREAT|os.O_RDWR,0o600)
        self.elect()

    def elect(self):
        if self.leader or self.closed: return self.leader
        try: fcntl.flock(self.owner_fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError: return False
        self.leader = True
        self.index.recover_interrupted()
        try: self.watcher = LocalWatch(self.changed,self.overflow)
        except (OSError,AttributeError): self.watcher = None
        return True

    def root_network(self, uri):
        if urlsplit(uri).scheme == 'smb': return True
        mount = mount_for_path(unquote(urlsplit(uri).path),read_mounts())
        return bool(mount and mount['fstype'] in NETWORK_FS)

    def policy(self, uri):
        if not uri.startswith('file:'): return []
        path = unquote(urlsplit(uri).path)
        # Selecting '/' means that filesystem, not other mounted filesystems.
        # Select the 2 TB volume separately, even if mounted beneath /media.
        # Whole-disk exclusions apply to descendants, not ancestors of an
        # explicitly selected root (e.g. /tmp/my-project). Otherwise its
        # initial scan and every event update would silently filter all files.
        excluded = [Path(p).as_uri() for p in EXCLUDED
                    if p.startswith(path.rstrip('/') + '/')]
        excluded.append(self.index.directory.resolve().as_uri())
        for m in read_mounts():
            target = m['path']
            if target != path and (path == '/' or target.startswith(path.rstrip('/')+'/')):
                excluded.append(Path(target).as_uri())
        return excluded

    def allowed(self, uri, root, excludes):
        if not below(uri,root): return False
        if any(below(uri,p) for p in excludes): return False
        # Don't recursively multiply the index by snapshot history.
        root_path = unquote(urlsplit(root).path).rstrip('/')
        relative = unquote(urlsplit(uri).path)[len(root_path):].strip('/')
        parts = relative.split('/')
        return not any(p in ('.snapshot','.snapshots','#snapshot','.zfs') for p in parts)

    def monitor(self, root, current, network):
        if not self.enabled or network: return
        if self.watcher:
            try: self.watcher.add(root,current)
            except OSError as exc: self.failed_watches[root] = str(exc)
        else: self.failed_watches[root] = 'inotify unavailable; using incremental checks.'

    def update_monitoring(self, root, network):
        error = self.failed_watches.get(root,'')
        mode = ('Paused' if not self.enabled else 'Incremental network checks (not push)' if network else
                'Live + timed fallback' if error else 'Live local events')
        self.index.monitoring(root,mode,self.watcher.count(root) if self.watcher else 0,error)

    def refresh(self, uri, *, explicit=True):
        if not self.leader:
            if explicit: self.index.enqueue('refresh',uri)
            return False
        with self.lock:
            if self.closed or uri in self.jobs: return False
            host = urlsplit(uri).hostname
            if host in self.paused_hosts:
                if not explicit: return False
                self.paused_hosts.discard(host)
            root = next((r for r in self.index.roots() if r['uri']==uri and r['enabled']),None)
            if not root: return False
            cancel = self.cancellation_factory()
            self.jobs[uri] = cancel
            self.last_requested[uri] = time.time()
            self.started.add(uri)
            self.executor.submit(self._run,uri,root,cancel)
            return True

    def _read(self, current, root, cancel, excludes):
        rows = []
        for batch in self.list_directory(current,bool(root['include_hidden']),cancel):
            cancel.check()
            rows.extend(e for e in batch if not e.get('symlink') and not e.get('isVirtual') and self.allowed(e['uri'],root['uri'],excludes))
            if len(rows)>1_000_000: raise ValueError('Directory exceeds the one-million-entry safety limit.')
        return rows

    def _run(self, uri, root, cancel):
        generation = None
        network = self.root_network(uri)
        try:
            generation = self.index.begin(uri)
            self.failed_watches.pop(uri,None)
            if self.watcher: self.watcher.remove(uri)
            excludes = self.policy(uri)
            self.notify()
            stack, visited = [(uri,0)], set()
            count, errors, last_emit = 0, [], time.monotonic()
            while stack:
                cancel.check()
                current,depth = stack.pop()
                if current in visited: continue
                visited.add(current)
                if depth>128:
                    errors.append('Some directories exceed the 128-level traversal limit.'); continue
                self.monitor(uri,current,network) # watch BEFORE enumerating, so changes aren't lost
                try:
                    for batch in self.list_directory(current,bool(root['include_hidden']),cancel):
                        cancel.check()
                        clean = [e for e in batch if not e.get('symlink') and not e.get('isVirtual') and self.allowed(e['uri'],uri,excludes)]
                        remaining = max(0,1_000_000-count)
                        count += self.index.put_batch(uri,generation,clean[:remaining])
                        for e in clean:
                            if e.get('isDir') and e['uri'] not in visited: stack.append((e['uri'],depth+1))
                        if len(clean)>remaining or count>=1_000_000:
                            raise ValueError('One-million-entry limit reached. Select smaller roots; additional entries were not indexed.')
                        if time.monotonic()-last_emit>.8:
                            self.update_monitoring(uri,network); self.notify(); last_emit=time.monotonic()
                except Exception as exc:
                    cancel.check(); errors.append(str(exc))
                    if current==uri or count>=1_000_000: break
            cancel.check()
            self.index.finish(uri,generation,complete=not errors,error=('Some folders could not be read. '+errors[0]) if errors else '')
        except Exception as exc:
            if generation: self.index.finish(uri,generation,complete=False,error=str(exc))
        finally:
            self.update_monitoring(uri,network)
            with self.lock:
                if self.jobs.get(uri) is cancel: self.jobs.pop(uri,None)
            self.notify()
            if self.closed: self._release()

    def changed(self, root, directory):
        if self.closed or not self.enabled or not below(directory,root): return
        if not self.leader:
            self.index.enqueue('changed',json.dumps([root,directory])); return
        with self.lock:
            if len(self.dirty)>8192:
                self.force_full.add(root)
                self.dirty = {k:v for k,v in self.dirty.items() if k[0]!=root}
            self.dirty[(root,directory)] = time.monotonic()

    def overflow(self):
        with self.lock:
            self.force_full.update(r['uri'] for r in self.index.roots() if r['enabled'])
        self.notify()

    def update(self, root, directory):
        with self.lock:
            key = (root['uri'],directory)
            if key in self.deltas or root['uri'] in self.jobs: return False
            if self.closed: return False
            self.deltas.add(key)
            self.delta_cancels[key] = self.cancellation_factory()
        self.executor.submit(self._update,root,directory,key)
        return True

    def _update(self, root, directory, key):
        cancel = self.delta_cancels[key]
        uri = root['uri']; network = self.root_network(uri)
        try:
            excludes = self.policy(uri)
            stack, seen = [directory],set()
            while stack:
                if self.closed or urlsplit(uri).hostname in self.paused_hosts: return
                cancel.check()
                current = stack.pop()
                if current in seen or not self.allowed(current,uri,excludes): continue
                seen.add(current)
                if len(seen)>10000: raise ValueError('Many new directories appeared; use Refresh for a complete scan.')
                self.monitor(uri,current,network)
                rows = self._read(current,root,cancel,excludes)
                new = self.index.replace_directory(uri,current,rows)
                stack.extend(new)
            self.update_monitoring(uri,network)
        except Exception as exc:
            # Keep the last successful data; network failures must not erase it.
            self.index.monitoring(uri,'Offline / incomplete checks',self.watcher.count(uri) if self.watcher else 0,str(exc))
        finally:
            with self.lock:
                self.deltas.discard(key)
                self.delta_cancels.pop(key,None)
            self.notify()
            if self.closed: self._release()

    def cancel(self, uri):
        if not self.leader: self.index.enqueue('cancel',uri)
        with self.lock:
            if uri in self.jobs: self.jobs[uri].cancel()
            for key,c in self.delta_cancels.items():
                if key[0]==uri: c.cancel()

    def pause_server(self, host):
        self.paused_hosts.add(host)
        if not self.leader: self.index.enqueue('pause-server',host)
        with self.lock:
            for uri,c in self.jobs.items():
                if urlsplit(uri).hostname==host: c.cancel()

    def resume_server(self, host):
        self.paused_hosts.discard(host)
        if not self.leader: self.index.enqueue('resume-server',host)

    def refresh_due(self):
        """Called once a second off the UI thread, not a 15-minute full rescan."""
        if self.closed or not self.elect(): return
        for request in self.index.drain_requests():
            if request['kind']=='refresh': self.refresh(request['uri'])
            elif request['kind']=='cancel': self.cancel(request['uri'])
            elif request['kind']=='changed': self.changed(*json.loads(request['uri']))
            elif request['kind']=='pause-server': self.pause_server(request['uri'])
            elif request['kind']=='resume-server': self.resume_server(request['uri'])
        roots = {r['uri']:r for r in self.index.roots() if r['enabled']}
        for removed in self.started-set(roots):
            self.cancel(removed)
            if self.watcher: self.watcher.remove(removed)
            self.started.discard(removed)
        for uri,root in roots.items():
            if urlsplit(uri).hostname in self.paused_hosts: continue
            # Catch changes made while OpenXplorer was closed with one initial scan.
            if self.enabled and uri not in self.started: self.refresh(uri,explicit=False)
        if self.enabled != self._enabled_before:
            for uri in roots:
                if not self.enabled and self.watcher: self.watcher.remove(uri)
                self.update_monitoring(uri,self.root_network(uri))
            if self.enabled: self.started.clear()
            self._enabled_before = self.enabled
        if not self.enabled: return
        with self.lock:
            dirty = [(k,t) for k,t in self.dirty.items() if time.monotonic()-t>.35]
            full = list(self.force_full)
        for uri in full:
            if self.refresh(uri,explicit=False):
                with self.lock: self.force_full.discard(uri)
        for (uri,directory),stamp in dirty[:64]:
            if uri in roots and urlsplit(uri).hostname not in self.paused_hosts and self.update(roots[uri],directory):
                with self.lock:
                    if self.dirty.get((uri,directory)) == stamp: self.dirty.pop((uri,directory),None)
        budget = 4
        for uri,root in roots.items():
            if budget<=0: break
            if uri in self.jobs or urlsplit(uri).hostname in self.paused_hosts: continue
            if not self.root_network(uri) and uri not in self.failed_watches: continue
            state = self.poll_state.setdefault(uri,{'after':None,'next':time.monotonic()+self.network_interval})
            if time.monotonic()<state['next']: continue
            if state['after'] is None:
                directories = [uri]; state['after']=''
            else:
                directories = self.index.directories(uri,state['after'],budget)
                if not directories:
                    state['after']=None; state['next']=time.monotonic()+self.network_interval; continue
                state['after']=directories[-1]
            for directory in directories:
                self.update(root,directory); budget-=1
            # Small batches avoid hammering a NAS. A large tree takes longer than
            # the interval; the UI reports this as polling, NEVER push coverage.
            state['next']=time.monotonic()+2

    def _release(self):
        with self.lock:
            if not self.jobs and not self.deltas and self.owner_fd is not None:
                os.close(self.owner_fd); self.owner_fd=None; self.leader=False

    def close(self):
        with self.lock:
            self.closed=True
            for c in list(self.jobs.values())+list(self.delta_cancels.values()): c.cancel()
        if self.watcher: self.watcher.close(); self.watcher=None
        self.executor.shutdown(wait=False,cancel_futures=False)
        self._release()
