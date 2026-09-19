# SPDX-License-Identifier: AGPL-3.0-only
"""Native launch-shell lifecycle contracts without a display or user state."""
import ast
from pathlib import Path
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock


class Widget:
    def __init__(self, kind, **kwargs):
        self.kind, self.text = kind, kwargs.get('label', '')
        self.children, self.classes, self.handlers = [], [], {}
        self.visible = self.no_show_all = False
        self.can_focus = kind == 'Button'

    def get_style_context(self): return NS(add_class=self.classes.append)
    def get_accessible(self): return NS(set_name=lambda name: None)
    def add(self, child): self.children.append(child)
    def pack_start(self, child, *_): self.children.append(child)
    def connect(self, event, handler): self.handlers[event] = handler
    def set_text(self, text): self.text = text
    def set_no_show_all(self, value): self.no_show_all = value
    def show(self): self.visible = True
    def hide(self): self.visible = False
    def show_all(self):
        if not self.no_show_all:
            self.show()
            for child in self.children: child.show_all()
    def __getattr__(self, name):
        if name.startswith('set_'): return lambda *args: None
        raise AttributeError(name)


METHODS = {'create_startup_cover', 'show_startup_error', 'reset_startup_cover',
           'begin_ui_load', 'startup_timed_out', 'retry_startup', 'on_load_failed', 'mark_ui_ready'}
SOURCE = Path(__file__).resolve().parents[1] / 'winspace.py'
TREE = ast.parse(SOURCE.read_text())
WINDOW = next(node for node in TREE.body if isinstance(node, ast.ClassDef) and node.name == 'OpenXplorerWindow')


class StartupTests(unittest.TestCase):
    def setUp(self):
        gtk = NS(**{name: (lambda kind: lambda **kwargs: Widget(kind, **kwargs))(name)
                    for name in ('Box', 'EventBox', 'Button', 'Label')},
                 Align=NS(CENTER=0, FILL=1, START=2, END=3),
                 Orientation=NS(VERTICAL=0, HORIZONTAL=1))
        self.glib = NS(SOURCE_REMOVE=False, source_remove=Mock(),
                       timeout_add_seconds=Mock(return_value=72), timeout_add=Mock())
        scope = {'Gtk': gtk, 'GLib': self.glib, 'APP_URI': 'file:///fictional/ui/index.html',
                 'WebKit2': NS(HardwareAccelerationPolicy=NS(NEVER='never'))}
        methods = [node for node in WINDOW.body if isinstance(node, ast.FunctionDef) and node.name in METHODS]
        exec(compile(ast.Module(body=methods, type_ignores=[]), str(SOURCE), 'exec'), scope)
        self.host = type('StartupHost', (), {name: scope[name] for name in METHODS})()
        self.host.window = NS(close=Mock())
        self.host.webview = Mock()
        self.host.ui_ready = self.host.closed = self.host.software_rendering = False
        self.host.writes = 0
        self.host.startup_timeout = None
        self.host.emit = Mock()
        self.host.pending_open, self.host.external_pending = [], []
        self.host.transfer = None
        self.host.index_timer = 10
        self.host.request_initial_paint = Mock()
        self.host.create_startup_cover().show_all()

    def visible_widgets(self, root=None):
        root = root or self.host.loading_cover
        if root.visible:
            yield root
            for child in root.children: yield from self.visible_widgets(child)

    def test_initial_surface_is_static_shell_without_text_or_focus_targets(self):
        widgets = list(self.visible_widgets())
        classes = {style for widget in widgets for style in widget.classes}
        self.assertTrue({'startup-titlebar', 'startup-navigation', 'startup-sidebar',
                         'startup-content', 'startup-footer'} <= classes)
        self.assertFalse(any(widget.kind == 'Spinner' or widget.can_focus for widget in widgets))
        self.assertFalse(any(widget.text for widget in widgets))
        self.assertFalse(self.host.startup_notice.visible)

    def test_timeout_reveals_recovery_without_dismissing_cover(self):
        self.host.startup_timed_out()
        self.assertTrue(self.host.loading_cover.visible)
        self.assertFalse(self.host.startup_rows.visible)
        self.assertIn('longer than expected', self.host.startup_label.text)
        labels = [widget.text for widget in self.visible_widgets() if widget.can_focus]
        self.assertEqual(labels, ['Retry', 'Retry with software rendering', 'Close'])

    def test_load_failure_keeps_recovery_visible_and_cancels_timeout(self):
        self.host.startup_timeout = 23
        self.assertTrue(self.host.on_load_failed(None, None, None, RuntimeError('fictional failure')))
        self.glib.source_remove.assert_called_once_with(23)
        self.assertIn('fictional failure', self.host.startup_label.text)
        self.assertTrue(self.host.startup_notice.visible)
        self.assertIsNone(self.host.startup_timeout)

    def test_software_retry_restores_skeleton_and_rearms_recovery(self):
        self.host.startup_timed_out()
        self.host.retry_startup(True)
        self.assertTrue(self.host.software_rendering)
        self.host.webview.get_settings().set_hardware_acceleration_policy.assert_called_once_with('never')
        self.host.webview.load_uri.assert_called_once_with('file:///fictional/ui/index.html')
        self.assertTrue(self.host.startup_rows.visible)
        self.assertFalse(self.host.startup_notice.visible)
        self.assertEqual(self.host.startup_label.text, '')
        self.assertEqual(self.host.startup_timeout, 72)
        self.assertFalse(any(widget.can_focus for widget in self.visible_widgets()))

    def test_only_existing_ui_ready_handoff_hides_cover_and_delivers_pending(self):
        self.host.pending_open = ['file:///fictional/Documents']
        self.host.external_pending = [{'method': 'ShowFolders', 'uris': ['file:///fictional/Documents']}]
        self.host.startup_timeout = 23
        self.host.mark_ui_ready()
        self.assertTrue(self.host.ui_ready)
        self.assertFalse(self.host.loading_cover.visible)
        self.assertEqual(self.host.pending_open, [])
        self.assertEqual(self.host.external_pending, [])
        self.glib.source_remove.assert_called_once_with(23)
        self.assertEqual(self.host.emit.call_count, 2)
        self.host.mark_ui_ready()
        self.assertEqual(self.host.emit.call_count, 2)

    def test_ready_or_busy_windows_cannot_reload_from_recovery(self):
        for state in ('ui_ready', 'writes'):
            with self.subTest(state=state):
                setattr(self.host, state, True)
                self.host.retry_startup(True)
                self.host.webview.load_uri.assert_not_called()
                setattr(self.host, state, False)

    def test_late_timeout_does_not_cover_ready_window(self):
        self.host.mark_ui_ready()
        self.host.startup_timed_out()
        self.assertFalse(self.host.loading_cover.visible)
        self.assertFalse(self.host.startup_notice.visible)


if __name__ == '__main__':
    unittest.main()
