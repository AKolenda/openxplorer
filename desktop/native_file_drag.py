# SPDX-License-Identifier: AGPL-3.0-only
"""Copy-only GTK file drag source for the WebKit file list.

HTML pointer simulation cannot offer OS files to an editor or attachment field.
This transport retains a real GDK gesture and publishes GTK's URI targets,
including its FileTransfer portal target when available. It never deletes files,
extracts archive members, mounts shares, writes the clipboard, or runs commands.
"""
from __future__ import annotations

from dataclasses import dataclass
import math
from pathlib import Path
from urllib.parse import unquote, urlsplit

from core import normalise_location

URI_INFO = 0x5851
TEXT_INFO = 0x5852
MAX_ITEMS = 200
MAX_BYTES = 1024 * 1024


def file_uri(value):
    """Only concrete file/SMB URIs; never treat virtual entries as local paths."""
    if not isinstance(value, str) or len(value) > 16384 or urlsplit(value).scheme.lower() not in ('file', 'smb'):
        raise ValueError('Only local files and folders or SMB locations can be dragged. Extract archive members first.')
    # A share root is a valid reference/pin. Destructive and destination copy
    # operations retain their own require_item_uri checks in the transfer engine.
    return normalise_location(value)


@dataclass(frozen=True)
class DragFiles:
    uris: tuple[str, ...]
    exported: tuple[str, ...]
    text: str
    remote_only: int


def prepare_files(values, resolve_local=None):
    if not isinstance(values, list) or not 1 <= len(values) <= MAX_ITEMS:
        raise ValueError('Drag between 1 and 200 files or folders at a time.')
    uris = tuple(dict.fromkeys(file_uri(value) for value in values))
    exported, paths, remote_only = [], [], 0
    for uri in uris:
        parsed = urlsplit(uri)
        path = unquote(parsed.path) if parsed.scheme == 'file' else None
        if path is None and resolve_local is not None:
            # The resolver only discovers already-mounted local paths. It must
            # not mount, download or authenticate during a pointer gesture.
            path = resolve_local(uri)
        if path is not None:
            if not isinstance(path, str) or not path.startswith('/'):
                raise ValueError('The mounted item did not resolve to an absolute local path.')
            local_uri = normalise_location(Path(path).as_uri())
            exported.append(local_uri)
            paths.append(unquote(urlsplit(local_uri).path))
        else:
            exported.append(uri)
            paths.append(uri)
            remote_only += 1
    if sum(len(value.encode('utf-8')) for value in (*uris, *exported, *paths)) > MAX_BYTES:
        raise ValueError('This file selection is too large to drag.')
    return DragFiles(uris, tuple(exported), '\n'.join(paths), remote_only)


def layout_value(data):
    if not isinstance(data, dict):
        raise ValueError('Invalid file drag layout.')

    def number(value):
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or abs(value) > 100000:
            raise ValueError('Invalid file drag coordinates.')
        return float(value)

    width, height = number(data.get('width')), number(data.get('height'))
    if width < 1 or height < 1:
        raise ValueError('Invalid file drag viewport.')
    rows = data.get('items', [])
    if not isinstance(rows, list) or len(rows) > 10000:
        raise ValueError('Too many file drag regions.')
    items = []
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError('Invalid file drag region.')
        bounds = {key: number(row.get(key)) for key in ('left', 'right', 'top', 'bottom')}
        if bounds['left'] >= bounds['right'] or bounds['top'] >= bounds['bottom']:
            raise ValueError('Invalid file drag bounds.')
        items.append(dict(uri=file_uri(row.get('uri')), **bounds))
    return {'width': width, 'height': height, 'items': items}


class NativeFileDrag:
    def __init__(self, controller, Gtk, Gdk, GLib, resolve_local=None):
        self.c, self.Gtk, self.Gdk, self.GLib = controller, Gtk, Gdk, GLib
        self.view, self.resolve_local = controller.webview, resolve_local
        self.layout = self.press = self.context = self.files = None
        self.uris = ()
        self.requested = self.failed = self.closed = False
        self.handlers = []
        self.targets = Gtk.TargetList.new([])
        self.targets.add_uri_targets(URI_INFO)
        self.targets.add_text_targets(TEXT_INFO)
        self.view.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK | Gdk.EventMask.POINTER_MOTION_MASK)
        for signal, fn in [('button-press-event', self.button_press), ('button-release-event', self.button_release),
                           ('motion-notify-event', self.pointer_motion), ('drag-data-get', self.data_get),
                           ('drag-data-delete', self.data_delete), ('drag-end', self.drag_end),
                           ('drag-failed', self.drag_failed)]:
            self.handlers.append(self.view.connect(signal, fn))

    def update(self, data):
        self.layout = layout_value(data)
        if self.press and not any(row['uri'] == self.press['uri'] for row in self.layout['items']):
            self.press = None
            self.requested = False

    def item_at(self, x, y):
        if not self.layout:
            return None
        scale = self.view.get_allocated_width() / self.layout['width']
        if scale <= 0:
            return None
        x, y = x / scale, y / scale
        if not 0 <= x < self.layout['width'] or not 0 <= y < self.layout['height']:
            return None
        return next((row for row in self.layout['items']
                     if row['left'] <= x < row['right'] and row['top'] <= y < row['bottom']), None)

    def available(self):
        tab_drag = getattr(self.c, 'tab_drag', None)
        return not (self.closed or self.context or self.c.writes or not self.c.ui_ready
                    or tab_drag and tab_drag.context)

    def button_press(self, view, event):
        self.press = None
        self.requested = False
        if event.button != 1 or not self.available():
            return False
        row = self.item_at(event.x, event.y)
        if row:
            self.press = {'uri': row['uri'], 'x': event.x, 'y': event.y, 'event': event.copy()}
        return False

    def button_release(self, *_):
        self.press = None
        self.requested = False
        return False

    def pointer_motion(self, view, event):
        if not self.press or not self.available():
            return False
        if not event.state & self.Gdk.ModifierType.BUTTON1_MASK:
            self.button_release()
            return False
        p = self.press
        if not view.drag_check_threshold(int(p['x']), int(p['y']), int(event.x), int(event.y)):
            return False
        if not self.requested:
            p['event'] = event.copy()
            self.requested = True
            self.c.emit('fileDragRequest', {'uri': p['uri']})
        return True

    def begin(self, uri, uris):
        if not self.available() or not self.press or not self.requested or self.press['uri'] != file_uri(uri):
            raise ValueError('The file drag gesture ended before it could start. Drag the selection again.')
        p = self.press
        try:
            files = prepare_files(uris, self.resolve_local)
            if p['uri'] not in files.uris:
                raise ValueError('The dragged file must be included in the selection.')
            self.files, self.uris = files, files.uris
            self.failed = False
            self.context = self.view.drag_begin_with_coordinates(self.targets, self.Gdk.DragAction.COPY, 1,
                                                                 p['event'], int(p['x']), int(p['y']))
            if self.context is None:
                raise ValueError('The desktop could not start the file drag.')
            self.Gtk.drag_set_icon_name(self.context, 'text-x-generic' if len(self.uris) == 1 else 'edit-copy', 0, 0)
        except Exception:
            if self.context is not None:
                self.Gtk.drag_cancel(self.context)
            self.context = self.files = None
            self.uris = ()
            raise
        finally:
            self.press = None
            self.requested = False
        self.c.emit('fileDragStarted', {'uris': list(self.uris)})
        return {'started': True, 'count': len(self.uris), 'remoteOnly': files.remote_only}

    def data_get(self, view, context, data, info, time):
        if self.context is None or context != self.context or self.files is None:
            return
        view.stop_emission_by_name('drag-data-get')
        if info == URI_INFO:
            # GTK encodes CRLF URI lists and negotiates its portal target.
            data.set_uris(list(self.files.exported))
        elif info == TEXT_INFO:
            data.set_text(self.files.text, -1)

    def data_delete(self, view, context):
        if self.context is not None and context == self.context:
            # COPY is the only offered action. A destination must never turn a
            # source-side delete request into a filesystem mutation.
            view.stop_emission_by_name('drag-data-delete')

    def drag_failed(self, view, context, result):
        if self.context is None or context != self.context:
            return False
        view.stop_emission_by_name('drag-failed')
        self.failed = True
        return True

    def drag_end(self, view, context):
        if self.context is None or context != self.context:
            return
        view.stop_emission_by_name('drag-end')
        cancelled = self.failed
        self.context = self.files = self.press = None
        self.uris = ()
        self.requested = self.failed = False
        self.c.emit('fileDragFinished', {'cancelled': cancelled})

    def close(self):
        self.closed = True
        if self.context is not None:
            self.Gtk.drag_cancel(self.context)
        for handler in self.handlers:
            self.view.disconnect(handler)
        self.handlers.clear()
        self.layout = self.press = self.context = self.files = None
        self.uris = ()
