# SPDX-License-Identifier: AGPL-3.0-only
import unittest
from enum import IntFlag
from types import SimpleNamespace as NS
from unittest.mock import Mock

from native_file_drop import NativeFileDrop, drop_layout, decode_uris, INFO


class PayloadTests(unittest.TestCase):
    def test_multiple_encoded_uris_and_comments(self):
        self.assertEqual(decode_uris(b'# comment\r\nfile:///tmp/Read%20me.txt\r\nfile:///tmp/caf%C3%A9.txt\r\nfile:///tmp/Read%20me.txt\r\n'),
                         ['file:///tmp/Read%20me.txt', 'file:///tmp/caf%C3%A9.txt'])

    def test_rejects_text_urls_credentials_virtual_entries_and_controls(self):
        for payload in (b'/tmp/file', b'https://example.org/file', b'smb://user:secret@studio-nas/Shared/file',
                        b'archive:///tmp/a.zip/file', b'file:///tmp/a%0Ab',
                        b'file://elsewhere/tmp/file', b'file:///tmp/a\nrelative', b'\xff', b'x' * 1048577,
                        b'file:///tmp/a\n' * 201):
            with self.subTest(payload=payload[:60]), self.assertRaises(ValueError):
                decode_uris(payload)

    def test_rejects_invalid_layout(self):
        for change in ({'width': float('nan')}, {'height': True}, {'targets': [{'kind': 'move'}]},
                       {'targets': [{'kind': 'copy', 'uri': 'smb://studio-nas', 'left': 0, 'right': 50, 'top': 0, 'bottom': 50}]},
                       {'targets': [{'kind': 'pin', 'left': 0, 'right': 101, 'top': 0, 'bottom': 50}]}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                drop_layout({'width': 100, 'height': 100, 'targets': [], **change})


class DropTests(unittest.TestCase):
    def setUp(self):
        self.receiver = NativeFileDrop.__new__(NativeFileDrop)
        r = self.receiver
        r.view = Mock()
        r.view.get_allocated_width.return_value = 200
        r.view.get_allocated_height.return_value = 200
        r.Gtk = NS(drag_finish=Mock(), drag_get_source_widget=Mock(return_value=None))
        r.Gdk = NS(DragAction=IntFlag('Action', {'COPY': 2, 'MOVE': 4}), drag_status=Mock())
        r.GLib = NS(timeout_add_seconds=Mock(return_value=3), source_remove=Mock(), SOURCE_REMOVE=False)
        r.c = NS(closed=False, ui_ready=True, writes=0, emit=Mock(), previous_versions=NS(assert_writable=Mock()), app=NS(controllers=[]))
        r.pending = r.timer = None
        r.atom = 'uri-list'
        r.update({'width': 100, 'height': 100, 'targets': [
            {'kind': 'copy', 'uri': 'file:///tmp/Target', 'left': 10, 'right': 90, 'top': 20, 'bottom': 90}]})
        self.context = Mock()
        self.context.list_targets.return_value = [NS(name=lambda: 'text/uri-list')]
        self.context.get_actions.return_value = r.Gdk.DragAction.COPY | r.Gdk.DragAction.MOVE

    def drop(self):
        r = self.receiver
        self.assertTrue(r.drop(r.view, self.context, 50, 60, 7))

    def receive(self, payload=b'file:///tmp/Source.txt\r\n'):
        r = self.receiver
        r.received(r.view, self.context, 50, 60, NS(get_data=lambda: payload), INFO, 8)

    def test_copy_only_and_scaled_target(self):
        r = self.receiver
        r.motion(r.view, self.context, 50, 60, 7)
        r.Gdk.drag_status.assert_called_once_with(self.context, r.Gdk.DragAction.COPY, 7)
        self.drop()
        r.view.drag_get_data.assert_called_once_with(self.context, r.atom, 7)
        self.receive()
        r.Gtk.drag_finish.assert_called_once_with(self.context, True, False, 7)
        r.c.emit.assert_any_call('fileDrop', {'uris': ['file:///tmp/Source.txt'], 'kind': 'copy', 'target': 'file:///tmp/Target', 'before': None})
        r.c.previous_versions.assert_writable.assert_called_once_with('file:///tmp/Target')
        self.assertIsNone(r.pending)

    def test_move_only_source_rejected_without_delete(self):
        r = self.receiver
        self.context.get_actions.return_value = r.Gdk.DragAction.MOVE
        self.drop()
        r.Gtk.drag_finish.assert_called_once_with(self.context, False, False, 7)
        r.view.drag_get_data.assert_not_called()

    def test_same_process_uses_original_remote_uris(self):
        r = self.receiver
        widget = object()
        r.Gtk.drag_get_source_widget.return_value = widget
        r.c.app.controllers = [NS(webview=widget, file_drag=NS(context=object(), uris=('smb://studio-nas/Shared/Read.txt',),
            files=NS(exported=('file:///run/user/1000/gvfs/Read.txt',))))]
        self.drop(); self.receive(b'file:///run/user/1000/gvfs/Read.txt\r\n')
        self.assertEqual(r.c.emit.call_args.args[1]['uris'], ['smb://studio-nas/Shared/Read.txt'])

    def test_late_drop_never_substitutes_another_selection(self):
        r = self.receiver
        widget = object()
        r.Gtk.drag_get_source_widget.return_value = widget
        r.c.app.controllers = [NS(webview=widget, file_drag=NS(context=object(), uris=('smb://studio-nas/Shared/Other.txt',),
            files=NS(exported=('file:///tmp/Other.txt',))))]
        self.drop(); self.receive()
        self.assertEqual(r.c.emit.call_args.args[1]['uris'], ['file:///tmp/Source.txt'])

    def test_invalid_payload_finishes_failure_without_operation(self):
        self.drop(); self.receive(b'https://example.org/download')
        self.receiver.Gtk.drag_finish.assert_called_once_with(self.context, False, False, 7)
        self.assertFalse(any(call.args[0] == 'fileDrop' for call in self.receiver.c.emit.call_args_list))

    def test_navigation_or_modal_after_drop_rejects_stale_target(self):
        self.drop()
        self.receiver.update({'width': 100, 'height': 100, 'targets': []})
        self.receive()
        self.receiver.Gtk.drag_finish.assert_called_once_with(self.context, False, False, 7)

    def test_snapshot_destination_rejected(self):
        self.receiver.c.previous_versions.assert_writable.side_effect = ValueError('Read only')
        self.drop(); self.receive()
        self.receiver.Gtk.drag_finish.assert_called_once_with(self.context, False, False, 7)

    def test_share_root_cannot_be_copied(self):
        self.drop(); self.receive(b'smb://studio-nas/Shared\r\n')
        self.receiver.Gtk.drag_finish.assert_called_once_with(self.context, False, False, 7)

    def test_share_root_can_be_pinned(self):
        r = self.receiver
        r.update({'width': 100, 'height': 100, 'targets': [
            {'kind': 'pin', 'left': 10, 'right': 90, 'top': 20, 'bottom': 90}]})
        self.drop(); self.receive(b'smb://studio-nas/Shared\r\n')
        r.Gtk.drag_finish.assert_called_once_with(self.context, True, False, 7)
        r.c.emit.assert_any_call('fileDrop', {'uris': ['smb://studio-nas/Shared'], 'kind': 'pin', 'target': None, 'before': None})

    def test_busy_window_rejects(self):
        self.receiver.c.writes = 1
        self.drop()
        self.receiver.view.drag_get_data.assert_not_called()

    def test_late_or_duplicate_selection_does_not_repeat_drop(self):
        self.drop(); self.receive(); self.receive()
        self.assertEqual(sum(c.args[0] == 'fileDrop' for c in self.receiver.c.emit.call_args_list), 1)

    def test_timeout_releases_native_handshake(self):
        self.drop()
        self.receiver.expire()
        self.receive()
        self.receiver.Gtk.drag_finish.assert_called_once_with(self.context, False, False, 7)

    def test_tab_drag_is_left_to_tab_transport(self):
        self.context.list_targets.return_value = [NS(name=lambda: 'application/x-openxplorer-tab-v1')]
        self.assertFalse(self.receiver.motion(self.receiver.view, self.context, 50, 60, 7))
        self.assertFalse(self.receiver.drop(self.receiver.view, self.context, 50, 60, 7))
        self.receiver.Gtk.drag_finish.assert_not_called()


if __name__ == '__main__':
    unittest.main()
