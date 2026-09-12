#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""OpenXplorer: a local-only WebKitGTK interface backed by native GIO/GVfs.

No localhost server, Electron runtime, password database, fstab edits or shell
file operations. Python's system GI packages are required; see README.md.
"""
from __future__ import annotations
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import sys
import subprocess
import time
from urllib.parse import urlsplit

INSTALL = 'sudo apt install python3-gi gir1.2-gtk-3.0 gir1.2-webkit2-4.1 gvfs-backends gvfs-fuse gir1.2-secret-1 xdg-utils'
try:
    import gi
    gi.require_version('Gtk', '3.0')
    gi.require_version('Gdk', '3.0')
    gi.require_version('WebKit2', '4.1')
    gi.require_version('Gio', '2.0')
    from gi.repository import Gtk, Gdk, Gio, GLib, WebKit2
except (ImportError, ValueError) as exc:
    print('OpenXplorer needs the system Python GTK/WebKit packages.\n\n' + INSTALL +
          '\n\nThen run: /usr/bin/python3 openxplorer.py\n\nDetails: ' + str(exc), file=sys.stderr)
    sys.exit(1)

from core import VERSION, Settings, normalise_location, require_share, safe_label, require_item_uri, is_smb_server
from gio_backend import (GioCancellation, GioNode, enumerate_folder, verify_folder,
                         inspect, create_item, rename_item, error_payload, verify_pin, index_directory, discover_servers,
                         trash_support)
from operations import TransferEngine
from search_index import SearchIndex, below
from index_service import IndexService
from auth_bridge import MountPrompts
from desktop_integration import DesktopIntegration
from folder_locations import FolderLocations
from mount_support import read_mounts, mount_plan, mount_for_path
from previous_versions import PreviousVersions
from file_services import properties, list_applications, prepare_launch, list_templates, create_from_template, SnapshotProvider
from file_clipboard import FileClipboard
from session_credentials import SessionCredentials
from activation import activation_kind
from native_opening import prepare_default, archive_stream, local_path
from archives import Archives
from zip_extraction import ZipExtractor
from gio_backend import exclusive_output
from app_catalog import editor_shortcuts
from folder_sizes import scan_folder
from network_locations import merge_network_locations
from window_state import tab_snapshot, location as window_location
from tab_transfers import TabTransfers
from native_tab_drag import NativeTabDrag
from native_file_drag import NativeFileDrag
from native_file_drop import NativeFileDrop
from reveal_integration import RevealRegistration
from filemanager_bus import FileManagerBus
from brave_integration import BraveIntegration
from terminal_integration import prepare_directory, launch_terminal
from runtime_guard import identity, Session, require_current

ROOT = Path(__file__).resolve().parent
APP_URI = (ROOT / 'ui' / 'index.html').as_uri()
# Captured at process start, not re-read when an old process creates a new window.
RUNTIME = identity(ROOT, VERSION)


class OpenXplorerWindow:
    """One independent browsing window, owned by a single Gtk.Application."""
    def __init__(self, app, initial=None, software_rendering=False, transfer=None):
        self.app = app
        self.transfer = transfer
        self.handoff = None
        self.tab_drag = None
        self.file_drag = None
        self.file_drop = None
        self.external_pending = []
        self.tab_titles = []
        self.settings_store = Settings()
        self.initial = initial
        self.pending_open = []
        self.request_locations = {}
        self.signing_out_hosts = set()
        self.query_workers = ThreadPoolExecutor(max_workers=2, thread_name_prefix='winspace-query')
        self.search_index = SearchIndex(recover=False)
        self.indexer = IndexService(self.search_index, index_directory, GioCancellation,
                                   lambda: self.emit('cacheChanged', {}))
        self.desktop_integration = app.desktop_integration
        self.brave = app.brave
        self.folder_locations = FolderLocations(self.settings_store.directory)
        self.previous_versions = PreviousVersions(self.settings_store.directory, SnapshotProvider())
        self.archives = Archives(archive_stream, Path(GLib.get_user_runtime_dir())/'winspace-archive-previews')
        try:
            gi.require_version('Secret', '1')
            from gi.repository import Secret
        except (ImportError, ValueError):
            Secret = None
        self.prompts = MountPrompts(Gio, GLib, self.emit, SessionCredentials(Secret))
        self.file_clipboard = None
        self.index_tick_busy = False
        self.index_timer = None
        self.software_rendering = software_rendering
        self.ui_ready = False
        self.ui_load_started = False
        self.startup_timeout = None
        self.desktop_settings = None
        self.theme_signals = []
        self.native_css = None
        self.gtk_system_dark = False
        self.window = None
        self.webview = None
        self.size_workers = ThreadPoolExecutor(max_workers=1, thread_name_prefix='winspace-size')
        self.readers = ThreadPoolExecutor(max_workers=3, thread_name_prefix='winspace-read')
        self.writer = ThreadPoolExecutor(max_workers=1, thread_name_prefix='winspace-write')
        self.jobs: dict[str, GioCancellation] = {}
        self.active_requests: set[int] = set()
        self.writes = 0
        self.closed = False
        self.monitors = {}
        self.monitor_debounce = {}
        self.mount_ops = {}
        self.volume_monitor = None
        self.volume_signals = []

    def on_open_locations(self, _app, files, *rest):
        # PyGObject versions differ in whether the array length is exposed.
        # We need only the GFile array; *rest accepts both signal signatures.
        uris = [normalise_location(f.get_uri()) for f in files]
        self.activate_window()
        if self.ui_ready:
            self.emit('openLocations', {'uris': uris})
        else:
            self.pending_open.extend(uris)

    def on_mouse_button(self, _view, event):
        if event.button in (8, 9):
            self.emit('mouseNavigate', {'delta': -1 if event.button == 8 else 1})
            return True  # Prevent a second DOM/browser history action.
        return False

    def index_tick(self):
        if self.closed:
            return GLib.SOURCE_REMOVE
        if not self.index_tick_busy:
            self.index_tick_busy = True
            prefs = self.settings_store.snapshot()['preferences']
            self.indexer.enabled = prefs.get('autoIndex',True)
            self.indexer.network_interval = prefs.get('networkInterval',60)
            future = self.query_workers.submit(self.indexer.refresh_due)
            def done(f):
                self.index_tick_busy = False
                try: f.result()
                except Exception:
                    self.emit('notice',{'message':'The index coordinator could not finish an update. Check Search cache status.'})
            future.add_done_callback(done)
        return GLib.SOURCE_CONTINUE

    def activate_window(self, *_):
        if self.window:
            self.window.present()
            return
        # Only the elected index owner recovers interrupted scans.
        self.window = Gtk.ApplicationWindow(application=self.app)
        # GTK otherwise inserts a fallback app-menu row above our HTML tabs
        # when the desktop shell does not export application menus. Disable
        # it before mapping EVERY window (including Settings and torn tabs).
        # No Gtk application-menu model is registered: the HTML and launcher
        # expose the actions. Keep decorations for compositor resizing.
        self.window.set_show_menubar(False)
        self.window.set_name('winspace-shell')
        self.window.set_title('OpenXplorer')
        Gtk.Window.set_default_icon_name('io.winspace.Development')
        self.window.set_icon_name('io.winspace.Development')
        try:
            self.file_clipboard = FileClipboard(Gtk,Gdk,lambda: self.emit('clipboardChanged',{}))
        except Exception:
            self.file_clipboard = None
        self.window.set_default_size(1320, 810)
        self.window.set_size_request(670, 470)
        self.window.connect('delete-event', self.on_delete)
        self.window.connect('destroy', lambda *_: self.app.window_closed(self))
        self.gtk_system_dark = bool(Gtk.Settings.get_default().get_property('gtk-application-prefer-dark-theme'))
        source = Gio.SettingsSchemaSource.get_default()
        schema = source.lookup('org.gnome.desktop.interface', True) if source else None
        if schema:
            self.desktop_settings = Gio.Settings.new_full(schema, None, None)
            for key in ('color-scheme', 'gtk-theme'):
                if schema.has_key(key):
                    self.theme_signals.append(self.desktop_settings.connect('changed::' + key, self.on_system_theme_changed))
        # A custom zero-height titlebar requests GTK client-side decorations,
        # preserving real compositor resizing. The visible tab strip is HTML.
        title = Gtk.Box()
        title.set_name('winspace-title')
        self.window.set_titlebar(title)
        self.native_css = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), self.native_css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        manager = WebKit2.UserContentManager()
        manager.register_script_message_handler('host')
        manager.connect('script-message-received::host', self.on_message)
        boot = {'theme': self.settings_store.snapshot()['preferences']['theme'], 'systemDark': self.system_dark()}
        injected = ('Object.defineProperty(window,"__OPENXPLORER_NATIVE__",{value:true,writable:false});'
                    + 'Object.defineProperty(window,"__OPENXPLORER_BOOT__",{value:' + json.dumps(boot) + ',writable:false});')
        script = WebKit2.UserScript.new(injected,
                                        WebKit2.UserContentInjectedFrames.TOP_FRAME,
                                        WebKit2.UserScriptInjectionTime.START, None, None)
        manager.add_script(script)
        context = WebKit2.WebContext.new_ephemeral()
        # Must precede creation of ANY web process. Never grant all of HOME or /.
        context.set_sandbox_enabled(True)
        context.add_path_to_sandbox(str(ROOT / 'ui'), True)
        context.connect('download-started', lambda _c, d: d.cancel())
        self.webview = WebKit2.WebView(web_context=context, user_content_manager=manager)
        websettings = self.webview.get_settings()
        websettings.set_enable_developer_extras(False)
        websettings.set_javascript_can_open_windows_automatically(False)
        websettings.set_enable_html5_database(False)
        websettings.set_enable_html5_local_storage(False)
        if self.software_rendering:
            websettings.set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.NEVER)
        self.apply_native_theme()
        self.webview.add_events(Gdk.EventMask.BUTTON_PRESS_MASK)
        self.webview.connect('button-press-event', self.on_mouse_button)
        self.webview.connect('map', self.on_webview_mapped)
        self.webview.connect('load-changed', self.on_load_changed)
        self.webview.connect('load-failed', self.on_load_failed)
        self.webview.connect('decide-policy', self.on_policy)
        self.webview.connect('permission-request', lambda _w, r: (r.deny(), True)[1])
        self.webview.connect('context-menu', lambda *_: True)
        self.webview.connect('web-process-terminated', self.on_web_crash)
        overlay = Gtk.Overlay()
        overlay.add(self.webview)
        # This native event surface covers ONLY blank titlebar space. Movement
        # receives a real GDK event/time/seat; it is not a fabricated X11 drag.
        self.drag_area = Gtk.EventBox()
        self.drag_area.set_visible_window(False)
        self.drag_area.set_halign(Gtk.Align.START)
        self.drag_area.set_valign(Gtk.Align.START)
        self.drag_area.set_size_request(1, 1)
        self.drag_area.add_events(Gdk.EventMask.BUTTON_PRESS_MASK)
        self.drag_area.connect('button-press-event', self.on_title_press)
        overlay.add_overlay(self.drag_area)
        # A native startup surface remains visible even if HTML/JS cannot load.
        # It disappears only after the DOM/bridge explicitly reports ready.
        self.loading_cover = Gtk.EventBox()
        self.loading_cover.set_name('winspace-loading')
        self.loading_cover.set_halign(Gtk.Align.FILL)
        self.loading_cover.set_valign(Gtk.Align.FILL)
        loading = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        loading.set_halign(Gtk.Align.CENTER)
        loading.set_valign(Gtk.Align.CENTER)
        self.startup_spinner = Gtk.Spinner()
        self.startup_spinner.start()
        self.startup_label = Gtk.Label(label='Starting OpenXplorer…')
        self.startup_label.set_line_wrap(True)
        self.startup_label.set_max_width_chars(64)
        self.startup_label.set_justify(Gtk.Justification.CENTER)
        loading.pack_start(self.startup_spinner, False, False, 0)
        loading.pack_start(self.startup_label, False, False, 0)
        self.startup_actions = Gtk.Box(spacing=10)
        self.startup_actions.set_halign(Gtk.Align.CENTER)
        retry = Gtk.Button(label='Retry')
        retry.connect('clicked', lambda *_: self.retry_startup(False))
        software = Gtk.Button(label='Retry with software rendering')
        software.connect('clicked', lambda *_: self.retry_startup(True))
        close = Gtk.Button(label='Close')
        close.connect('clicked', lambda *_: self.window.close())
        for button in (retry, software, close):
            self.startup_actions.pack_start(button, False, False, 0)
        loading.pack_start(self.startup_actions, False, False, 0)
        self.loading_cover.add(loading)
        overlay.add_overlay(self.loading_cover)
        self.window.add(overlay)
        self.tab_drag = NativeTabDrag(self, Gtk, Gdk, GLib)
        self.file_drag = NativeFileDrag(self, Gtk, Gdk, GLib, resolve_local=local_path)
        self.file_drop = NativeFileDrop(self, Gtk, Gdk, GLib)
        self.volume_monitor = Gio.VolumeMonitor.get()
        for signal in ('mount-added', 'mount-removed', 'mount-changed', 'volume-added', 'volume-removed'):
            self.volume_signals.append(self.volume_monitor.connect(signal, lambda *_: self.emit('mounts', {})))
        # Map the GTK surface BEFORE loading HTML; load_uri used to run before
        # any native surface existed. The map signal schedules exactly one load.
        self.window.show_all()
        self.startup_actions.hide()
        self.window.present()

    def system_dark(self) -> bool:
        if self.desktop_settings:
            schema = self.desktop_settings.props.settings_schema
            if schema.has_key('color-scheme'):
                value = self.desktop_settings.get_string('color-scheme')
                if value == 'prefer-dark':
                    return True
                if value == 'prefer-light':
                    return False
            if schema.has_key('gtk-theme'):
                return 'dark' in self.desktop_settings.get_string('gtk-theme').lower()
        return self.gtk_system_dark

    def on_system_theme_changed(self, *_):
        self.apply_native_theme()
        self.emit('theme', {'systemDark': self.system_dark()})

    def apply_native_theme(self):
        preference = self.settings_store.snapshot()['preferences']['theme']
        dark = preference == 'dark' or (preference == 'system' and self.system_dark())
        bg, fg = ('#202020', '#f1f1f1') if dark else ('#f7f7f7', '#242424')
        if self.native_css:
            self.native_css.load_from_data((
                '#winspace-title { min-height: 0; padding: 0; margin: 0; border: 0; }'
                + '#winspace-shell, #winspace-loading { background-color: ' + bg + '; color: ' + fg + '; }'
            ).encode())
        # Only this application's GTK settings are touched, not GNOME's theme.
        Gtk.Settings.get_default().set_property('gtk-application-prefer-dark-theme', dark)
        if self.webview:
            color = Gdk.RGBA()
            color.parse(bg)
            self.webview.set_background_color(color)

    def on_webview_mapped(self, *_):
        if not self.ui_load_started:
            self.ui_load_started = True
            GLib.idle_add(self.begin_ui_load)
        return False

    def begin_ui_load(self):
        if self.closed:
            return GLib.SOURCE_REMOVE
        self.ui_ready = False
        self.webview.load_uri(APP_URI)
        if self.startup_timeout:
            GLib.source_remove(self.startup_timeout)
        self.startup_timeout = GLib.timeout_add_seconds(12, self.startup_timed_out)
        return GLib.SOURCE_REMOVE

    def startup_timed_out(self):
        self.startup_timeout = None
        if not self.ui_ready and not self.closed:
            self.startup_spinner.stop()
            self.startup_label.set_text('The interface is taking longer than expected.\nTry software rendering if this window previously stayed blank.')
            self.startup_actions.show_all()
        return GLib.SOURCE_REMOVE

    def retry_startup(self, software=False):
        if self.writes or self.ui_ready:
            return
        self.software_rendering = software or self.software_rendering
        if self.software_rendering:
            self.webview.get_settings().set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.NEVER)
        self.startup_actions.hide()
        self.startup_label.set_text('Starting OpenXplorer…')
        self.startup_spinner.start()
        self.begin_ui_load()

    def on_load_changed(self, _view, event):
        if event == WebKit2.LoadEvent.FINISHED:
            self.request_initial_paint()

    def on_load_failed(self, _view, _event, _uri, error):
        if self.startup_timeout:
            GLib.source_remove(self.startup_timeout)
            self.startup_timeout = None
        self.startup_spinner.stop()
        self.startup_label.set_text('Could not load the OpenXplorer interface.\n' + str(error))
        self.startup_actions.show_all()
        self.loading_cover.show()
        return True

    def request_initial_paint(self):
        # Real widget invalidation, not synthetic window movement, forced X11,
        # an arbitrary resize, or a permanently running redraw timer.
        if self.closed or not self.webview or not self.webview.get_mapped():
            return GLib.SOURCE_REMOVE
        self.webview.queue_resize()
        self.webview.queue_draw()
        self.window.queue_draw()
        return GLib.SOURCE_REMOVE

    def mark_ui_ready(self):
        if self.ui_ready:
            return
        self.ui_ready = True
        if self.pending_open:
            self.emit('openLocations', {'uris': self.pending_open[:]})
            self.pending_open.clear()
        for request in self.external_pending:
            self.emit('fileManagerRequest', request)
        self.external_pending.clear()
        if self.transfer:
            self.emit('restoreTab', self.transfer)
        if self.index_timer is None:
            self.index_timer = GLib.timeout_add_seconds(1, self.index_tick)
            self.index_tick()
        if self.startup_timeout:
            GLib.source_remove(self.startup_timeout)
            self.startup_timeout = None
        self.startup_spinner.stop()
        self.loading_cover.hide()
        self.request_initial_paint()
        # Bounded extra invalidations cover the GTK/WebKit handoff at mapping.
        GLib.timeout_add(80, self.request_initial_paint)
        GLib.timeout_add(300, self.request_initial_paint)

    def on_title_press(self, _widget, event):
        if event.button != 1:
            return False
        if event.type == Gdk.EventType._2BUTTON_PRESS:
            self.toggle_maximize()
        else:
            self.window.begin_move_drag(event.button, int(event.x_root), int(event.y_root), event.time)
        return True

    def toggle_maximize(self):
        if self.window.is_maximized():
            self.window.unmaximize()
        else:
            self.window.maximize()

    def on_policy(self, _web, decision, policy):
        if policy in (WebKit2.PolicyDecisionType.NAVIGATION_ACTION, WebKit2.PolicyDecisionType.NEW_WINDOW_ACTION):
            uri = decision.get_navigation_action().get_request().get_uri()
            if policy == WebKit2.PolicyDecisionType.NEW_WINDOW_ACTION or uri != APP_URI:
                decision.ignore()
                return True
        return False

    def on_web_crash(self, *_):
        self.ui_ready = False
        self.app.tab_transfers.window_closed(self.window.get_id())
        for cancel in self.jobs.values():
            cancel.cancel()
        print('OpenXplorer UI process stopped. Operations have been cancelled; inspect any .winspace-transfer-*.part folders before removing them.', file=sys.stderr)
        dialog = Gtk.MessageDialog(transient_for=self.window, modal=True, message_type=Gtk.MessageType.ERROR,
                                   buttons=Gtk.ButtonsType.CLOSE, text='The interface stopped unexpectedly.')
        dialog.format_secondary_text('Active operations were asked to cancel. Keep this window open until transfers finish. Restart OpenXplorer to continue; inspect any .winspace-transfer-*.part directories before removing them.')
        dialog.connect('response', lambda d, _r: d.destroy())
        dialog.show()

    def js(self, code: str):
        if self.closed or self.webview is None:
            return
        # This is trusted application code plus JSON-encoded data, never names
        # interpolated into HTML, script syntax, shell commands or URLs.
        try:
            self.webview.evaluate_javascript(code, -1, None, None, None, self.on_js_done, None)
        except Exception as exc:
            print('UI dispatch failed:', str(exc), file=sys.stderr)

    def on_js_done(self, view, result, *_):
        try:
            view.evaluate_javascript_finish(result)
        except GLib.Error:
            pass  # The page can be closing or not ready for a mount event.

    def emit(self, name: str, data: dict):
        def send():
            self.js('window.__nativeEvent && window.__nativeEvent(' + json.dumps(name) + ',' + json.dumps(data, ensure_ascii=True) + ');')
            return GLib.SOURCE_REMOVE
        GLib.idle_add(send)

    def respond(self, request: dict, result=None, error: Exception | dict | None = None):
        self.active_requests.discard(request['id'])
        payload = error if isinstance(error, dict) else error_payload(error) if error else None
        self.js('window.__nativeResolve && window.__nativeResolve(' + json.dumps(request['id']) + ',' +
                json.dumps(result, ensure_ascii=True) + ',' + json.dumps(payload, ensure_ascii=True) + ');')

    def on_message(self, _manager, message):
        request = None
        try:
            if self.webview.get_uri() != APP_URI:
                return
            text = message.get_js_value().to_string()
            if len(text) > 2_000_000:
                raise ValueError('Request is too large.')
            request = json.loads(text)
            if not isinstance(request, dict) or not isinstance(request.get('id'), int) or not isinstance(request.get('method'), str) or not isinstance(request.get('args'), dict):
                raise ValueError('Malformed request.')
            if request['id'] in self.active_requests:
                return
            if len(self.active_requests) > 128:
                raise ValueError('Too many outstanding requests. Wait for the current tasks.')
            # New UI files may have appeared while this process was running.
            # Reject mismatched actions BEFORE any filesystem or shell activity.
            if request.get('release') not in (None, VERSION) and request['method'] not in ('environment', 'quit', 'uiReady'):
                self.respond(request, error={'code': 'version-mismatch', 'message':
                    'The interface and running application are different versions. Finish file operations, then run openxplorer --restart.'})
                return
            self.active_requests.add(request['id'])
            self.dispatch(request)
        except Exception as exc:
            if isinstance(request, dict) and isinstance(request.get('id'), int):
                self.respond(request, error=exc)
            else:
                print('Rejected an invalid UI message.', file=sys.stderr)

    def dispatch(self, request: dict):
        method, a = request['method'], request['args']
        if method.startswith('clipboard') and self.file_clipboard is None:
            raise ValueError('The desktop file clipboard is unavailable. Restart OpenXplorer in your normal desktop session.')
        if method == 'environment':
            self.respond(request, self.environment())
        elif method == 'authReply':
            self.respond(request, self.prompts.answer(a))
        elif method == 'search':
            self.start_worker(request, lambda c: self.search_index.search(
                a.get('query', ''), a.get('scope'), a.get('limit', 500), bool(a.get('showHidden')), c),
                pool=self.query_workers)
        elif method == 'cacheStatus':
            self.start_worker(request, lambda c: self.search_index.snapshot(), pool=self.query_workers)
        elif method == 'cacheSet':
            uri = normalise_location(a.get('uri'))
            if a.get('enabled') is not True:
                self.indexer.cancel(uri)
            def configure(c):
                result = self.search_index.configure(uri, a.get('enabled'), a.get('label', ''), bool(a.get('includeHidden')))
                if a.get('enabled'):
                    self.indexer.refresh(uri)
                return result
            self.start_worker(request, configure, pool=self.query_workers)
        elif method == 'cacheRefresh':
            uri = normalise_location(a['uri']) if a.get('uri') else None
            def refresh_cache(c):
                roots = [uri] if uri else [r['uri'] for r in self.search_index.roots() if r['enabled']]
                for root in roots:
                    self.indexer.refresh(root)
                return self.search_index.snapshot()
            self.start_worker(request, refresh_cache, pool=self.query_workers)
        elif method in ('cacheClear', 'cacheRemove', 'cacheStop'):
            uri = normalise_location(a.get('uri'))
            self.indexer.cancel(uri)
            def change_cache(c):
                if method == 'cacheClear':
                    self.search_index.clear(uri)
                elif method == 'cacheRemove':
                    self.search_index.remove(uri)
                return self.search_index.snapshot()
            self.start_worker(request, change_cache, pool=self.query_workers)
        elif method == 'desktopStatus':
            self.start_worker(request, lambda c: self.desktop_integration.status(),
                              complete=lambda value: self.respond(request, {**value, **self.app.reveal_status()}))
        elif method == 'desktopDefault':
            def finish_default(value):
                if a.get('reveal') is True:
                    try: self.app.enable_reveal()
                    except Exception as exc: value['integrationError'] = str(exc)
                self.respond(request, {**value, **self.app.reveal_status()})
            self.start_worker(request, lambda c: self.desktop_integration.make_default(include_zip=a.get('zip') is True), complete=finish_default)
        elif method in ('zipDefault', 'zipRestore'):
            self.start_worker(request, lambda c: self.desktop_integration.zip_default() if method == 'zipDefault' else self.desktop_integration.restore(zip_only=True),
                              complete=lambda value: self.respond(request, {**value, **self.app.reveal_status()}))
        elif method == 'revealEnable':
            self.app.enable_reveal()
            self.respond(request, self.app.reveal_status())
        elif method == 'revealDisable':
            self.respond(request, self.app.disable_reveal())
        elif method == 'revealTest':
            if not self.app.bus_endpoint or not self.app.bus_endpoint.owned:
                raise ValueError('OpenXplorer does not own Show in folder yet. Close other file managers, or log out and back in after enabling.')
            connection = self.app.get_dbus_connection()
            connection.call('org.freedesktop.FileManager1', '/org/freedesktop/FileManager1',
                'org.freedesktop.FileManager1', 'ShowFolders',
                GLib.Variant('(ass)', ([Path.home().as_uri()], '')), None,
                Gio.DBusCallFlags.NONE, 5000, None,
                lambda conn, result: self.finish_reveal_test(request, conn, result))
        elif method == 'braveStatus':
            self.start_worker(request, lambda c: self.brave.status())
        elif method == 'braveSync':
            def sync_brave(c):
                destination = self.folder_locations.validate('DOWNLOAD', a['path'])['path']
                return self.brave.sync(a.get('profiles'), destination, a.get('confirmed') is True)
            self.start_worker(request, sync_brave, write=True)
        elif method == 'braveRestore':
            self.start_worker(request, lambda c: self.brave.restore(a.get('profile'), a.get('confirmed') is True), write=True)
        elif method == 'windows':
            self.respond(request, self.app.window_list())
        elif method == 'focusWindow':
            self.respond(request, self.app.focus_window(a.get('id')))
        elif method == 'quit':
            self.respond(request, {'closed':self.app.quit_safely()})
        elif method == 'windowMetadata':
            self.tab_titles = [str(x)[:200] for x in a.get('titles', [])[:200]]
            title = str(a.get('title', 'Files'))[:200]
            self.window.set_title(title + ' — OpenXplorer')
            self.app.broadcast('windowsChanged', {})
            self.respond(request, True)
        elif method == 'tabDragLayout':
            self.tab_drag.update(a)
            self.respond(request, True)
        elif method == 'beginTabDrag':
            self.respond(request, self.tab_drag.begin(a.get('tabId'), a.get('tab')))
        elif method == 'fileDragLayout':
            self.file_drag.update(a)
            self.file_drop.update(a)
            self.respond(request, True)
        elif method == 'beginFileDrag':
            self.respond(request, self.file_drag.begin(a.get('uri'), a.get('uris')))
        elif method == 'moveTabToWindow':
            destination = a.get('windowId')
            if type(destination) is not int or destination == self.window.get_id() or not self.app.transfer_available(destination):
                raise ValueError('Choose another ready OpenXplorer window.')
            token = self.app.tab_transfers.offer(self.window.get_id(), a.get('tabId'), a.get('tab'))
            try:
                self.app.tab_transfers.claim(token, destination, a.get('beforeId'))
                self.app.focus_window(destination)
            except Exception:
                self.app.tab_transfers.cancel(token)
                raise
            self.respond(request, {'pending': True, 'token': token})
        elif method == 'tabTransferReady':
            self.respond(request, self.app.tab_transfers.ready(a.get('token'), self.window.get_id(), a.get('accepted')))
        elif method == 'detachTab':
            snapshot = tab_snapshot(a.get('tab'))
            child = self.app.create_window(snapshot['uri'], self.software_rendering, transfer=snapshot)
            child.handoff = (self, request)
            child.handoff_timer = GLib.timeout_add_seconds(20, child.handoff_timeout)
        elif method == 'handoffReady':
            self.finish_handoff()
            self.respond(request, True)
        elif method == 'desktopRestore':
            def finish_restore(value):
                self.app.disable_reveal()
                self.respond(request, {**value, **self.app.reveal_status()})
            self.start_worker(request, lambda c: self.desktop_integration.restore(), complete=finish_restore)
        elif method == 'discover':
            self.discover_network(request)
        elif method == 'signOut':
            self.sign_out(request)
        elif method == 'normalise':
            self.respond(request, {'uri': normalise_location(a.get('value'), a.get('base'))})
        elif method == 'uiReady':
            self.mark_ui_ready()
            self.respond(request, True)
        elif method == 'newWindow':
            uri = window_location(a.get('uri') or Path.home().as_uri())
            child = self.app.create_window(uri, self.software_rendering)
            self.respond(request, {'id': child.window.get_id()})
        elif method == 'clipboardSet':
            self.respond(request,self.file_clipboard.set(a))
        elif method == 'clipboardGet':
            self.file_clipboard.read(lambda value:self.respond(request,value))
        elif method == 'clipboardConsume':
            self.file_clipboard.consume(a.get('token'),a.get('done',[]),lambda value:self.respond(request,value))
        elif method == 'folderSize':
            token = a.setdefault('token', 'size-' + str(request['id']))
            self.start_worker(request, lambda c: scan_folder(a['uri'], c,
                lambda data: self.emit('folderSizeProgress', {**data, 'token': token})),
                mount_retry=True, pool=self.size_workers)
        elif method == 'properties':
            self.start_worker(request,lambda c:properties(a['uri'],c),mount_retry=True)
        elif method == 'applications':
            self.start_worker(request,lambda c:list_applications(a['uri'],c,bool(a.get('allApps'))),mount_retry=True)
        elif method == 'openTerminal':
            # Only a location crosses the native boundary. A website cannot
            # supply an executable, command, environment or argument list.
            uri = normalise_location(a.get('uri'))
            a['uri'] = uri
            def open_terminal(c):
                prepared = prepare_directory(uri, inspect, local_path,
                                             self.previous_versions.assert_writable, c)
                c.check()
                return launch_terminal(prepared)
            self.start_worker(request, open_terminal, mount_retry=True)
        elif method == 'openWith':
            self.previous_versions.assert_writable(a['uri'])
            self.start_worker(request,lambda c:prepare_launch(a['uri'],a['appId'],c),mount_retry=True,
                              complete=lambda data:self.launch_selected(request,data))
        elif method == 'templates':
            self.start_worker(request,lambda c:list_templates(self.folder_locations.paths()['TEMPLATES'],c))
        elif method == 'createTemplate':
            self.previous_versions.assert_writable(a['uri'])
            self.start_worker(request,lambda c:create_from_template(a['uri'],a['name'],a['template'],self.folder_locations.paths()['TEMPLATES'],c),write=True)
        elif method == 'locationCheck':
            self.start_worker(request,lambda c:self.folder_locations.validate(a['key'],a['value']))
        elif method == 'locationApply':
            self.start_worker(request,lambda c:self.folder_locations.apply(a['key'],a['value'],confirmed=a.get('confirmed') is True),write=True,
                              complete=lambda value:(self.emit('environmentChanged',{}),self.respond(request,value)))
        elif method == 'mountPlan':
            self.respond(request,mount_plan(a['value'],os.getuid(),os.getgid()))
        elif method == 'previousVersions':
            self.start_worker(request,lambda c:self.previous_versions.list(a['uri'],inspect(a['uri'],c)['isDir'],c),mount_retry=True)
        elif method == 'snapshotSource':
            self.respond(request,self.previous_versions.configure(a['live'],a['snapshots'],a.get('layout','direct'),a.get('remove',False)))
            self.emit('environmentChanged',{})
        elif method == 'activateItem':
            self.start_worker(request,lambda c:inspect(a['uri'],c),mount_retry=True,
                              complete=lambda entry:self.resolve_activation(request,entry))
        elif method == 'archiveList':
            self.start_worker(request,lambda c:self.archives.list(a['uri'],a.get('prefix',''),c),mount_retry=True)
        elif method == 'archiveOpenMember':
            self.start_worker(request,lambda c:self.archives.preview_member(a['uri'],a['member'],c),mount_retry=True,
                              complete=lambda value:self.open_archive_preview(request,value))
        elif method == 'archiveInspect':
            self.start_worker(request, lambda c: ZipExtractor(self.archives, GioNode, exclusive_output).inspect(a['uri'], c), mount_retry=True)
        elif method == 'archiveExtract':
            if self.writes:
                raise ValueError('Finish the current file operation before extracting another ZIP.')
            uri = normalise_location(a['uri'])
            target = normalise_location(a['target'])
            self.previous_versions.assert_writable(target)
            if is_smb_server(target):
                raise ValueError('Open a network share before choosing it as an extraction destination.')
            a['uri'], a['target'] = uri, target
            token = a.setdefault('token', 'extract-' + str(request['id']))
            last_emit = [0.0]
            def progress(data):
                now = time.monotonic()
                if now - last_emit[0] >= .08 or data.get('fraction') == 1:
                    last_emit[0] = now
                    self.emit('transfer', {'token': token, **data})
            extractor = ZipExtractor(self.archives, GioNode, exclusive_output, progress)
            # A write is NOT automatically replayed after an authentication/connection error.
            # The user opens/signs in to the source/destination share, then retries.
            self.start_worker(request, lambda c: extractor.extract(uri, target, a['name'], c), write=True)
        elif method == 'preferences':
            value = self.settings_store.update_preferences(a)
            self.apply_native_theme()
            if 'textSize' in a:
                self.app.broadcast('textSizeChanged', {'textSize': value.get('textSize', 100)})
            self.respond(request, value)
        elif method == 'pin':
            items = a.get('items')
            if not isinstance(items, list) or not 1 <= len(items) <= 200:
                raise ValueError('Drag between 1 and 200 folders.')
            clean = []
            for item in items:
                if not isinstance(item, dict):
                    raise ValueError('Invalid folder shortcut.')
                clean.append({'uri': normalise_location(item.get('uri')),
                              'label': safe_label(item.get('label', ''), '')})
            before = normalise_location(a['before']) if a.get('before') else None
            quick = self.environment()['quick']
            known = {p['uri']: p for p in quick}
            def verify_batch(cancel):
                verified = []
                for item in clean:
                    cancel.check()
                    # Existing pins can be reordered even while their NAS is
                    # disconnected. New pins are verified using GIO metadata.
                    verified.append(item if item['uri'] in known else verify_pin(item['uri'], item['label'], cancel))
                return verified
            def pin_complete(verified):
                pins = self.settings_store.pin_many(verified, before=before, quick_order=[p['uri'] for p in self.environment()['quick']])
                self.respond(request, {'pins': pins})
            self.start_worker(request, verify_batch, complete=pin_complete)
        elif method == 'bookmark':
            self.settings_store.bookmark(a.get('action'), a.get('kind'), a.get('uri'), a.get('label', ''))
            self.respond(request, True)
        elif method == 'list':
            uri = normalise_location(a['uri'])
            a['uri'] = uri
            if urlsplit(uri).hostname in self.signing_out_hosts:
                raise ValueError('This server is being signed out. Reopen it after sign-out finishes.')
            self.start_worker(request, lambda c: enumerate_folder(uri, bool(a.get('showHidden')), c,
                lambda batch: self.emit('entries', {'token': a['token'], 'entries': self.previous_versions.annotate(batch)})), mount_retry=True)
        elif method == 'create':
            self.previous_versions.assert_writable(a['uri'])
            self.start_worker(request, lambda c: create_item(a['uri'], a['name'], a['kind'], c), write=True)
        elif method == 'rename':
            self.previous_versions.assert_writable(a['uri'])
            self.start_worker(request, lambda c: rename_item(a['uri'], a['name'], c), write=True)
        elif method == 'operate':
            if self.writes:
                raise ValueError('Another file operation is still running.')
            uris = [require_item_uri(u) for u in a.get('uris', [])]
            target = normalise_location(a['target']) if a.get('target') else None
            if target: self.previous_versions.assert_writable(target)
            if a.get('mode') in ('move','trash','delete'):
                for uri in uris: self.previous_versions.assert_writable(uri)
            if target and is_smb_server(target):
                raise ValueError('Open a network share before pasting files.')
            last_emit = [0.0]
            def progress(data):
                now = time.monotonic()
                if now-last_emit[0] >= .08 or data.get('fraction') == 1:
                    last_emit[0] = now
                    self.emit('transfer', {'token': a['token'], **data})
            engine = TransferEngine(GioNode, progress)
            self.start_worker(request, lambda c: engine.run(a['mode'], uris, target, a.get('policy', 'skip'), c).as_dict(), write=True)
        elif method == 'trashSupport':
            uri = normalise_location(a['uri'])
            self.start_worker(request, lambda c: trash_support(uri, c), mount_retry=True)
        elif method == 'cancel':
            token = a.get('token')
            if token in self.jobs:
                self.jobs[token].cancel()
            self.respond(request, True)
        elif method == 'connect':
            uri = require_share(a.get('address'))
            if urlsplit(uri).hostname in self.signing_out_hosts:
                raise ValueError('Sign-out is in progress. Reconnect after it finishes.')
            label = safe_label(a.get('label', ''), Gio.File.new_for_uri(uri).get_basename() or 'Network folder')
            token = a.get('token', 'request-' + str(request['id']))
            if not isinstance(token, str) or len(token) > 100:
                raise ValueError('Invalid connection token.')
            cancel = self.jobs.setdefault(token, GioCancellation())
            self.request_locations[token] = uri
            self.mount(uri, cancel, lambda error: self.after_connect(request, uri, label, error))
        elif method == 'open':
            self.previous_versions.assert_writable(a['uri'])
            self.start_worker(request,lambda c:prepare_default(a['uri'],c),mount_retry=True,
                              complete=lambda data:self.launch_default(request,data))
        elif method == 'clipboardText':
            text = a.get('text')
            if not isinstance(text, str) or len(text) > 100000:
                raise ValueError('Invalid clipboard text.')
            clipboard = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
            clipboard.set_text(text, -1)
            clipboard.store()
            self.respond(request, True)
        elif method == 'mountVolume':
            volume = next((v for v in self.volume_monitor.get_volumes() if self.volume_id(v) == a.get('id')), None)
            if not volume:
                raise ValueError('This volume is no longer available.')
            existing = volume.get_mount()
            if existing:
                self.respond(request, {'uri': existing.get_root().get_uri()})
            else:
                op = Gtk.MountOperation.new(self.window)
                self.mount_ops[request['id']] = op
                def finished(v, result, *_):
                    self.mount_ops.pop(request['id'], None)
                    try:
                        v.mount_finish(result)
                        mounted = v.get_mount()
                        if mounted is None:
                            raise ValueError('The system did not return a mount for this volume.')
                        self.respond(request, {'uri': mounted.get_root().get_uri()})
                    except Exception as exc:
                        self.respond(request, error=exc)
                volume.mount(Gio.MountMountFlags.NONE, op, None, finished, None)
        elif method == 'unmount':
            if self.writes:
                raise ValueError('Finish the active file operation first.')
            uri = normalise_location(a['uri'])
            mounts = self.volume_monitor.get_mounts()
            file = Gio.File.new_for_uri(uri)
            mount = next((m for m in mounts if file.equal(m.get_root()) or file.has_prefix(m.get_root())), None)
            if mount is None:
                raise ValueError('This location has no active user-session mount.')
            if not mount.can_unmount():
                raise ValueError('The system does not permit unmounting this location.')
            op = Gtk.MountOperation.new(self.window)
            self.mount_ops[request['id']] = op
            def done(m, res, *_):
                self.mount_ops.pop(request['id'], None)
                try:
                    m.unmount_with_operation_finish(res)
                    self.respond(request, True)
                except Exception as exc:
                    self.respond(request, error=exc)
            mount.unmount_with_operation(Gio.MountUnmountFlags.NONE, op, None, done, None)
        elif method == 'chrome':
            x, y, width, height = (int(a.get(k, 0)) for k in ('x', 'y', 'width', 'height'))
            self.drag_area.set_margin_start(max(0, x))
            self.drag_area.set_margin_top(max(0, y))
            self.drag_area.set_size_request(max(1, min(width, 10000)), max(1, min(height, 60)))
            self.respond(request, True)
        elif method == 'window':
            action = a.get('action')
            self.respond(request, True)
            if action == 'minimize':
                self.window.iconify()
            elif action == 'maximize':
                self.toggle_maximize()
            elif action == 'close':
                self.window.close()
            else:
                raise ValueError('Unknown window action.')
        else:
            raise ValueError('Unknown action: ' + method)

    def invalidate_cache_for_write(self,args):
        affected = []
        if args.get('target'): affected.append(args['target'])
        if args.get('uri'):
            affected.append(args['uri'])
            parent = Gio.File.new_for_uri(args['uri']).get_parent()
            if parent: affected.append(parent.get_uri())
        for uri in args.get('uris',[]):
            parent = Gio.File.new_for_uri(uri).get_parent()
            if parent: affected.append(parent.get_uri())
        for root in self.search_index.roots():
            if root['enabled']:
                for uri in set(affected):
                    if below(uri,root['uri']): self.indexer.changed(root['uri'],uri)

    def start_worker(self, request, function, write=False, mount_retry=False, complete=None, pool=None):
        a = request['args']
        token = a.get('token', 'request-' + str(request['id']))
        if not isinstance(token, str) or len(token) > 100:
            raise ValueError('Invalid operation token.')
        cancel = self.jobs.setdefault(token, GioCancellation())
        self.request_locations[token] = a.get('uri', '')
        if write:
            self.writes += 1
        future = (pool or (self.writer if write else self.readers)).submit(function, cancel)
        def completed(f):
            def on_main():
                if write:
                    self.writes -= 1
                try:
                    value = f.result()
                    self.jobs.pop(token, None)
                    self.request_locations.pop(token, None)
                    if write:
                        self.query_workers.submit(self.invalidate_cache_for_write, request['args'])
                    if request['method'] == 'list':
                        self.watch(value['uri'])
                        self.app.remember_network(value['uri'])
                    if complete:
                        complete(value)
                    else:
                        self.respond(request, value)
                except Exception as exc:
                    code = error_payload(exc)['code']
                    if mount_retry and code == 'not-mounted' and not a.get('_mounted_once') and not cancel.is_cancelled():
                        a['_mounted_once'] = True
                        self.mount(a['uri'], cancel, lambda error: self.retry_list(request, function, cancel, token, error, write, complete, pool))
                    else:
                        self.jobs.pop(token, None)
                        self.request_locations.pop(token, None)
                        self.respond(request, error=exc)
                return GLib.SOURCE_REMOVE
            GLib.idle_add(on_main)
        future.add_done_callback(completed)

    def retry_list(self, request, function, cancel, token, error, write=False, complete=None, pool=None):
        if error:
            self.jobs.pop(token, None)
            self.respond(request, error=error)
        elif cancel.is_cancelled():
            self.jobs.pop(token, None)
            self.respond(request, error={'code': 'cancelled', 'message': 'Cancelled.'})
        else:
            # Enumeration failed before emitting entries when NOT_MOUNTED is
            # returned by enumerate_children. Retry once after a native mount.
            if request['method']=='list':
                self.emit('entries', {'token': token, 'entries': [], 'reset': True})
            self.start_worker(request, function, write=write, mount_retry=False, complete=complete, pool=pool)

    def discover_network(self, request):
        token = request['args'].setdefault('token', 'discover-' + str(request['id']))
        cancel = self.jobs.setdefault(token, GioCancellation())
        file = Gio.File.new_for_uri('network:///')
        op = Gio.MountOperation()
        # Discovery may activate GVfs services but may never request credentials.
        def abort_prompt(operation, *_):
            operation.stop_emission_by_name('ask-password')
            operation.reply(Gio.MountOperationResult.ABORTED)
        op.connect('ask-password', abort_prompt)
        self.mount_ops[id(op)] = op
        deadline = [None]
        def expire():
            deadline[0] = None
            cancel.cancel()
            return GLib.SOURCE_REMOVE
        def stop_deadline():
            if deadline[0]:
                GLib.source_remove(deadline[0])
                deadline[0] = None
        deadline[0] = GLib.timeout_add_seconds(15, expire)
        def done(f, result, *_):
            self.mount_ops.pop(id(op), None)
            try:
                f.mount_enclosing_volume_finish(result)
            except GLib.Error as exc:
                if not exc.matches(Gio.io_error_quark(), Gio.IOErrorEnum.ALREADY_MOUNTED):
                    stop_deadline()
                    self.jobs.pop(token, None)
                    self.respond(request, error=exc)
                    return
            def scan(c):
                try:
                    return discover_servers(c)
                finally:
                    GLib.idle_add(lambda: (stop_deadline(), False)[1])
            self.start_worker(request, scan)
        file.mount_enclosing_volume(Gio.MountMountFlags.NONE, op, cancel.raw, done, None)

    def sign_out(self, request):
        if any(c.writes for c in self.app.controllers):
            raise ValueError('Finish active file operations in every OpenXplorer window before signing out.')
        uri = normalise_location(request['args'].get('uri'))
        u = urlsplit(uri)
        if u.scheme != 'smb' or not u.hostname:
            raise ValueError('Select an SMB location to sign out.')
        host = u.hostname
        if host in self.signing_out_hosts:
            raise ValueError('Sign-out is already in progress for this server.')
        forget = request['args'].get('forget', True) is True
        clear_cache = request['args'].get('clearCache', False) is True
        self.app.broadcast('serverSigningOut', {'host':host})
        self.app.visited_network={k:v for k,v in self.app.visited_network.items() if urlsplit(k).hostname!=host}
        for controller in self.app.controllers:
            controller.signing_out_hosts.add(host)
            controller.indexer.pause_server(host)
            controller.prompts.credentials.forget_memory(uri)
            if controller is self:continue
            for token,location in list(controller.request_locations.items()):
                if urlsplit(location).hostname==host and token in controller.jobs:controller.jobs[token].cancel()
            for location,monitor in list(controller.monitors.items()):
                if urlsplit(location).hostname==host:
                    monitor.cancel();controller.monitors.pop(location,None)
                    if location in controller.monitor_debounce:GLib.source_remove(controller.monitor_debounce.pop(location))
        def release_signout():
            for controller in self.app.controllers:controller.signing_out_hosts.discard(host)
            self.app.broadcast('environmentChanged',{})
            return False
        self.indexer.pause_server(host)
        self.prompts.credentials.forget_memory(uri)
        for token, location in list(self.request_locations.items()):
            if urlsplit(location).hostname == host and token in self.jobs:
                self.jobs[token].cancel()
        for location, monitor in list(self.monitors.items()):
            if urlsplit(location).hostname == host:
                monitor.cancel()
                self.monitors.pop(location, None)
                if location in self.monitor_debounce:
                    GLib.source_remove(self.monitor_debounce.pop(location))
        mounts = [m for m in self.volume_monitor.get_mounts()
                  if m.get_root().get_uri().startswith('smb:') and
                  urlsplit(m.get_root().get_uri()).hostname == host]
        count = len(mounts)
        cancel = GioCancellation()
        token = request['args'].setdefault('token', 'signout-' + str(request['id']))
        self.jobs[token] = cancel
        def finish(error=None):
            if error:
                release_signout()
                self.jobs.pop(token, None)
                self.respond(request, error=error)
                return
            def clean(c):
                removed = False
                try:
                    self.prompts.workers.submit(self.prompts.credentials.forget,uri,forget).result(timeout=25)
                    if forget:
                        try:
                            gi.require_version('Secret', '1')
                            from gi.repository import Secret
                        except (ImportError, ValueError) as exc:
                            raise ValueError('Disconnected, but saved credentials could not be removed. Install gir1.2-secret-1 and try Sign out again.') from exc
                        schema = Secret.Schema.new('org.gnome.keyring.NetworkPassword',
                            Secret.SchemaFlags.DONT_MATCH_NAME,
                            {'server': Secret.SchemaAttributeType.STRING, 'protocol': Secret.SchemaAttributeType.STRING})
                        removed = Secret.password_clear_sync(schema, {'server': host, 'protocol': 'smb'}, c.raw)
                    if clear_cache:
                        for r in self.search_index.roots():
                            if urlsplit(r['uri']).hostname == host:
                                self.search_index.clear(r['uri'])
                    return {'host': host, 'disconnected': count, 'credentialsRemoved': bool(removed),
                            'forgotRequested': forget, 'cacheCleared': clear_cache}
                finally:
                    GLib.idle_add(release_signout)
            self.start_worker(request, clean)
        def next_mount():
            if not mounts:
                finish()
                return
            m = mounts.pop()
            if not m.can_unmount():
                finish(ValueError('The system cannot disconnect one of this server’s mounts. Close other applications using it and try again.'))
                return
            op = self.prompts.create(m.get_root().get_uri(), cancel)
            self.mount_ops[id(op)] = op
            def done(mount, res, *_):
                self.mount_ops.pop(id(op), None)
                self.prompts.finish(op)
                try:
                    mount.unmount_with_operation_finish(res)
                except Exception as exc:
                    finish(exc)
                    return
                next_mount()
            m.unmount_with_operation(Gio.MountUnmountFlags.NONE, op, cancel.raw, done, None)
        next_mount()

    def mount(self, uri: str, cancel: GioCancellation, callback):
        file = Gio.File.new_for_uri(uri)
        op = self.prompts.create(uri, cancel)
        key = id(op)
        self.mount_ops[key] = op  # retain while the native prompt is displayed
        def done(f, result, *_):
            self.mount_ops.pop(key, None)
            error = None
            try:
                f.mount_enclosing_volume_finish(result)
            except GLib.Error as exc:
                if not exc.matches(Gio.io_error_quark(), Gio.IOErrorEnum.ALREADY_MOUNTED):
                    error = exc
            self.prompts.finish(op,success=error is None)
            if error is None and urlsplit(uri).hostname: self.indexer.resume_server(urlsplit(uri).hostname)
            callback(error)
        file.mount_enclosing_volume(Gio.MountMountFlags.NONE, op, cancel.raw, done, None)

    def after_connect(self, request, uri, label, error):
        token = request['args'].get('token', 'request-' + str(request['id']))
        self.request_locations.pop(token, None)
        cancel = self.jobs.get(token)
        if cancel and cancel.is_cancelled() and not error:
            error = {'code': 'cancelled', 'message': 'Connection cancelled.'}
        if error:
            self.jobs.pop(token, None)
            self.respond(request, error=error)
            return
        def complete(verified):
            if request['args'].get('remember'):
                self.settings_store.bookmark('add', 'share', verified, label)
            self.app.remember_network(verified)
            self.app.broadcast('environmentChanged', {})
            self.respond(request, {'uri': verified})
        self.start_worker(request, lambda c: verify_folder(uri, c), complete=complete)

    def resolve_activation(self, request, entry):
        action = activation_kind(entry)
        if action == 'directory':
            self.respond(request,{'action':'directory','uri':entry.get('targetUri') or entry['uri'],'entry':entry})
        elif action == 'archive':
            self.respond(request,{'action':'archive','uri':entry['uri'],'entry':entry})
        else:
            self.previous_versions.assert_writable(entry['uri'])
            self.start_worker(request,lambda c:prepare_default(entry['uri'],c),mount_retry=True,
                              complete=lambda data:self.launch_default(request,data))

    def launch_default(self, request, data):
        app,file,entry = data
        try:
            context = self.window.get_display().get_app_launch_context()
            if not app.launch([file],context): raise ValueError('The application did not accept this file.')
            self.settings_store.remember_open(entry)
            self.respond(request,{'action':'opened','uri':entry['uri'],'app':app.get_display_name()})
        except Exception as exc:
            self.respond(request,error=exc)

    def launch_selected(self, request, data):
        app,file,content_type,entry = data
        warning = ''
        try:
            if not app.launch([file],self.window.get_display().get_app_launch_context()):
                raise ValueError('The selected application could not be started.')
            if request['args'].get('makeDefault'):
                if entry['isDir']:
                    warning = 'Opened the folder. Its default file-manager association was not changed.'
                else:
                    try: app.set_as_default_for_type(content_type)
                    except Exception: warning = 'Opened the file, but the default application could not be changed.'
            if not entry['isDir']: self.settings_store.remember_open(entry)
            self.respond(request,{'action':'opened','app':app.get_display_name(),'warning':warning})
        except Exception as exc: self.respond(request,error=exc)

    def open_archive_preview(self, request, value):
        self.emit('notice',{'message':value['warning']})
        # Keep the original request/URI for error reporting, but launch the one
        # intentionally extracted temporary member by its own content type.
        self.start_worker(request,lambda c:prepare_default(value['uri'],c),
                          complete=lambda data:self.launch_default(request,data))

    def watch(self, uri):
        if uri in self.monitors:
            return
        if len(self.monitors) >= 8:
            oldest = next(iter(self.monitors))
            self.monitors.pop(oldest).cancel()
        # Creating a backend monitor can involve I/O. Do it off the GTK thread.
        future = self.readers.submit(lambda: Gio.File.new_for_uri(uri).monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, None))
        def done(f):
            def attach():
                try:
                    monitor = f.result()
                    if self.closed:
                        monitor.cancel()
                        return GLib.SOURCE_REMOVE
                    self.monitors[uri] = monitor
                    monitor.connect('changed', lambda *_: self.changed(uri))
                except GLib.Error:
                    pass  # F5/manual refresh remains available on unsupported SMB servers.
                return GLib.SOURCE_REMOVE
            GLib.idle_add(attach)
        future.add_done_callback(done)

    def changed(self, uri):
        if uri in self.monitor_debounce:
            GLib.source_remove(self.monitor_debounce[uri])
        def send():
            self.monitor_debounce.pop(uri, None)
            if not self.writes:
                self.emit('changed', {'uri': uri})
            return GLib.SOURCE_REMOVE
        self.monitor_debounce[uri] = GLib.timeout_add(350, send)

    @staticmethod
    def volume_id(volume):
        return volume.get_uuid() or volume.get_identifier('unix-device') or volume.get_name()

    def environment(self):
        data = self.settings_store.snapshot()
        quick = []
        builtins = [('Desktop', GLib.UserDirectory.DIRECTORY_DESKTOP, 'desktop', '#3b8ec7'),
                    ('Downloads', GLib.UserDirectory.DIRECTORY_DOWNLOAD, 'downloads', '#138266'),
                    ('Documents', GLib.UserDirectory.DIRECTORY_DOCUMENTS, 'documents', '#4a94d1'),
                    ('Pictures', GLib.UserDirectory.DIRECTORY_PICTURES, 'pictures', '#9a79cb'),
                    ('Music', GLib.UserDirectory.DIRECTORY_MUSIC, 'music', '#c66b9c'),
                    ('Videos', GLib.UserDirectory.DIRECTORY_VIDEOS, 'videos', '#b48540')]
        for label, key, icon, color in builtins:
            folder_key = {'Desktop':'DESKTOP','Downloads':'DOWNLOAD','Documents':'DOCUMENTS','Pictures':'PICTURES','Music':'MUSIC','Videos':'VIDEOS'}[label]
            path = self.folder_locations.paths()[folder_key]
            uri = Path(path).as_uri()
            if uri not in data['hiddenQuick']:
                quick.append({'uri': uri, 'label': label, 'icon': icon, 'color': color, 'folderKey':folder_key})
        for pin in data['pins']:
            if not any(p['uri'] == pin['uri'] for p in quick):
                quick.append(pin)
        rank = {uri: index for index, uri in enumerate(data['quickOrder'])}
        quick.sort(key=lambda p: rank.get(p['uri'], len(rank)))
        mounts = []
        if self.volume_monitor:
            for m in self.volume_monitor.get_mounts():
                if not m.is_shadowed() and m.get_root().get_uri().startswith(('file:', 'smb:')):
                    mounts.append({'label': m.get_name(), 'uri': m.get_root().get_uri(), 'mounted': True})
            for v in self.volume_monitor.get_volumes():
                if not v.get_mount() and v.can_mount():
                    mounts.append({'id': self.volume_id(v), 'label': v.get_name(), 'mounted': False})
        shares = []
        for item in data['shares']:
            file = Gio.File.new_for_uri(item['uri'])
            connected = any(m['mounted'] and (file.equal(Gio.File.new_for_uri(m['uri'])) or file.has_prefix(Gio.File.new_for_uri(m['uri']))) for m in mounts)
            shares.append({**item, 'connected': connected})
        stable = [m for m in read_mounts() if m['fstype'] in ('cifs','smb3')]
        for pin in quick:
            local_mount = mount_for_path(str(Gio.File.new_for_uri(pin['uri']).get_path() or ''), stable)
            pin['isShared'] = pin['uri'].startswith('smb:') or bool(local_mount)
        editors = editor_shortcuts(Gio.AppInfo.get_all())
        result = {'knownFolders':self.folder_locations.snapshot(),'stableMounts':stable,
                  'snapshotRoots':sorted(self.previous_versions.roots()),'editors':editors,
                  'home': Path.home().as_uri(), 'quick': quick, 'mounts': mounts, 'shares': shares,
                  'networkLocations': merge_network_locations(shares, mounts, stable, self.app.visited_network.values()),
                  'recent': data['recent'], 'preferences': data['preferences'], 'version': VERSION,
                  'runtime': RUNTIME, 'windowId': self.window.get_id(), 'nativeTabDrag': self.tab_drag is not None,
                  'nativeFileDrag': self.file_drag is not None,
                  'warning': self.settings_store.warning, 'systemDark': self.system_dark(),
                  'renderer': 'software' if self.software_rendering else 'automatic'}
        if self.initial:
            result['startUri'] = window_location(self.initial)
        return result

    def finish_reveal_test(self, request, connection, result):
        try:
            connection.call_finish(result)
            self.respond(request, {'tested':True})
        except Exception as exc:
            self.respond(request, error=exc)

    def finish_handoff(self):
        if getattr(self, 'handoff_timer', None):
            GLib.source_remove(self.handoff_timer)
            self.handoff_timer = None
        if self.handoff:
            source, request = self.handoff
            self.handoff = None
            source.respond(request, {'ready':True,'windowId':self.window.get_id()})
        self.transfer = None

    def handoff_timeout(self):
        self.handoff_timer = None
        if self.handoff:
            source,request = self.handoff
            self.handoff = None
            source.respond(request, error={'code':'handoff-timeout','message':'The new window did not become ready. The original tab was kept.'})
        return GLib.SOURCE_REMOVE

    def on_delete(self, *_):
        if self.writes:
            self.emit('notice', {'message': 'A file operation is still finishing. Wait or cancel it before closing.'})
            return True
        if self.app.tab_transfers.busy(self.window.get_id()):
            self.emit('notice', {'message': 'A tab is moving. Wait for the handoff or its timeout before closing.'})
            return True
        self.closed = True
        if self.tab_drag: self.tab_drag.close()
        if self.file_drop: self.file_drop.close()
        if self.file_drag: self.file_drag.close()
        if self.handoff: self.handoff_timeout()
        self.prompts.close()
        if self.desktop_settings:
            for handler in self.theme_signals:self.desktop_settings.disconnect(handler)
            self.theme_signals.clear()
        if self.native_css:
            Gtk.StyleContext.remove_provider_for_screen(Gdk.Screen.get_default(),self.native_css)
        if self.volume_monitor:
            for handler in self.volume_signals: self.volume_monitor.disconnect(handler)
            self.volume_signals.clear()
        self.indexer.close()
        if self.index_timer:
            GLib.source_remove(self.index_timer)
            self.index_timer = None
        if self.startup_timeout:
            GLib.source_remove(self.startup_timeout)
            self.startup_timeout = None
        for cancel in self.jobs.values():
            cancel.cancel()
        for monitor in self.monitors.values():
            monitor.cancel()
        for source in self.monitor_debounce.values():
            GLib.source_remove(source)
        self.size_workers.shutdown(wait=False, cancel_futures=True)
        self.readers.shutdown(wait=False, cancel_futures=True)
        self.writer.shutdown(wait=False, cancel_futures=True)
        self.query_workers.shutdown(wait=False, cancel_futures=True)
        return False


class OpenXplorer(Gtk.Application):
    """Single desktop identity, many windows, optional FileManager1 ownership."""
    def __init__(self):
        super().__init__(application_id='io.winspace.Development',
                         flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE | Gio.ApplicationFlags.HANDLES_OPEN)
        self.controllers = []
        self.tab_transfers = TabTransfers(self.transfer_emit, self.transfer_available)
        self.transfer_timer = None
        self.visited_network = {}
        self.directory = Settings().directory
        self.desktop_integration = DesktopIntegration(self.directory)
        self.reveal = RevealRegistration(self.directory)
        self.brave = BraveIntegration(self.directory)
        self.bus_endpoint = None
        self.service_held = False
        for option, description in CLI_OPTIONS.items():
            self.add_main_option(option, 0, GLib.OptionFlags.NONE, GLib.OptionArg.NONE, description, None)
        self.connect('startup', self.startup)
        self.connect('activate', self.activate_app)
        self.connect('open', self.open_files)
        self.connect('command-line', self.command_line)
        self.connect('shutdown', self.shutdown_app)

    def startup(self, *_):
        GLib.set_application_name('OpenXplorer')
        GLib.set_prgname('io.winspace.Development')
        Gtk.Window.set_default_icon_name('io.winspace.Development')
        for name, callback in [('new-window',lambda *_:self.create_window()),
                               ('windows',lambda *_:self.show_windows()),
                               ('settings',lambda *_:self.open_settings()),
                               ('quit',lambda *_:self.quit_safely())]:
            action=Gio.SimpleAction.new(name,None);action.connect('activate',callback);self.add_action(action)
        # Never register a fallback app-menu: it duplicates our HTML tab row.
        self.set_app_menu(None)
        runtime_action = Gio.SimpleAction.new_stateful('runtime-info', None,
            GLib.Variant('s', json.dumps(RUNTIME, sort_keys=True)))
        self.add_action(runtime_action)
        self.bus_endpoint=FileManagerBus(Gio,GLib,self.get_dbus_connection(),self.handle_reveal,
                                        lambda:self.broadcast('integrationChanged',{}))
        if self.reveal.enabled():self.enable_reveal(write=False)
        self.transfer_timer = GLib.timeout_add_seconds(2, self.expire_tab_transfers)

    def transfer_available(self, identifier):
        return any(c.window and c.window.get_id() == identifier and not c.closed and c.ui_ready and not c.writes for c in self.controllers)

    def transfer_emit(self, identifier, name, data):
        for c in self.controllers:
            if c.window and c.window.get_id() == identifier and not c.closed:
                c.emit(name, data)
                return

    def expire_tab_transfers(self):
        self.tab_transfers.expire()
        return GLib.SOURCE_CONTINUE

    def create_window(self, uri=None, software=False, transfer=None):
        controller=OpenXplorerWindow(self,initial=uri,software_rendering=software,transfer=transfer)
        self.controllers.append(controller)
        controller.activate_window()
        self.broadcast('windowsChanged',{})
        return controller

    def active_controller(self):
        window=self.get_active_window()
        return next((c for c in self.controllers if c.window==window and not c.closed),
                    next((c for c in reversed(self.controllers) if not c.closed),None))

    def activate_app(self,*_):
        if not self.controllers:self.create_window()
        elif len([c for c in self.controllers if not c.closed])>1:self.show_windows()
        else:self.controllers[0].window.present()

    def open_settings(self):
        controller=self.active_controller() or self.create_window('settings:')
        controller.window.present()
        if controller.ui_ready:controller.emit('showSettings',{})
        else:controller.initial='settings:'

    def show_windows(self):
        controller=self.active_controller() or self.create_window()
        controller.window.present()
        if controller.ui_ready:controller.emit('showWindows',{})

    def window_list(self):
        return [{'id':c.window.get_id(),'title':c.window.get_title(), 'tabs':c.tab_titles,
                 'active':c.window==self.get_active_window(), 'ready':bool(c.ui_ready and not c.writes)} for c in self.controllers if not c.closed]

    def focus_window(self, identifier):
        c=next((c for c in self.controllers if c.window.get_id()==identifier and not c.closed),None)
        if c is None:raise ValueError('That window is no longer open.')
        c.window.present();return True

    def window_closed(self,controller):
        self.tab_transfers.window_closed(controller.window.get_id())
        if controller in self.controllers:self.controllers.remove(controller)
        self.broadcast('windowsChanged',{})

    def broadcast(self,name,data):
        for c in self.controllers:
            if not c.closed:c.emit(name,data)

    def remember_network(self,uri):
        if not uri.startswith('smb:'):return
        u=urlsplit(uri);parts=u.path.strip('/').split('/')
        root='smb://'+u.netloc+('/'+parts[0] if parts and parts[0] else '/')
        if root not in self.visited_network:
            self.visited_network[root]={'uri':root}
            self.broadcast('environmentChanged',{})

    def open_files(self,_app,files,*_):
        c=self.active_controller() or self.create_window()
        c.on_open_locations(self,files)

    def handle_reveal(self,request,startup_id=''):
        c=self.active_controller()
        if c is None:c=self.create_window()
        if startup_id:
            try:c.window.set_startup_id(startup_id)
            except (TypeError,AttributeError):pass
        c.window.present()
        if c.ui_ready:c.emit('fileManagerRequest',request)
        else:c.external_pending.append(request)

    def reveal_status(self):
        return {'revealEnabled':self.reveal.enabled(),
                'revealOwned':bool(self.bus_endpoint and self.bus_endpoint.owned),
                'revealOwner':self.bus_endpoint.status().get('ownerLabel', '') if self.bus_endpoint else '',
                'portalNote':'Browsers may use a desktop portal. File-picker dialogs remain system dialogs.'}

    def enable_reveal(self,write=True):
        if write:self.reveal.enable()
        if not self.service_held:self.hold();self.service_held=True
        self.bus_endpoint.enable()
        return self.reveal_status()

    def disable_reveal(self):
        result=self.reveal.disable()
        if self.bus_endpoint:self.bus_endpoint.disable()
        if self.service_held:self.release();self.service_held=False
        return {**result,**self.reveal_status()}

    def quit_safely(self):
        if any(c.writes for c in self.controllers if not c.closed):
            self.broadcast('notice',{'message':'Finish or cancel active file operations before quitting OpenXplorer.'})
            return False
        for c in list(self.controllers):
            if not c.closed:c.window.close()
        if any(not c.closed for c in self.controllers):return False
        if self.bus_endpoint:self.bus_endpoint.disable()
        if self.service_held:self.release();self.service_held=False
        self.quit()
        return True

    def shutdown_app(self,*_):
        if self.transfer_timer:
            GLib.source_remove(self.transfer_timer);self.transfer_timer = None
        if self.bus_endpoint:self.bus_endpoint.disable()

    def command_line(self,_app,command):
        try:
            parser=argument_parser()
            argv=command.get_arguments()[1:]
            args=parser.parse_args(argv)
            options=command.get_options_dict()
            for option in CLI_OPTIONS:
                value=options.lookup_value(option, None)
                if value is not None:setattr(args,option.replace('-','_'),bool(value.unpack()))
            if args.quit:
                return 0 if self.quit_safely() else 1
            if args.filemanager_service:
                if self.reveal.enabled():self.enable_reveal(write=False)
                return 0
            uris=[normalise_location(u,base=Path(os.fsdecode(command.get_cwd())).as_uri()) for u in args.location]
            if args.select:
                if not uris:raise ValueError('--select needs a file path.')
                self.handle_reveal({'method':'ShowItems','uris':uris});return 0
            if args.windows:self.show_windows();return 0
            if args.settings:self.open_settings();return 0
            if args.new_window:
                c=self.create_window(uris[0] if uris else None,args.software_rendering)
                if len(uris)>1:c.pending_open.extend(uris[1:])
            elif uris:
                c=self.active_controller() or self.create_window(software=args.software_rendering)
                c.window.present()
                if c.ui_ready:c.emit('openLocations',{'uris':uris})
                else:c.pending_open.extend(uris)
            elif not self.controllers:self.create_window(software=args.software_rendering)
            else:self.activate_app()
            return 0
        except (ValueError,SystemExit) as exc:
            print(str(exc),file=sys.stderr)
            return 2


CLI_OPTIONS = {
    'new-window':'Create a separate OpenXplorer window',
    'windows':'Show existing OpenXplorer windows',
    'settings':'Open Settings',
    'select':'Reveal files in their parent folders',
    'filemanager-service':'Start the opted-in FileManager1 service',
    'software-rendering':'Use software rendering for a new window',
    'quit':'Close all OpenXplorer windows after file operations finish',
}


def argument_parser():
    parser=argparse.ArgumentParser(description='OpenXplorer — Explorer-inspired files for Zorin')
    parser.add_argument('location',nargs='*',help='Local paths or smb:// locations')
    parser.add_argument('--quit',action='store_true',help='Quit all windows safely; integration remains enabled for next login')
    parser.add_argument('--check',action='store_true',help='Check native libraries without opening a window')
    parser.add_argument('--restart',action='store_true',help='Safely quit the current process and launch the installed build; never force active transfers')
    parser.add_argument('--diagnose',action='store_true',help='Print installed and running build identities without filenames or credentials')
    parser.add_argument('--version',action='store_true',help='Print the installed application version')
    parser.add_argument('--new-window',action='store_true',help='Create another window in the same desktop application')
    parser.add_argument('--windows',action='store_true',help='Show existing OpenXplorer windows')
    parser.add_argument('--settings',action='store_true',help='Open Settings')
    parser.add_argument('--select',action='store_true',help='Reveal a file in its parent folder')
    parser.add_argument('--filemanager-service',action='store_true',help='Start the opt-in FileManager1 service without a window')
    parser.add_argument('--software-rendering',action='store_true',help='Use WebKit software rendering')
    return parser


def confirm_restart(status):
    """An explicit upgrade prompt; restarting closes tabs, not files on disk."""
    running = (status.get('running') or {}).get('version', 'an older release')
    dialog = Gtk.MessageDialog(modal=True, message_type=Gtk.MessageType.INFO,
        buttons=Gtk.ButtonsType.NONE, text='Restart OpenXplorer to finish updating')
    dialog.format_secondary_text(
        f"The installed version is {VERSION}; the existing background process is {running}. "
        "A restart closes existing windows. File operations must finish first; they will not be force-stopped.")
    dialog.add_button('Not now', Gtk.ResponseType.CANCEL)
    dialog.add_button('Restart OpenXplorer', Gtk.ResponseType.OK)
    answer = dialog.run()
    dialog.destroy()
    return answer == Gtk.ResponseType.OK


def main():
    args=argument_parser().parse_args()
    if args.version:
        print(f'OpenXplorer {VERSION}'); return 0
    if args.check:
        print(f'OpenXplorer {VERSION}\nGTK {Gtk.get_major_version()}.{Gtk.get_minor_version()}\nWebKitGTK {WebKit2.get_major_version()}.{WebKit2.get_minor_version()}.{WebKit2.get_micro_version()}\nGIO/GVfs: {Gio.Vfs.get_default().__class__.__name__}\nBuild: {RUNTIME["build"]}')
        return 0
    if os.geteuid()==0:
        print('Run OpenXplorer as your regular desktop user, not with sudo.',file=sys.stderr)
        return 1
    try:
        session = Session(Gio, GLib)
        if args.diagnose:
            report = session.status(RUNTIME)
            report['associations'] = DesktopIntegration(Settings().directory).status()
            report['showInFolder'] = FileManagerBus(Gio, GLib, session.connection, lambda *_: None).status()
            report['showInFolder']['enabled'] = RevealRegistration(Settings().directory).enabled()
            print(json.dumps(report, indent=2)); return 0
        if args.quit:
            owner = session.owner()
            if owner: session.stop(owner)
            return 0
        require_current(session, RUNTIME, restart=args.restart,
                        confirm=None if args.filemanager_service else confirm_restart)
    except (GLib.Error, RuntimeError) as exc:
        print(str(exc),file=sys.stderr)
        return 3
    app=OpenXplorer()
    # The restart flag belongs to the fresh launcher, not the primary's parser.
    return app.run([v for v in sys.argv if v != '--restart'])

if __name__=='__main__':
    raise SystemExit(main())
