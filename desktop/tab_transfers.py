# SPDX-License-Identifier: AGPL-3.0-only
"""Process-local, single-use tab transfers. Never transfer credentials or files.

Only a destination acknowledgement retires the source tab. All failure paths
keep it, remove a tentative destination tab, and discard the capability token.
"""
from __future__ import annotations
from dataclasses import dataclass
import secrets
import time
from window_state import tab_snapshot


@dataclass
class Transfer:
    source: int
    tab_id: str
    snapshot: dict
    expires: float
    destination: int | None = None


class TabTransfers:
    def __init__(self, send, available, clock=time.monotonic, ttl=30):
        self.send, self.available, self.clock, self.ttl = send, available, clock, ttl
        self.pending: dict[str, Transfer] = {}

    def offer(self, source, tab_id, snapshot):
        self.expire()
        if type(source) is not int or not self.available(source):
            raise ValueError('The source window is no longer ready.')
        if not isinstance(tab_id, str) or not tab_id or len(tab_id) > 80:
            raise ValueError('Invalid tab identifier.')
        if len(self.pending) >= 64 or any(p.source == source and p.tab_id == tab_id for p in self.pending.values()):
            raise ValueError('That tab is already moving. Wait for it to finish.')
        token = secrets.token_hex(32)
        self.pending[token] = Transfer(source, tab_id, tab_snapshot(snapshot), self.clock() + self.ttl)
        return token

    def claim(self, token, destination, before=None):
        self.expire()
        p = self.pending.get(token) if isinstance(token, str) else None
        if p is None or p.destination is not None:
            raise ValueError('The tab move has expired or was already accepted. The original tab was kept.')
        if type(destination) is not int or destination == p.source or not self.available(destination):
            raise ValueError('Choose a different, ready OpenXplorer window.')
        if before is not None and (not isinstance(before, str) or len(before) > 80):
            raise ValueError('Invalid tab position.')
        p.destination = destination
        p.expires = self.clock() + self.ttl
        try:
            self.send(destination, 'tabReceive', {'token': token, 'tab': p.snapshot, 'beforeId': before})
        except Exception:
            self.cancel(token, 'The destination could not receive the tab.')
            raise
        return True

    def ready(self, token, destination, accepted):
        self.expire()
        p = self.pending.get(token) if isinstance(token, str) else None
        if p is None or p.destination != destination:
            return {'committed': False}
        if accepted is not True or not self.available(p.source) or not self.available(destination):
            self.cancel(token, 'The destination was busy or closed. The original tab was kept.')
            return {'committed': False}
        del self.pending[token]
        # Destination state is already restored, not a promise to restore later.
        self.send(destination, 'tabTransferSettled', {'token': token, 'committed': True})
        self.send(p.source, 'tabTransferDone', {'token': token, 'tabId': p.tab_id, 'committed': True})
        return {'committed': True}

    def cancel(self, token, message='Tab move cancelled. The original tab was kept.'):
        p = self.pending.pop(token, None)
        if p is None:
            return False
        if p.destination is not None:
            self.send(p.destination, 'tabTransferSettled', {'token': token, 'committed': False})
        self.send(p.source, 'tabTransferDone', {'token': token, 'tabId': p.tab_id, 'committed': False, 'message': message})
        return True

    def expire(self):
        for token, p in list(self.pending.items()):
            if p.expires <= self.clock():
                self.cancel(token, 'The tab move timed out. The original tab was kept.')

    def window_closed(self, identifier):
        for token, p in list(self.pending.items()):
            if identifier in (p.source, p.destination):
                self.cancel(token, 'A window closed before the tab move finished.')

    def busy(self, identifier):
        return any(identifier in (p.source, p.destination) for p in self.pending.values())
