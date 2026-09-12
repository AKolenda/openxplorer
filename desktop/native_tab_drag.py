# SPDX-License-Identifier: AGPL-3.0-only
"""GTK drag transport for HTML tabs, restricted to this application process.

GTK owns the pointer grab so it can cross windows/Wayland surfaces. The browser
publishes hit rectangles; no screen-coordinate/window-position guesses, shell
commands, file paths, passwords, or external drop data are accepted here.
"""
from __future__ import annotations
import math

MIME = 'application/x-openxplorer-tab-v1'
INFO = 0x584F
# The compositor may consume this empty signal to confirm a drop on the
# desktop. It never receives our process-local token or a filesystem path.
ROOT_MIME = 'application/x-rootwindow-drop'
ROOT_INFO = 0x5850


def layout_value(data):
    if not isinstance(data, dict):
        raise ValueError('Invalid tab layout.')
    def number(v):
        if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) or abs(v) > 100000:
            raise ValueError('Invalid tab coordinates.')
        return float(v)
    width = number(data.get('width'))
    height = number(data.get('height'))
    end = number(data.get('end'))
    if width < 1 or height < 1 or height > 200 or not 0 <= end <= width:
        raise ValueError('Invalid tab strip dimensions.')
    rows = data.get('tabs', [])
    if not isinstance(rows, list) or len(rows) > 200:
        raise ValueError('Too many tabs.')
    tabs = []
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get('id'), str) or len(row['id']) > 80:
            raise ValueError('Invalid tab target.')
        left, right, close = number(row.get('left')), number(row.get('right')), number(row.get('close'))
        if right < left:
            raise ValueError('Invalid tab bounds.')
        tabs.append({'id': row['id'], 'left': left, 'right': right, 'close': close})
    return {'width': width, 'height': height, 'end': end, 'tabs': tabs}


class NativeTabDrag:
    def __init__(self, controller, Gtk, Gdk, GLib):
        self.c, self.Gtk, self.Gdk, self.GLib = controller, Gtk, Gdk, GLib
        self.view = controller.webview
        self.layout = None
        self.press = None
        self.requested = False
        self.token = None
        self.tab_id = None
        self.context = None
        self.drop_context = None
        self.tear_out_pending = False
        self.last_rejected = False
        self.handlers = []
        self.atom = Gdk.Atom.intern(MIME, False)
        self.root_atom = Gdk.Atom.intern(ROOT_MIME, False)
        self.targets = Gtk.TargetList.new([Gtk.TargetEntry.new(MIME, Gtk.TargetFlags.SAME_APP, INFO),
                                           Gtk.TargetEntry.new(ROOT_MIME, 0, ROOT_INFO)])
        # Preserve WebKit's targets and its default file/web-content behavior.
        targets = self.view.drag_dest_get_target_list()
        if targets is None:
            self.view.drag_dest_set(0, [], Gdk.DragAction.MOVE)
            targets = Gtk.TargetList.new([])
        targets.add(self.atom, Gtk.TargetFlags.SAME_APP, INFO)
        self.view.drag_dest_set_target_list(targets)
        self.view.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK | Gdk.EventMask.POINTER_MOTION_MASK)
        for signal, fn in [('button-press-event', self.button_press), ('button-release-event', self.button_release),
                           ('motion-notify-event', self.pointer_motion), ('drag-data-get', self.data_get),
                           ('drag-motion', self.drag_motion), ('drag-leave', self.drag_leave),
                           ('drag-drop', self.drag_drop), ('drag-data-received', self.data_received),
                           ('drag-end', self.drag_end), ('drag-failed', self.drag_failed)]:
            self.handlers.append((self.view, self.view.connect(signal, fn)))

        # The native window-move EventBox covers blank titlebar space. It must
        # accept tab drops too; otherwise dropping beside the last tab is lost.
        blank = getattr(controller, 'drag_area', None)
        if blank is not None:
            blank.drag_dest_set(0, [Gtk.TargetEntry.new(MIME, Gtk.TargetFlags.SAME_APP, INFO)], Gdk.DragAction.MOVE)
            for signal, fn in [('drag-motion', self.drag_motion), ('drag-leave', self.drag_leave),
                               ('drag-drop', self.drag_drop), ('drag-data-received', self.data_received)]:
                self.handlers.append((blank, blank.connect(signal, fn)))

    def drop_coords(self, widget, x, y):
        if widget != self.view:
            translated = widget.translate_coordinates(self.view, int(x), int(y))
            if translated is None:
                return None
            if len(translated) == 3:  # Bindings with an explicit success result.
                if not translated[0]: return None
                x, y = translated[1:]
            else:
                x, y = translated
        return self.coords(x, y)

    def update(self, data):
        self.layout = layout_value(data)

    def coords(self, x, y):
        if not self.layout:
            return None
        scale = self.view.get_allocated_width() / self.layout['width']
        if scale <= 0:
            return None
        x, y = x / scale, y / scale
        if not 0 <= y < self.layout['height'] or not 0 <= x < self.layout['end']:
            return None
        return x, y

    def button_press(self, view, event):
        self.press = None
        if event.button != 1 or self.token or self.c.writes:
            return False
        pos = self.coords(event.x, event.y)
        if pos:
            row = next((t for t in self.layout['tabs'] if max(0, t['left']) <= pos[0] < min(t['close'], t['right'])), None)
            if row:
                self.press = {'id': row['id'], 'x': event.x, 'y': event.y, 'event': event.copy()}
                self.requested = False
        return False

    def button_release(self, *_):
        self.press = None
        self.requested = False
        return False

    def pointer_motion(self, view, event):
        if not self.press or self.token or not event.state & self.Gdk.ModifierType.BUTTON1_MASK:
            return False
        p = self.press
        if not view.drag_check_threshold(int(p['x']), int(p['y']), int(event.x), int(event.y)):
            return False
        if not self.requested:
            p['event'] = event.copy()
            self.requested = True
            self.c.emit('tabDragRequest', {'id': p['id']})
        return True

    def begin(self, tab_id, snapshot):
        if not self.press or not self.requested or self.press['id'] != tab_id or self.token:
            raise ValueError('The drag gesture ended before it could start. Try again or use Move tab to window.')
        self.token = self.c.app.tab_transfers.offer(self.c.window.get_id(), tab_id, snapshot)
        self.tab_id = tab_id
        self.tear_out_pending = False
        self.last_rejected = False
        p = self.press
        try:
            self.context = self.view.drag_begin_with_coordinates(self.targets, self.Gdk.DragAction.MOVE, 1, p['event'], int(p['x']), int(p['y']))
            if self.context is None:
                raise ValueError('The desktop could not start a tab drag.')
            self.Gtk.drag_set_icon_name(self.context, 'folder', 0, 0)
        except Exception:
            self.c.app.tab_transfers.cancel(self.token)
            self.token = None
            self.tab_id = None
            self.context = None
            raise
        finally:
            self.press = None
        return {'started': True}

    def source(self, context):
        if not any(a.name() == MIME for a in context.list_targets()):
            return None
        widget = self.Gtk.drag_get_source_widget(context)
        source = next((c for c in self.c.app.controllers if c.webview == widget and not c.closed), None)
        drag = getattr(source, 'tab_drag', None)
        return drag if drag and drag.token else None

    def before(self, x):
        return next((t['id'] for t in self.layout['tabs'] if x < (t['left'] + t['right']) / 2), None)

    def data_get(self, view, context, data, info, time):
        if context != self.context or not self.token or info not in (INFO, ROOT_INFO):
            return
        view.stop_emission_by_name('drag-data-get')
        if info == ROOT_INFO:
            # Complete the selection even though there is no payload. Failing to
            # reply can hang the compositor's drag handshake. Defer new-window
            # creation until drag-end releases the native pointer grab.
            data.set(self.root_atom, 8, b'')
            p = self.c.app.tab_transfers.pending.get(self.token)
            self.tear_out_pending = bool(p and p.destination is None)
            self.last_rejected = False
        else:
            self.tear_out_pending = False
            data.set(self.atom, 8, self.token.encode('ascii'))

    def drop_action(self, widget, source, x, y):
        """Only the source's body is a tear-out area; other windows need a tab strip."""
        if not self.layout or self.c.writes or not self.c.ui_ready:
            return None, None
        pos = self.drop_coords(widget, x, y)
        if pos is not None:
            return 'tabs', self.before(pos[0])
        if source is self and widget == self.view:
            scale = self.view.get_allocated_width() / self.layout['width']
            if scale > 0 and 0 <= x / scale < self.layout['width'] and y / scale >= self.layout['height'] + 40:
                return 'detach', None
        return None, None

    def drag_motion(self, view, context, x, y, time):
        source = self.source(context)
        if source is None:
            return False
        view.stop_emission_by_name('drag-motion')
        action, before = self.drop_action(view, source, x, y)
        source.last_rejected = action is None
        # DEFAULT is a nonzero action, not a refusal. Zero explicitly rejects.
        self.Gdk.drag_status(context, self.Gdk.DragAction.MOVE if action else self.Gdk.DragAction(0), time)
        self.c.emit('tabDropHint', {'show': action == 'tabs', 'beforeId': before})
        self.c.emit('tabTearOutHint', {'show': action == 'detach', 'x': 40, 'y': self.layout['height'] + 60 if self.layout else 0})
        return True

    def drag_leave(self, view, context, time):
        if self.source(context) is not None:
            view.stop_emission_by_name('drag-leave')
            self.c.emit('tabDropHint', {'show': False})
            self.c.emit('tabTearOutHint', {'show': False})

    def drag_drop(self, view, context, x, y, time):
        if self.source(context) is None:
            return False
        view.stop_emission_by_name('drag-drop')
        action, before = self.drop_action(view, self.source(context), x, y)
        if action is None:
            self.Gtk.drag_finish(context, False, False, time)
            return True
        self.drop_context = (context, before, time, action)
        view.drag_get_data(context, self.atom, time)
        return True

    def data_received(self, view, context, x, y, data, info, time):
        if info != INFO or not self.drop_context or self.drop_context[0] != context:
            return
        view.stop_emission_by_name('drag-data-received')
        _, before, timestamp, action = self.drop_context
        self.drop_context = None
        ok = False
        try:
            source = self.source(context)
            raw = data.get_data()
            if not source or raw is None or len(raw) != 64 or raw.decode('ascii') != source.token:
                raise ValueError('Invalid tab drop. Only tabs from this OpenXplorer process can be moved.')
            if action == 'detach':
                if source is not self:
                    raise ValueError('Only the source window can request a tear-out here.')
                source.tear_out_pending = True
            elif source is self:
                self.c.emit('tabReorder', {'id': source.tab_id, 'beforeId': before})
                self.c.app.tab_transfers.cancel(source.token, '')
            else:
                self.c.app.tab_transfers.claim(source.token, self.c.window.get_id(), before)
                self.c.window.present()
            ok = True
        except Exception as exc:
            self.c.emit('notice', {'message': str(exc)})
        self.c.emit('tabDropHint', {'show': False})
        # GTK never deletes anything. The application retires the tab only on ACK.
        self.Gtk.drag_finish(context, ok, False, timestamp)

    def drag_failed(self, view, context, result):
        if context != self.context or not self.token:
            return False
        view.stop_emission_by_name('drag-failed')
        # Wayland can report ERROR for both a cancelled and an unaccepted drag.
        # Do not turn all errors into a new window (Escape must remain cancel).
        self.tear_out_pending = result == self.Gtk.DragResult.NO_TARGET and not self.last_rejected
        if not self.tear_out_pending:
            self.c.app.tab_transfers.cancel(self.token, 'Tab move cancelled. To detach, release below the tab strip or use Move tab to new window.')
        return True

    def drag_end(self, view, context):
        if context != self.context:
            return
        view.stop_emission_by_name('drag-end')
        token, tab_id = self.token, self.tab_id
        p = self.c.app.tab_transfers.pending.get(token)
        detach = bool(self.tear_out_pending and p and p.destination is None)
        if p and p.destination is None:
            self.c.app.tab_transfers.cancel(token, '')
        self.token = self.tab_id = self.context = self.press = None
        self.requested = self.tear_out_pending = self.last_rejected = False
        self.c.emit('tabDropHint', {'show': False})
        self.c.emit('tabTearOutHint', {'show': False})
        if detach:
            # The existing acknowledged handoff retains the original tab until
            # the new WebKit window is ready, including startup/error timeouts.
            self.c.emit('tabDetachRequested', {'id': tab_id})

    def close(self):
        for widget, handler in self.handlers:
            widget.disconnect(handler)
        self.handlers.clear()
        self.press = self.context = self.drop_context = None
