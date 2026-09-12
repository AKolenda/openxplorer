# SPDX-License-Identifier: AGPL-3.0-only
"""File drag payload and gesture policy. Mocks do not validate GTK/WebKit."""
from enum import IntFlag
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock

from native_file_drag import NativeFileDrag, URI_INFO, TEXT_INFO, file_uri, layout_value, prepare_files


class PayloadTests(unittest.TestCase):
    def test_paths_spaces_unicode_and_reserved_characters_round_trip(self):
        value = prepare_files(['file:///tmp/Sample%20Files/r%C3%A9sum%C3%A9%20%23%3F%25.txt', 'file:///tmp/Sample%20Files/Folder'])
        self.assertEqual(value.exported, value.uris)
        self.assertEqual(value.text, '/tmp/Sample Files/résumé #?%.txt\n/tmp/Sample Files/Folder')

    def test_deduplicates_without_reordering_selection(self):
        value = prepare_files(['file:///tmp/B', 'file://localhost/tmp/A', 'file:///tmp/B'])
        self.assertEqual(value.uris, ('file:///tmp/B', 'file:///tmp/A'))

    def test_existing_smb_mount_exports_local_path_and_keeps_original(self):
        resolver = Mock(return_value='/run/user/1000/gvfs/smb-share:server=nas.example,share=Projects/Sample.txt')
        value = prepare_files(['smb://nas.example/Projects/Sample.txt'], resolver)
        self.assertEqual(value.uris, ('smb://nas.example/Projects/Sample.txt',))
        self.assertTrue(value.exported[0].startswith('file:///run/user/1000/gvfs/'))
        self.assertEqual(value.remote_only, 0)
        resolver.assert_called_once_with(value.uris[0])

    def test_unmounted_smb_remains_a_uri_without_implicit_download(self):
        value = prepare_files(['smb://nas.example/Projects/Sample.txt'], lambda _: None)
        self.assertEqual(value.exported, value.uris)
        self.assertEqual(value.remote_only, 1)
        self.assertEqual(value.text, value.uris[0])

    def test_local_files_never_consult_mount_resolver(self):
        resolver = Mock(side_effect=AssertionError('Unexpected resolver'))
        prepare_files(['file:///tmp/Sample.txt'], resolver)

    def test_network_share_root_remains_a_reference_for_pinning(self):
        self.assertEqual(file_uri('smb://nas.example/Projects'), 'smb://nas.example/Projects')

    def test_rejects_credentials_controls_virtual_members_and_non_file_schemes(self):
        for uri in ('smb://person:secret@nas.example/Projects/a', 'file:///tmp/a%0D%0Afile:///etc/passwd',
                    'file:///tmp/a%00b', 'https://example.com/a', 'javascript:alert(1)',
                    'archive:file:///tmp/Example.zip!/a.txt', 'zip:///a.txt', 'a.txt', '/tmp/a.txt',
                    'file://remote.example/tmp/a.txt', 'file:///tmp/a?secret=1', 'file:///tmp/a#fragment',
                    'file:relative'):
            with self.subTest(uri=uri), self.assertRaises(ValueError):
                file_uri(uri)

    def test_rejects_non_absolute_or_unsafe_resolver_results(self):
        for path in ('relative', 'file:///tmp/Sample', '/tmp/Example\n.txt', '/tmp/Example\x00.txt', 15):
            with self.subTest(path=path), self.assertRaises(ValueError):
                prepare_files(['smb://nas.example/Projects/a'], lambda _: path)

    def test_rejects_empty_non_lists_and_excessive_selection(self):
        for value in ([], None, 'file:///tmp/a', ['file:///tmp/a'] * 201):
            with self.subTest(value=value), self.assertRaises(ValueError):
                prepare_files(value)

    def test_rejects_oversized_individual_uri_and_total_payload(self):
        with self.assertRaises(ValueError):
            prepare_files(['file:///tmp/' + 'a' * 16384])
        with self.assertRaises(ValueError):
            prepare_files(['file:///tmp/' + str(i) + 'a' * 4000 for i in range(100)])


class FileDragTests(unittest.TestCase):
    def setUp(self):
        self.events = []
        self.view = Mock()
        self.view.get_allocated_width.return_value = 1000
        self.view.drag_check_threshold.return_value = True
        self.view.drag_begin_with_coordinates.return_value = object()
        self.Gtk = NS(TargetList=NS(new=Mock(return_value=Mock())), drag_set_icon_name=Mock(), drag_cancel=Mock())
        self.Gdk = NS(DragAction=IntFlag('Action', {'COPY': 2, 'MOVE': 4}), ModifierType=NS(BUTTON1_MASK=256),
                      EventMask=NS(BUTTON_PRESS_MASK=256, BUTTON_RELEASE_MASK=512, POINTER_MOTION_MASK=4))
        self.controller = NS(webview=self.view, writes=0, ui_ready=True, tab_drag=None,
                             emit=lambda name, data: self.events.append((name, data)))
        self.drag = NativeFileDrag(self.controller, self.Gtk, self.Gdk, None)
        self.layout = {'width': 1000, 'height': 800, 'items': [
            {'uri': 'file:///tmp/Sample.txt', 'left': 10, 'right': 900, 'top': 150, 'bottom': 190}]}
        self.drag.update(self.layout)

    def event(self, x=30, y=165, button=1, state=256):
        event = NS(x=x, y=y, button=button, state=state)
        event.copy = lambda: event
        return event

    def request(self):
        self.drag.button_press(self.view, self.event())
        self.drag.pointer_motion(self.view, self.event(x=70))

    def begin(self, values=None):
        self.request()
        return self.drag.begin('file:///tmp/Sample.txt', values or ['file:///tmp/Sample.txt'])

    def test_uses_gtk_uri_and_text_target_helpers_for_portal_compatibility(self):
        self.drag.targets.add_uri_targets.assert_called_once_with(URI_INFO)
        self.drag.targets.add_text_targets.assert_called_once_with(TEXT_INFO)

    def test_real_primary_threshold_requests_once_and_leaves_click_to_webkit(self):
        self.assertFalse(self.drag.button_press(self.view, self.event()))
        self.view.drag_check_threshold.return_value = False
        self.assertFalse(self.drag.pointer_motion(self.view, self.event(x=35)))
        self.assertFalse(self.events)
        self.view.drag_check_threshold.return_value = True
        self.assertTrue(self.drag.pointer_motion(self.view, self.event(x=70)))
        self.assertTrue(self.drag.pointer_motion(self.view, self.event(x=80)))
        self.assertEqual(self.events, [('fileDragRequest', {'uri': 'file:///tmp/Sample.txt'})])

    def test_secondary_click_blank_area_busy_window_and_tab_drag_never_request_files(self):
        for event in (self.event(button=2), self.event(y=20)):
            self.drag.button_press(self.view, event)
            self.assertIsNone(self.drag.press)
        self.controller.writes = 1
        self.request()
        self.assertFalse(self.events)
        self.controller.writes = 0
        self.controller.tab_drag = NS(context=object())
        self.request()
        self.assertFalse(self.events)

    def test_scaled_geometry_and_clipping(self):
        self.view.get_allocated_width.return_value = 2000
        self.assertEqual(self.drag.item_at(60, 330)['uri'], 'file:///tmp/Sample.txt')
        self.assertIsNone(self.drag.item_at(-1, 330))
        self.assertIsNone(self.drag.item_at(60, 1600))
        self.assertIsNone(self.drag.item_at(1800, 330))

    def test_rejects_script_only_drag_and_selection_omitting_pressed_item(self):
        with self.assertRaises(ValueError):
            self.drag.begin('file:///tmp/Sample.txt', ['file:///tmp/Sample.txt'])
        self.request()
        with self.assertRaises(ValueError):
            self.drag.begin('file:///tmp/Sample.txt', ['file:///tmp/Different.txt'])
        self.view.drag_begin_with_coordinates.assert_not_called()

    def test_release_or_hidden_source_invalidates_pending_async_reply(self):
        for invalidate in (lambda: self.drag.button_release(),
                           lambda: self.drag.update(dict(self.layout, items=[])),
                           lambda: self.drag.pointer_motion(self.view, self.event(state=0))):
            self.drag.update(self.layout)
            self.request()
            invalidate()
            with self.assertRaises(ValueError):
                self.drag.begin('file:///tmp/Sample.txt', ['file:///tmp/Sample.txt'])

    def test_begin_exports_all_selected_items_using_copy_only(self):
        result = self.begin(['file:///tmp/Sample.txt', 'file:///tmp/Folder'])
        self.assertEqual(result, {'started': True, 'count': 2, 'remoteOnly': 0})
        self.assertEqual(self.view.drag_begin_with_coordinates.call_args.args[1], self.Gdk.DragAction.COPY)
        self.assertEqual(self.drag.uris, ('file:///tmp/Sample.txt', 'file:///tmp/Folder'))
        self.assertEqual(self.events[-1][0], 'fileDragStarted')

    def test_native_payload_has_uri_selection_and_separate_compatible_text(self):
        self.begin()
        data = Mock()
        self.drag.data_get(self.view, self.drag.context, data, URI_INFO, 0)
        data.set_uris.assert_called_once_with(['file:///tmp/Sample.txt'])
        self.drag.data_get(self.view, self.drag.context, data, TEXT_INFO, 0)
        data.set_text.assert_called_once_with('/tmp/Sample.txt', -1)

    def test_other_context_cannot_read_data_or_clear_active_drag(self):
        self.begin()
        data = Mock()
        self.view.stop_emission_by_name.reset_mock()
        self.drag.data_get(self.view, object(), data, URI_INFO, 0)
        self.drag.data_delete(self.view, object())
        self.drag.drag_end(self.view, object())
        self.assertFalse(self.drag.drag_failed(self.view, object(), 1))
        data.set_uris.assert_not_called()
        self.view.stop_emission_by_name.assert_not_called()
        self.assertIsNotNone(self.drag.context)

    def test_source_delete_signal_is_suppressed(self):
        self.begin()
        self.drag.data_delete(self.view, self.drag.context)
        self.view.stop_emission_by_name.assert_called_with('drag-data-delete')
        self.assertEqual(self.drag.uris, ('file:///tmp/Sample.txt',))

    def test_escape_and_failure_clear_payload_once_without_claiming_success(self):
        self.begin()
        context = self.drag.context
        self.drag.drag_failed(self.view, context, 2)
        self.drag.drag_end(self.view, context)
        self.drag.drag_end(self.view, context)
        self.assertEqual([data for name, data in self.events if name == 'fileDragFinished'], [{'cancelled': True}])
        self.assertIsNone(self.drag.files)
        self.assertFalse(self.drag.uris)

    def test_gtk_begin_failure_cleans_payload_and_can_retry(self):
        self.view.drag_begin_with_coordinates.return_value = None
        with self.assertRaises(ValueError):
            self.begin()
        self.assertIsNone(self.drag.files)
        self.assertIsNone(self.drag.press)
        self.assertFalse(self.drag.uris)
        self.view.drag_begin_with_coordinates.return_value = object()
        self.assertTrue(self.begin()['started'])

    def test_close_cancels_active_drag_and_disconnects_all_signals(self):
        self.begin()
        context = self.drag.context
        count = len(self.drag.handlers)
        self.drag.close()
        self.Gtk.drag_cancel.assert_called_once_with(context)
        self.assertEqual(self.view.disconnect.call_count, count)
        self.assertFalse(self.drag.available())


class LayoutTests(unittest.TestCase):
    def test_rejects_malformed_layout_without_native_geometry_assumptions(self):
        cases = [None, {}, {'width': float('nan'), 'height': 100}, {'width': True, 'height': 100},
                 {'width': 100, 'height': 0}, {'width': 100, 'height': 100, 'items': 'not a list'},
                 {'width': 100, 'height': 100, 'items': [None]},
                 {'width': 100, 'height': 100, 'items': [{'uri': 'file:///tmp/a', 'left': 10,
                                                       'right': 5, 'top': 0, 'bottom': 10}]}]
        for case in cases:
            with self.subTest(case=case), self.assertRaises(ValueError):
                layout_value(case)


if __name__ == '__main__':
    unittest.main()
