# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-11; original notices: licenses/Winspace-MIT.txt.
"""GTK3 file clipboard, including across OpenXplorer processes.

GTK's set_with_data is not introspectable; the small C-ABI adapter below calls
that official GTK function with retained callbacks. All calls run on GTK's main
thread. The transfer engine still validates every URI and confirms each paste.
"""
from __future__ import annotations
import ctypes as C
import ctypes.util
import hashlib
import json
import uuid
from core import require_item_uri

CUSTOM = 'application/x-winspace-files'
GNOME = 'x-special/gnome-copied-files'
URI_LIST = 'text/uri-list'
KDE_CUT = 'x-kde-cutselection'
MAX_BYTES = 1024 * 1024


def validate_clipboard(value):
    if not isinstance(value, dict) or value.get('mode') not in ('copy', 'move'):
        raise ValueError('Invalid file clipboard operation.')
    values = value.get('uris')
    if not isinstance(values, list) or not 1 <= len(values) <= 200:
        raise ValueError('Copy or cut between 1 and 200 items at a time.')
    uris = list(dict.fromkeys(require_item_uri(u) for u in values))
    token = value.get('token')
    if not isinstance(token, str) or len(token) > 80:
        token = uuid.uuid4().hex
    return {'mode': value['mode'], 'uris': uris, 'token': token}


def encode_clipboard(value):
    value = validate_clipboard(value)
    return {CUSTOM: json.dumps(value).encode('utf-8'),
            GNOME: (('cut' if value['mode'] == 'move' else 'copy') + '\n' + '\n'.join(value['uris'])).encode('utf-8'),
            URI_LIST: ('\r\n'.join(value['uris']) + '\r\n').encode('utf-8'),
            KDE_CUT: b'1' if value['mode'] == 'move' else b'0'}


def decode_clipboard(mime, payload, cut_selection=None):
    if not payload or len(payload) > MAX_BYTES: return None
    try:
        text = bytes(payload).decode('utf-8').rstrip('\x00')
        if mime == CUSTOM:
            return validate_clipboard(json.loads(text))
        lines = text.splitlines()
        if mime == GNOME:
            if not lines or lines[0] not in ('cut', 'copy'): return None
            value = {'mode': 'move' if lines[0] == 'cut' else 'copy', 'uris': lines[1:]}
        elif mime == URI_LIST:
            # KDE keeps the copy/cut flag separate from its URI list. Only the
            # exact cut marker grants move semantics; arbitrary text does not.
            move = cut_selection is not None and bytes(cut_selection).rstrip(b'\x00') == b'1'
            value = {'mode': 'move' if move else 'copy', 'uris': [u for u in lines if u and not u.startswith('#')]}
        else:
            return None
        # External clipboard formats have no OpenXplorer token. Re-reading the
        # same payload must keep its identity so successful cut items can be
        # consumed, while a changed payload must not be cleared by an old paste.
        fingerprint = mime.encode('utf-8') + b'\x00' + bytes(payload) + b'\x00' + value['mode'].encode('ascii')
        value['token'] = 'external-' + hashlib.sha256(fingerprint).hexdigest()
        return validate_clipboard(value)
    except (ValueError, TypeError, UnicodeError, KeyError):
        return None


class Target(C.Structure):
    _fields_ = [('target', C.c_char_p), ('flags', C.c_uint), ('info', C.c_uint)]

GET = C.CFUNCTYPE(None, C.c_void_p, C.c_void_p, C.c_uint, C.c_void_p)
CLEAR = C.CFUNCTYPE(None, C.c_void_p, C.c_void_p)


class FileClipboard:
    def __init__(self, Gtk, Gdk, changed=lambda: None):
        self.Gtk, self.Gdk = Gtk, Gdk
        self.clipboard = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
        self._generation = 0
        def owner_changed(*_):
            self._generation += 1
            changed()
        self.clipboard.connect('owner-change', owner_changed)
        self._gtk = C.CDLL(ctypes.util.find_library('gtk-3') or 'libgtk-3.so.0')
        self._gdk = C.CDLL(ctypes.util.find_library('gdk-3') or 'libgdk-3.so.0')
        self._gdk.gdk_atom_intern.argtypes = [C.c_char_p, C.c_int]
        self._gdk.gdk_atom_intern.restype = C.c_void_p
        self._gtk.gtk_clipboard_get.argtypes = [C.c_void_p]
        self._gtk.gtk_clipboard_get.restype = C.c_void_p
        self._gtk.gtk_clipboard_set_with_data.argtypes = [C.c_void_p, C.POINTER(Target), C.c_uint, GET, CLEAR, C.c_void_p]
        self._gtk.gtk_clipboard_set_with_data.restype = C.c_int
        self._gtk.gtk_selection_data_set.argtypes = [C.c_void_p, C.c_void_p, C.c_int, C.c_void_p, C.c_int]
        self._gtk.gtk_clipboard_set_can_store.argtypes = [C.c_void_p, C.POINTER(Target), C.c_int]
        self._ptr = self._gtk.gtk_clipboard_get(self._atom('CLIPBOARD'))
        self._payloads = []
        self._get_callback = GET(self._get)
        self._clear_callback = CLEAR(lambda *_: None)
        self._targets = None

    def _atom(self, name):
        return self._gdk.gdk_atom_intern(name.encode(), 0)

    def _get(self, _clipboard, selection, info, _data):
        try:
            mime, payload = self._payloads[info]
            buffer = C.create_string_buffer(payload)
            self._gtk.gtk_selection_data_set(selection, self._atom(mime), 8, C.cast(buffer, C.c_void_p), len(payload))
        except Exception:
            # Exceptions cannot propagate through a C callback.
            return

    def set(self, value):
        value = validate_clipboard(value)
        payloads = list(encode_clipboard(value).items())
        targets = (Target * len(payloads))(*[Target(mime.encode(), 0, i) for i, (mime, _) in enumerate(payloads)])
        if not self._gtk.gtk_clipboard_set_with_data(self._ptr, targets, len(targets), self._get_callback, self._clear_callback, None):
            raise ValueError('The desktop clipboard could not be claimed.')
        self._payloads, self._targets = payloads, targets
        self._gtk.gtk_clipboard_set_can_store(self._ptr, targets, len(targets))
        return value

    def read(self, callback):
        """Read asynchronously; text copied in another app is never a file list."""
        choices = [CUSTOM, GNOME, URI_LIST]
        generation = self._generation
        def next_format():
            if not choices: callback(None); return
            mime = choices.pop(0)
            def received(_clip, selection, *_):
                if generation != self._generation: callback(None); return
                payload = selection.get_data()
                value = decode_clipboard(mime, payload)
                if value and mime == URI_LIST:
                    def marker_received(_clip, marker, *_):
                        if generation != self._generation: callback(None); return
                        data = marker.get_data()
                        # A missing or malformed marker remains an ordinary
                        # copy. Never use a marker from a different owner.
                        if data is not None and len(data) > 16: data = None
                        callback(decode_clipboard(URI_LIST, payload, data))
                    self.clipboard.request_contents(self.Gdk.Atom.intern(KDE_CUT, False), marker_received, None)
                elif value: callback(value)
                else: next_format()
            self.clipboard.request_contents(self.Gdk.Atom.intern(mime, False), received, None)
        next_format()

    def consume(self, token, done, callback=lambda value: None):
        def read_back(value):
            if value and value['mode'] == 'move' and value.get('token') == token:
                value['uris'] = [u for u in value['uris'] if u not in done]
                if value['uris']: self.set(value)
                else: self.clipboard.set_text('',0); value = None
            callback(value)
        self.read(read_back)
