# SPDX-License-Identifier: AGPL-3.0-only
"""Native file drop receiver. A drop proposes a copy; the UI confirms the write.

Only explicit filesystem URIs are accepted. GTK is never asked to delete the
source, including when another file manager suggests a move. The existing GIO
operation/pin handlers validate destinations and metadata before any writes.
"""
from __future__ import annotations
import math
from urllib.parse import urlsplit

from core import normalise_location, require_item_uri, is_smb_server
from file_clipboard import MAX_BYTES, URI_LIST

INFO = 0x5858


def drop_layout(data):
    def number(value):
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 <= value <= 100000:
            raise ValueError('Invalid file drop coordinates.')
        return float(value)
    if not isinstance(data, dict):
        raise ValueError('Invalid file drop layout.')
    width, height = number(data.get('width')), number(data.get('height'))
    if width < 1 or height < 1:
        raise ValueError('Invalid file drop dimensions.')
    rows = data.get('targets', [])
    if not isinstance(rows, list) or len(rows) > 4000:
        raise ValueError('Too many file drop targets.')
    targets = []
    for row in rows:
        if not isinstance(row, dict) or row.get('kind') not in ('copy', 'pin'):
            raise ValueError('Invalid file drop target.')
        rect = {key: number(row.get(key)) for key in ('left', 'right', 'top', 'bottom')}
        if not 0 <= rect['left'] < rect['right'] <= width or not 0 <= rect['top'] < rect['bottom'] <= height:
            raise ValueError('File drop target is outside the view.')
        target = {**rect, 'kind': row['kind']}
        if row['kind'] == 'copy':
            uri = normalise_location(row.get('uri'))
            if is_smb_server(uri):
                raise ValueError('Open a share before dropping files.')
            target['uri'] = uri
        elif row.get('before') is not None:
            target['before'] = normalise_location(row['before'])
        targets.append(target)
    return {'width': width, 'height': height, 'targets': targets}


def decode_uris(payload):
    if not isinstance(payload, (bytes, bytearray)) or not payload or len(payload) > MAX_BYTES:
        raise ValueError('The file drop is empty or too large.')
    text = bytes(payload).decode('utf-8').rstrip('\x00')
    rows = [line for line in text.splitlines() if line and not line.startswith('#')]
    if not 1 <= len(rows) <= 200:
        raise ValueError('Drop between 1 and 200 files or folders.')
    if any(urlsplit(uri).scheme.lower() not in ('file', 'smb') for uri in rows):
        raise ValueError('Drop files or folders, rather than links or text.')
    return list(dict.fromkeys(normalise_location(uri) for uri in rows))


class NativeFileDrop:
    def __init__(self, controller, Gtk, Gdk, GLib):
        self.c, self.Gtk, self.Gdk, self.GLib = controller, Gtk, Gdk, GLib
        self.view = controller.webview
        self.layout = None
        self.pending = None
        self.timer = None
        self.atom = Gdk.Atom.intern(URI_LIST, False)
        # Disable WebKit's automatic drop handling: our handlers finish each
        # accepted file handshake, while NativeTabDrag retains its own targets.
        targets = self.view.drag_dest_get_target_list() or Gtk.TargetList.new([])
        # TargetList.add permits duplicates and lookup returns the first one.
        # Replace WebKit's URI entry so data-received uses our handler's info.
        targets.remove(self.atom)
        targets.add(self.atom, 0, INFO)
        self.view.drag_dest_set(0, [], Gdk.DragAction.COPY | Gdk.DragAction.MOVE)
        self.view.drag_dest_set_target_list(targets)
        self.handlers = [self.view.connect(name, fn) for name, fn in (
            ('drag-motion', self.motion), ('drag-leave', self.leave),
            ('drag-drop', self.drop), ('drag-data-received', self.received))]

    def update(self, data):
        self.layout = drop_layout(data)

    def accepts(self, context):
        return URI_LIST in [atom.name() for atom in context.list_targets()]

    def target(self, x, y):
        if not self.layout or self.c.closed or not self.c.ui_ready or self.c.writes:
            return None
        width, height = self.view.get_allocated_width(), self.view.get_allocated_height()
        if width <= 0 or height <= 0:
            return None
        x, y = x * self.layout['width'] / width, y * self.layout['height'] / height
        for target in self.layout['targets']:
            if target['left'] <= x < target['right'] and target['top'] <= y < target['bottom']:
                return target
        return None

    def motion(self, view, context, x, y, time):
        if not self.accepts(context):
            return False
        view.stop_emission_by_name('drag-motion')
        target = self.target(x, y) if context.get_actions() & self.Gdk.DragAction.COPY else None
        self.Gdk.drag_status(context, self.Gdk.DragAction.COPY if target else self.Gdk.DragAction(0), time)
        self.c.emit('fileDropHint', {'show': bool(target), **(target or {})})
        return True

    def leave(self, view, context, time):
        if self.accepts(context):
            view.stop_emission_by_name('drag-leave')
            self.c.emit('fileDropHint', {'show': False})

    def drop(self, view, context, x, y, time):
        if not self.accepts(context):
            return False
        view.stop_emission_by_name('drag-drop')
        target = self.target(x, y) if context.get_actions() & self.Gdk.DragAction.COPY else None
        if target is None or self.pending:
            self.Gtk.drag_finish(context, False, False, time)
            return True
        self.pending = (context, target, time)
        self.timer = self.GLib.timeout_add_seconds(10, self.expire)
        view.drag_get_data(context, self.atom, time)
        return True

    def received(self, view, context, x, y, data, info, time):
        if info != INFO or not self.pending or self.pending[0] != context:
            return
        view.stop_emission_by_name('drag-data-received')
        _, target, timestamp = self.pending
        self.clear_pending()
        event = None
        try:
            if self.c.closed or not self.c.ui_ready or self.c.writes:
                raise ValueError('Finish the current operation before dropping files.')
            if not self.layout or target not in self.layout['targets']:
                raise ValueError('The destination changed. Drop the files again.')
            uris = decode_uris(data.get_data())
            # The same-process source retains canonical remote URIs even when
            # the external payload uses a local GVfs path for editor support.
            widget = self.Gtk.drag_get_source_widget(context)
            for controller in self.c.app.controllers:
                source = getattr(controller, 'file_drag', None)
                files = getattr(source, 'files', None)
                if (controller.webview == widget and source and source.context is not None and files
                        and list(dict.fromkeys(files.exported)) == uris):
                    uris = list(source.uris)
                    break
            if target['kind'] == 'copy':
                uris = [require_item_uri(uri) for uri in uris]
                self.c.previous_versions.assert_writable(target['uri'])
            event = {'uris': uris, 'kind': target['kind'], 'target': target.get('uri'), 'before': target.get('before')}
        except Exception as exc:
            self.c.emit('notice', {'message': str(exc)})
        # Finish the native grab before a confirmation dialog can be opened.
        self.Gtk.drag_finish(context, event is not None, False, timestamp)
        self.c.emit('fileDropHint', {'show': False})
        if event is not None:
            self.c.emit('fileDrop', event)

    def clear_pending(self):
        self.pending = None
        if self.timer is not None:
            self.GLib.source_remove(self.timer)
            self.timer = None

    def expire(self):
        pending, self.timer = self.pending, None
        self.pending = None
        if pending:
            self.Gtk.drag_finish(pending[0], False, False, pending[2])
        return self.GLib.SOURCE_REMOVE

    def close(self):
        if self.pending:
            self.Gtk.drag_finish(self.pending[0], False, False, self.pending[2])
        self.clear_pending()
        for handler in self.handlers:
            self.view.disconnect(handler)
        self.handlers.clear()
