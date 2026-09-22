#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Real GTK3/WebKit file drag transport on an isolated X11 display.

Run: DISPLAY=:98 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 python3 tests/native_file_transport.py
Use Xvfb, not the user's active display: XTest moves the pointer and presses keys.
Set CHROMIUM to an executable and provide Playwright to also test a separate
Chromium receiver using real native DataTransfer.Files. Use --ozone-platform=x11
(the harness sets this) so Chromium uses the isolated display.
Exercises production transports with synthetic HTML/controllers and temporary
files. This is not the complete app, T3 Code, Wayland, portal, or SMB validation.
"""
from __future__ import annotations
import ctypes as C
import ctypes.util
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace as NS

import gi
gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
gi.require_version('WebKit2', '4.1')
from gi.repository import Gtk, Gdk, Gio, GLib, WebKit2

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from native_file_drag import NativeFileDrag
from native_file_drop import NativeFileDrop
from native_tab_drag import NativeTabDrag, MIME as TAB_MIME
from tab_transfers import TabTransfers


def bind(lib, name, result, *args):
    function = getattr(lib, name)
    function.restype, function.argtypes = result, args
    return function


xlib = C.CDLL(ctypes.util.find_library('X11'))
xtest = C.CDLL(ctypes.util.find_library('Xtst'))
open_display = bind(xlib, 'XOpenDisplay', C.c_void_p, C.c_char_p)
close_display = bind(xlib, 'XCloseDisplay', C.c_int, C.c_void_p)
flush = bind(xlib, 'XFlush', C.c_int, C.c_void_p)
move = bind(xtest, 'XTestFakeMotionEvent', C.c_int, C.c_void_p, C.c_int, C.c_int, C.c_int, C.c_ulong)
button = bind(xtest, 'XTestFakeButtonEvent', C.c_int, C.c_void_p, C.c_uint, C.c_int, C.c_ulong)
key = bind(xtest, 'XTestFakeKeyEvent', C.c_int, C.c_void_p, C.c_uint, C.c_int, C.c_ulong)
keysym = bind(xlib, 'XKeysymToKeycode', C.c_uint, C.c_void_p, C.c_ulong)
checks, errors, events, receipts, browser_drops, windows = [], [], [], [], [], []
def record_exception(kind, value, traceback):
    errors.append(repr(value))
    sys.__excepthook__(kind, value, traceback)


sys.excepthook = record_exception


def check(name, condition):
    if not condition:
        raise AssertionError(name + (': ' + '; '.join(errors) if errors else ''))
    checks.append(name)


def pump(seconds=.15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        time.sleep(.004)


def until(predicate, message, seconds=8):
    deadline = time.monotonic() + seconds
    while not predicate() and time.monotonic() < deadline:
        pump(.025)
    check(message, predicate())


def javascript(view, code):
    done = []
    def completed(webview, result, _):
        try:
            webview.evaluate_javascript_finish(result)
            done.append(True)
        except Exception as exc:
            errors.append(repr(exc))
    view.evaluate_javascript(code, -1, None, None, None, completed, None)
    until(lambda: bool(done), 'WebKit JavaScript bridge completed')


def window(widget, x, y, width=520, height=350):
    top = Gtk.Window()
    top.set_default_size(width, height)
    top.move(x, y)
    top.add(widget)
    top.show_all()
    windows.append(top)
    return top


def at(widget, x, y):
    origin = widget.get_window().get_origin()
    move(display, -1, int(origin[-2] + x), int(origin[-1] + y), 0)
    flush(display)
    pump()


def release():
    button(display, 1, False, 0)
    flush(display)
    pump(.3)


if not Gtk.init_check()[0]:
    raise SystemExit('An isolated X11 display is required.')
if not Gdk.Display.get_default().get_name().startswith(':'):
    raise SystemExit('Use an isolated X11 display, not a Wayland session.')
display = open_display(None)
if not display:
    raise SystemExit('Could not open X11 test display.')

try:
    with tempfile.TemporaryDirectory(prefix='openxplorer-drag-fixture-') as temporary:
        folder = Path(temporary)
        first = folder / 'Meeting (1) #sample.mp4'
        second = folder / 'résumé.txt'
        child = folder / 'Sample Folder'
        first.write_text('First synthetic fixture\n')
        second.write_text('Second synthetic fixture\n')
        child.mkdir()
        paths = [first, second, child]
        uris = [Gio.File.new_for_path(str(path)).get_uri() for path in paths]
        exported = [path.as_uri() for path in paths]
        manager = WebKit2.UserContentManager()
        manager.register_script_message_handler('test')
        context = WebKit2.WebContext.new_ephemeral()
        context.set_sandbox_enabled(True)
        view = WebKit2.WebView(web_context=context, user_content_manager=manager)
        controller = NS(webview=view, writes=0, ui_ready=True, closed=False, drag_area=None,
                        window=NS(get_id=lambda: 1), previous_versions=NS(assert_writable=lambda _: None))
        app = NS(controllers=[controller])
        controller.app = app
        app.tab_transfers = TabTransfers(lambda *_: None, lambda _: True)

        def emit(name, data):
            events.append((name, data))
            if name == 'fileDragRequest':
                check('Native request preserves the exact GIO file-list identity', data['uri'] == uris[0])
                code = 'window.webkit.messageHandlers.test.postMessage({type:"begin",uri:' + json.dumps(data['uri']) + ',uris:window.selectedUris})'
                view.evaluate_javascript(code, -1, None, None, None, None, None)
            elif name == 'tabDragRequest':
                GLib.idle_add(lambda: controller.tab_drag.begin(data['id'], {'uri': uris[0]}) and False)

        controller.emit = emit
        controller.tab_drag = NativeTabDrag(controller, Gtk, Gdk, GLib)
        controller.file_drag = NativeFileDrag(controller, Gtk, Gdk, GLib)
        controller.file_drop = NativeFileDrop(controller, Gtk, Gdk, GLib)
        controller.tab_drag.update({'width': 520, 'height': 42, 'end': 430,
                                    'tabs': [{'id': 't1', 'left': 10, 'right': 190, 'close': 170}]})
        ready = []

        def message(_manager, result):
            value = json.loads(result.get_js_value().to_json(0))
            if value['type'] == 'ready':
                controller.file_drag.update(value)
                controller.file_drop.update({'width': value['width'], 'height': value['height'],
                                             'targets': [{'kind': 'copy', 'uri': child.as_uri(),
                                                          'left': 20, 'right': 490, 'top': 240, 'bottom': 330}]})
                ready.append(True)
            elif value['type'] == 'begin':
                controller.file_drag.begin(value['uri'], value['uris'])

        manager.connect('script-message-received::test', message)
        window(view, 20, 20)
        html = '''<!doctype html><meta charset="utf-8"><style>body{margin:0;user-select:none;-webkit-user-select:none}#file{position:absolute;left:20px;top:80px;width:470px;height:60px;background:#ddd;-webkit-user-drag:none}</style><div id="file" draggable="false">Synthetic file selection</div><script>window.selectedUris=URIS;requestAnimationFrame(()=>{const r=document.getElementById('file').getBoundingClientRect();window.webkit.messageHandlers.test.postMessage({type:'ready',width:innerWidth,height:innerHeight,items:[{uri:window.selectedUris[0],left:r.left,right:r.right,top:r.top,bottom:r.bottom}]});});</script>'''.replace('URIS', json.dumps(uris))
        view.load_html(html, 'file:///')
        until(lambda: bool(ready), 'Real WebKit source published its file hit rectangle')

        receiver = Gtk.EventBox()
        receiver.add(Gtk.Label(label='Native URI receiver'))
        receiver.drag_dest_set(0, [Gtk.TargetEntry.new('text/uri-list', 0, 1)], Gdk.DragAction.COPY)
        atom = Gdk.Atom.intern('text/uri-list', False)
        receiver.connect('drag-motion', lambda w, ctx, x, y, stamp: Gdk.drag_status(ctx, Gdk.DragAction.COPY, stamp) or True)
        receiver.connect('drag-drop', lambda w, ctx, x, y, stamp: w.drag_get_data(ctx, atom, stamp) or True)

        def received(widget, ctx, x, y, data, info, stamp):
            receipts.append({'uris': list(data.get_uris()), 'raw': bytes(data.get_data()), 'actions': int(ctx.get_actions())})
            Gtk.drag_finish(ctx, True, False, stamp)

        receiver.connect('drag-data-received', received)
        window(receiver, 580, 20, 450, 260)

        def start(selection=uris):
            javascript(view, 'window.selectedUris=' + json.dumps(selection))
            at(view, 70, 105)
            button(display, 1, True, 0)
            flush(display)
            pump(.08)
            at(view, 110, 105)
            until(lambda: controller.file_drag.context is not None, 'Real GDK gesture started production native file drag')

        start()
        targets = [target.name() for target in controller.file_drag.context.list_targets()]
        check('Native file targets include URI files and text, without tab capability',
              'text/uri-list' in targets and 'text/plain' in targets and TAB_MIME not in targets)
        at(receiver, 100, 100)
        release()
        until(lambda: bool(receipts), 'Native GTK receiver obtained the selected files and folder')
        check('Only COPY is offered to the native destination', receipts[-1]['actions'] == int(Gdk.DragAction.COPY))
        check('Native URI selection preserves punctuation, spaces, Unicode, order and folder references', receipts[-1]['uris'] == exported)
        check('URI payload uses standards-compatible CRLF line endings', receipts[-1]['raw'] == ('\r\n'.join(exported) + '\r\n').encode())
        until(lambda: controller.file_drag.context is None, 'Successful native drag cleared source state')
        check('Source files survived the external drop', first.is_file() and second.is_file() and child.is_dir())

        # A separate browser process mirrors native editor/attachment consumers.
        # Playwright sets up/reads the receiver; XTest performs the actual drag.
        if os.environ.get('CHROMIUM'):
            from playwright.sync_api import sync_playwright
            with sync_playwright() as playwright:
                chrome = playwright.chromium.launch(executable_path=os.environ['CHROMIUM'], headless=False,
                    args=['--no-sandbox', '--disable-dev-shm-usage', '--ozone-platform=x11', '--window-position=580,320', '--window-size=450,420'])
                page = chrome.new_page(no_viewport=True)
                page.set_content('''<!doctype html><body style="margin:0;height:280px">Browser attachment receiver<script>window.drops=[];document.addEventListener('dragover',e=>e.preventDefault());document.addEventListener('drop',async e=>{e.preventDefault();const files=Array.from(e.dataTransfer.files);window.drops.push(await Promise.all(files.map(async f=>({name:f.name,size:f.size,text:await f.text()}))));});</script>''')
                position = page.evaluate('({x:screenX+(outerWidth-innerWidth)/2+100,y:screenY+outerHeight-innerHeight+120})')
                start(uris[:2])
                move(display, -1, int(position['x']), int(position['y']), 0)
                flush(display)
                pump(.3)
                move(display, -1, int(position['x']) + 20, int(position['y']) + 10, 0)
                flush(display)
                pump(.4)
                release()
                deadline = time.monotonic() + 8
                while not browser_drops and time.monotonic() < deadline:
                    pump(.05)
                    browser_drops = page.evaluate('window.drops')
                check('Separate native Chromium receiver delivered a drop', bool(browser_drops))
                check('Real Chromium attachment drop contains actual readable DataTransfer.Files',
                      browser_drops[-1] == [{'name': first.name, 'size': first.stat().st_size, 'text': first.read_text()},
                                            {'name': second.name, 'size': second.stat().st_size, 'text': second.read_text()}])
                chrome.close()

        start(uris[:2])
        at(view, 120, 280)
        release()
        until(lambda: any(name == 'fileDrop' for name, _ in events), 'Production NativeFileDrop accepted an internal copy proposal')
        dropped = next(data for name, data in reversed(events) if name == 'fileDrop')
        check('Internal copy proposal preserves selection and destination', dropped['uris'] == exported[:2] and dropped['target'] == child.as_uri() and dropped['kind'] == 'copy')
        check('Native copy proposal alone never writes the destination', not list(child.iterdir()))

        start(uris[:2])
        escape = keysym(display, 0xff1b)
        key(display, escape, True, 0)
        key(display, escape, False, 0)
        flush(display)
        release()
        until(lambda: controller.file_drag.context is None, 'Escape cancelled native drag and released pointer grab')
        check('Cancellation was reported without deleting files', events[-1] == ('fileDragFinished', {'cancelled': True}) and first.is_file() and second.is_file())

        at(view, 70, 20)
        button(display, 1, True, 0)
        flush(display)
        pump(.08)
        at(view, 110, 20)
        until(lambda: controller.tab_drag.context is not None, 'Native tab drag still starts after file drags')
        check('Tab drags retain private payload and never export file targets', 'text/uri-list' not in [target.name() for target in controller.tab_drag.context.list_targets()])
        key(display, escape, True, 0)
        key(display, escape, False, 0)
        flush(display)
        release()
        until(lambda: controller.tab_drag.context is None, 'Tab drag cancellation still clears native state')
        check('Production signal handlers completed without exceptions', not errors)
        controller.file_drop.close()
        controller.file_drag.close()
        controller.tab_drag.close()
finally:
    button(display, 1, False, 0)
    flush(display)
    for top in windows:
        top.destroy()
    pump(.1)
    close_display(display)

print(json.dumps({'passed': len(checks), 'checks': checks, 'chromiumReceiverTested': bool(browser_drops),
                  'scope': 'Production NativeFileDrag/NativeFileDrop/NativeTabDrag with real GTK 3 and WebKit, XTest pointer events on isolated X11, temporary synthetic files and HTML. Optional separate Chromium receiver uses real native DataTransfer.Files; not full application, T3 Code, Wayland, sandbox portal or SMB validation.'}, indent=2))
