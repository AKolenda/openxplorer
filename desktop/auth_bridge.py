# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Own the SMB prompt, not a credential database.

Gio.MountOperation, not Gtk.MountOperation, dispatches challenges to the trusted
local interface. GVfs stores remembered passwords through the system keyring.
Password values are never emitted back to JavaScript, settings or log messages.
"""
from __future__ import annotations
import uuid
from concurrent.futures import ThreadPoolExecutor
from session_credentials import SessionCredentials, server_key
from urllib.parse import urlsplit
from core import normalise_location


def split_identity(username: str, default_domain: str = '') -> tuple[str,str]:
    if not isinstance(username,str) or len(username)>512 or any(c in username for c in '\x00\r\n'):
        raise ValueError('Enter a valid username.')
    username=username.strip()
    # No domain control in the UI. Advanced accounts can still use DOMAIN\user.
    if '\\' in username:
        domain,username=username.split('\\',1)
        if not domain or not username or '\\' in username:
            raise ValueError('Enter a username, or use DOMAIN\\username.')
        return username,domain
    return username,default_domain or ''


class MountPrompts:
    def __init__(self, gio, glib, emit, credentials=None):
        self.Gio=gio
        self.GLib=glib
        self.emit=emit
        self.credentials = credentials or SessionCredentials()
        self.workers = ThreadPoolExecutor(max_workers=1, thread_name_prefix="winspace-keyring")
        self.closed = False
        self.pending={}
        self.operations={}

    def create(self, uri: str, cancel=None):
        uri=normalise_location(uri)
        op=self.Gio.MountOperation()
        self.operations[id(op)]={'op':op,'uri':uri,'cancel':cancel,'attempts':0,'generation':self.credentials.generation(uri)}
        op.connect('ask-password',self._ask_password)
        op.connect('ask-question',self._ask_question)
        op.connect('show-processes',self._show_processes)
        op.connect('aborted',lambda *_:self.finish(op))
        return op

    def _new(self, op, kind, data):
        # A retry supersedes only this mount's challenge, not another server's.
        self._dismiss(op)
        token=uuid.uuid4().hex
        request={'op':op,'kind':kind,**data}
        self.pending[token]=request
        request['timeout']=self.GLib.timeout_add_seconds(180,lambda:self._expire(token))
        self.emit('auth',{'id':token,'kind':kind,**{k:v for k,v in data.items() if k!='defaultDomain'}})
        return token

    def _ask_password(self, op, message, default_user, default_domain, flags):
        # Suppress Gio's default UNHANDLED reply, and never construct Gtk's
        # MountOperation (which delegates its dialog to the GNOME shell).
        op.stop_emission_by_name('ask-password')
        record=self.operations.get(id(op))
        if not record:
            op.reply(self.Gio.MountOperationResult.ABORTED);return
        if record.get('cancel') and record['cancel'].is_cancelled():
            op.reply(self.Gio.MountOperationResult.ABORTED);return
        record['attempts']+=1
        record['challenge'] = (message, default_user, default_domain, flags)
        if record['attempts'] == 1:
            cached = self.credentials.peek(record['uri'])
            if cached:
                self._reuse(op, cached)
                return
            # Secret Service may unlock a keyring: never block the GTK/UI thread.
            future = self.workers.submit(self.credentials.load, record['uri'])
            def loaded(f):
                def deliver():
                    if id(op) not in self.operations or self.closed: return False
                    try: value = f.result()
                    except Exception: value = None
                    if value: self._reuse(op, value)
                    else: self._show_password(op)
                    return False
                self.GLib.idle_add(deliver)
            future.add_done_callback(loaded)
            return
        self._show_password(op)

    def _reuse(self, op, value):
        record = self.operations.get(id(op))
        if not record or (record.get('cancel') and record['cancel'].is_cancelled()):
            op.reply(self.Gio.MountOperationResult.ABORTED)
            return
        record['candidate'] = dict(value)
        record['autoUsed'] = True
        op.set_anonymous(False)
        op.set_username(value['username']); op.set_domain(value['domain']); op.set_password(value['password'])
        op.set_password_save(self.Gio.PasswordSave.PERMANENTLY if value.get('remember') else self.Gio.PasswordSave.FOR_SESSION)
        op.reply(self.Gio.MountOperationResult.HANDLED)

    def _show_password(self, op):
        record = self.operations.get(id(op))
        if not record: return
        if self.closed or (record.get('cancel') and record['cancel'].is_cancelled()):
            op.reply(self.Gio.MountOperationResult.ABORTED); return
        message, default_user, default_domain, flags = record['challenge']
        host=urlsplit(record['uri']).hostname or ''
        F=self.Gio.AskPasswordFlags
        self._new(op,'password',{'host':host,'uri':record['uri'],
                  'username':default_user or '', 'defaultDomain':default_domain or '',
                  'needUsername':bool(flags & F.NEED_USERNAME),
                  'needPassword':bool(flags & F.NEED_PASSWORD),
                  'canSave':bool(flags & F.SAVING_SUPPORTED),
                  'canGuest':bool(flags & F.ANONYMOUS_SUPPORTED),
                  'retry':record['attempts']>1})

    def _ask_question(self, op, message, choices):
        op.stop_emission_by_name('ask-question')
        record=self.operations.get(id(op),{})
        self._new(op,'question',{'host':urlsplit(record.get('uri','')).hostname or '',
                                'message':str(message)[:4000],'choices':list(choices)[:12]})

    def _show_processes(self, op, message, processes, choices):
        op.stop_emission_by_name('show-processes')
        # Explain busy mounts. Do not kill processes or force an unmount here.
        self._new(op,'question',{'host':'','message':str(message)[:4000],
                                'choices':list(choices)[:12]})

    def answer(self, data: dict):
        password=data.pop('password','')
        token=data.get('id')
        request=self.pending.get(token)
        if not request:
            data.pop('password',None)
            raise ValueError('This sign-in request expired or was cancelled. Try connecting again.')
        op=request['op']
        if data.get('cancel'):
            self._consume(token)
            op.reply(self.Gio.MountOperationResult.ABORTED)
            return True
        if request['kind']=='password':
            if not isinstance(password,str) or len(password)>16384 or '\x00' in password:
                raise ValueError('The password is not valid.')
            guest=bool(data.get('guest')) and request['canGuest']
            username,domain=split_identity(data.get('username',''),request['defaultDomain'])
            if request['needUsername'] and not username and not guest:
                raise ValueError('Enter your username.')
            op.set_anonymous(guest)
            if not guest:
                op.set_username(username)
                op.set_domain(domain)
                op.set_password(password)
                # Default is permanent; never silently write a plaintext fallback.
                remember=data.get('remember',True) is True and request['canSave']
                op.set_password_save(self.Gio.PasswordSave.PERMANENTLY if remember else self.Gio.PasswordSave.FOR_SESSION)
                self.operations[id(op)]['candidate'] = {'username': username, 'domain': domain,
                    'password': password, 'remember': remember}
            else:
                op.set_password_save(self.Gio.PasswordSave.NEVER)
            password=None
        else:
            choice=data.get('choice')
            if not isinstance(choice,int) or isinstance(choice,bool) or not 0<=choice<len(request['choices']):
                raise ValueError('Choose one of the offered actions.')
            op.set_choice(choice)
        self._consume(token)
        op.reply(self.Gio.MountOperationResult.HANDLED)
        return True

    def _consume(self, token):
        request=self.pending.pop(token,None)
        if request:
            if request.get('timeout'):
                self.GLib.source_remove(request['timeout'])
            self.emit('authDismiss',{'id':token})
        return request

    def _dismiss(self, op):
        for token,r in list(self.pending.items()):
            if r['op'] is op:
                self._consume(token)

    def _expire(self, token):
        request=self.pending.get(token)
        if request:
            request['timeout']=None
            self._consume(token)
            request['op'].reply(self.Gio.MountOperationResult.ABORTED)
        return False

    def finish(self, op, success=False):
        self._dismiss(op)
        record = self.operations.pop(id(op), None)
        if record and success and record.get('candidate') and record['generation'] == self.credentials.generation(record['uri']):
            value = record['candidate']
            self.credentials.accept_memory(record['uri'], value)
            if not record.get('autoUsed') or record['attempts'] > 1:
                future = self.workers.submit(self.credentials.persist, record['uri'], value, record['generation'])
                def stored(f):
                    try: f.result()
                    except Exception:
                        self.emit('notice', {'message': 'Credentials work in this window, but could not be saved in the system keyring. Session reuse after closing OpenXplorer is not available until the keyring works.'})
                future.add_done_callback(stored)
        op.set_password(None)

    def close(self):
        self.closed = True
        for token in list(self.pending):
            r=self._consume(token)
            r['op'].reply(self.Gio.MountOperationResult.ABORTED)
        for rec in list(self.operations.values()):
            rec['op'].set_password(None)
        self.operations.clear()
        self.credentials.clear_memory()
        self.workers.shutdown(wait=False, cancel_futures=True)
