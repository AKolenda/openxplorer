# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Server-scoped SMB credentials. No settings/database/plaintext-file fallback.

The session collection survives closing OpenXplorer but ends at logout. Permanent
remembering uses the default keyring. Call these methods off the GTK thread.
Server names are kept distinct: we NEVER resolve aliases or forward credentials
to another hostname or port based on DNS, search results or a share redirect.
"""
from __future__ import annotations
import json
import threading
from urllib.parse import urlsplit
from core import normalise_location


def server_key(uri):
    u = urlsplit(normalise_location(uri))
    if u.scheme != 'smb' or not u.hostname:
        return None
    return u.hostname.lower(), str(u.port or 445)


class SessionCredentials:
    _generations = {}
    _guard = threading.RLock()
    _io_locks = {}

    @classmethod
    def generation(cls, uri):
        with cls._guard:
            return cls._generations.get(server_key(uri), 0)

    @classmethod
    def _io_lock(cls, key):
        with cls._guard:
            return cls._io_locks.setdefault(key, threading.RLock())

    def __init__(self, secret=None):
        self.memory = {}
        self.lock = threading.RLock()
        self.secret = secret
        self.schema = None
        if secret is not None:
            self.schema = secret.Schema.new('io.winspace.SmbCredentials', secret.SchemaFlags.NONE,
                {'server': secret.SchemaAttributeType.STRING, 'port': secret.SchemaAttributeType.STRING,
                 'scope': secret.SchemaAttributeType.STRING})

    def _attrs(self, key, scope=None):
        attrs = {'server': key[0], 'port': key[1]}
        if scope: attrs['scope'] = scope
        return attrs

    def peek(self, uri):
        key = server_key(uri)
        with self.lock:
            value = self.memory.get(key)
            return dict(value) if value else None

    def load(self, uri):
        key = server_key(uri)
        generation = self.generation(uri)
        if not key: return None
        value = self.peek(uri)
        if value or self.secret is None: return value
        # Session credentials supersede an older permanently saved account.
        for scope in ('session', 'permanent'):
            encoded = self.secret.password_lookup_sync(self.schema, self._attrs(key, scope), None)
            if not encoded: continue
            try:
                if len(encoded) > 24000 or generation != self.generation(uri): continue
                value = json.loads(encoded)
                if not isinstance(value, dict) or not all(isinstance(value.get(k), str) for k in ('username', 'domain', 'password')):
                    continue
                if len(encoded) > 24000: continue
                value = {k: value[k] for k in ('username', 'domain', 'password')}
                value['remember'] = scope == 'permanent'
                with self.lock: self.memory[key] = value
                return dict(value)
            except (ValueError, TypeError):
                continue
        return None

    def accept_memory(self, uri, value):
        key = server_key(uri)
        if key:
            with self.lock: self.memory[key] = dict(value)

    def persist(self, uri, value, generation=None):
        key = server_key(uri)
        if not key: return
        if generation is None: generation = self.generation(uri)
        with self._io_lock(key):
            if generation != self.generation(uri): return
            self._persist_current(uri, value)

    def _persist_current(self, uri, value):
        key = server_key(uri)
        if self.secret is None:
            raise ValueError('The system keyring is unavailable. Credentials are reused in this window only; they cannot survive closing it.')
        permanent = value.get('remember') is True
        scope = 'permanent' if permanent else 'session'
        collection = self.secret.COLLECTION_DEFAULT if permanent else self.secret.COLLECTION_SESSION
        if not self.secret.password_store_sync(self.schema, self._attrs(key, scope), collection,
                'OpenXplorer SMB: ' + key[0], json.dumps(value), None):
            raise ValueError('The keyring did not save the credential. It remains in this window’s memory only.')
        # Read the keyring on the next challenge so signing out in another
        # OpenXplorer window also invalidates credentials here. Retain memory only
        # while saving is in flight, or if the keyring is unavailable.
        with self.lock:
            if self.memory.get(key) == value: self.memory.pop(key,None)
        # Remove stale session credentials after explicitly remembering a new account.
        if permanent:
            self.secret.password_clear_sync(self.schema, self._attrs(key, 'session'), None)

    def forget_memory(self, uri):
        key = server_key(uri)
        with self._guard:
            self._generations[key] = self._generations.get(key, 0) + 1
        with self.lock: self.memory.pop(key, None)

    def forget(self, uri, permanent=True):
        key = server_key(uri)
        self.forget_memory(uri)
        if key and self.secret:
            with self._io_lock(key):
                attrs = self._attrs(key, None if permanent else 'session')
                self.secret.password_clear_sync(self.schema, attrs, None)

    def clear_memory(self):
        with self.lock: self.memory.clear()
