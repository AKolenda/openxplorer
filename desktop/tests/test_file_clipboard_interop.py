# SPDX-License-Identifier: AGPL-3.0-only
"""Clipboard protocol/async regressions with doubles; no native GTK claim."""
from pathlib import Path
import sys
from types import SimpleNamespace as NS
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from file_clipboard import FileClipboard, CUSTOM, GNOME, URI_LIST, KDE_CUT, decode_clipboard, encode_clipboard


ONE = 'file:///home/demo/Read%20me.txt'
TWO = 'file:///home/demo/Planning.pdf'
URI_PAYLOAD = (ONE + '\r\n' + TWO + '\r\n').encode()


class ClipboardHarness:
    """Drive real async read/consume methods through queued selection replies."""
    def __init__(self, formats):
        self.formats = formats
        self.pending = []
        self.requested = []
        self.writes = []
        self.clip = FileClipboard.__new__(FileClipboard)
        self.clip._generation = 0
        self.clip.Gdk = NS(Atom=NS(intern=lambda name, *_: name))
        self.clip.clipboard = self
        self.clip.set = self.set_files

    def request_contents(self, mime, callback, _data):
        self.requested.append(mime)
        self.pending.append((callback, self.formats.get(mime)))

    def reply(self):
        callback, payload = self.pending.pop(0)
        callback(self, NS(get_data=lambda: payload))

    def drain(self):
        while self.pending:
            self.reply()

    def replace(self, formats):
        self.formats = formats
        self.clip._generation += 1

    def set_files(self, value):
        self.writes.append(value.copy())
        self.replace(encode_clipboard(value))
        return value

    def set_text(self, text, _length):
        self.writes.append(text)
        self.replace({'text/plain': text.encode()})

    def read(self):
        values = []
        self.clip.read(values.append)
        self.drain()
        return values[0]

    def consume(self, token, done):
        values = []
        self.clip.consume(token, done, values.append)
        self.drain()
        return values[0]


class ExternalClipboardTests(unittest.TestCase):
    def test_gnome_cut_can_consume_successful_items_across_reads(self):
        board = ClipboardHarness({GNOME: ('cut\n' + ONE + '\n' + TWO).encode()})
        initial = board.read()
        self.assertEqual(board.read()['token'], initial['token'])
        remaining = board.consume(initial['token'], [ONE])
        self.assertEqual(remaining['uris'], [TWO])
        self.assertEqual(remaining['mode'], 'move')
        self.assertEqual(board.read()['uris'], [TWO])
        self.assertIsNone(board.consume(remaining['token'], [TWO]))
        self.assertEqual(board.writes[-1], '')

    def test_changed_external_payload_is_not_consumed_by_old_operation(self):
        board = ClipboardHarness({GNOME: ('cut\n' + ONE).encode()})
        initial = board.read()
        board.replace({GNOME: ('cut\n' + TWO).encode()})
        current = board.consume(initial['token'], [ONE])
        self.assertEqual(current['uris'], [TWO])
        self.assertNotEqual(current['token'], initial['token'])
        self.assertEqual(board.writes, [])

    def test_changed_mode_is_not_consumed_by_old_cut(self):
        board = ClipboardHarness({GNOME: ('cut\n' + ONE).encode()})
        initial = board.read()
        board.replace({GNOME: ('copy\n' + ONE).encode()})
        self.assertEqual(board.consume(initial['token'], [ONE])['mode'], 'copy')
        self.assertEqual(board.writes, [])

    def test_kde_cut_imports_as_move_and_consumes(self):
        board = ClipboardHarness({URI_LIST: URI_PAYLOAD, KDE_CUT: b'1'})
        initial = board.read()
        self.assertEqual(initial['mode'], 'move')
        self.assertEqual(board.read()['token'], initial['token'])
        self.assertEqual(board.consume(initial['token'], [ONE])['uris'], [TWO])

    def test_uri_list_without_exact_kde_cut_marker_remains_copy(self):
        for marker in (None, b'0', b'', b'cut', b'1\n', b'1' * 20):
            with self.subTest(marker=marker):
                board = ClipboardHarness({URI_LIST: URI_PAYLOAD, KDE_CUT: marker})
                self.assertEqual(board.read()['mode'], 'copy')

    def test_nul_terminated_kde_cut_marker(self):
        board = ClipboardHarness({URI_LIST: URI_PAYLOAD, KDE_CUT: b'1\0'})
        self.assertEqual(board.read()['mode'], 'move')

    def test_kde_marker_cannot_turn_plain_text_into_files(self):
        board = ClipboardHarness({'text/plain': ONE.encode(), KDE_CUT: b'1'})
        self.assertIsNone(board.read())
        self.assertNotIn(KDE_CUT, board.requested)

    def test_kde_marker_cannot_turn_invalid_uri_list_into_files(self):
        board = ClipboardHarness({URI_LIST: b'https://example.test/file', KDE_CUT: b'1'})
        self.assertIsNone(board.read())
        self.assertNotIn(KDE_CUT, board.requested)

    def test_custom_payload_keeps_priority_and_its_token(self):
        value = {'mode': 'copy', 'uris': [ONE], 'token': 'existing-openxplorer-token'}
        board = ClipboardHarness({**encode_clipboard(value), KDE_CUT: b'1'})
        self.assertEqual(board.read(), value)
        self.assertEqual(board.requested, [CUSTOM])

    def test_gnome_copy_ignores_unrelated_kde_cut_marker(self):
        board = ClipboardHarness({GNOME: ('copy\n' + ONE).encode(), URI_LIST: URI_PAYLOAD, KDE_CUT: b'1'})
        self.assertEqual(board.read()['mode'], 'copy')
        self.assertNotIn(KDE_CUT, board.requested)

    def test_owner_change_between_uri_list_and_marker_rejects_mixed_clipboard(self):
        board = ClipboardHarness({URI_LIST: URI_PAYLOAD, KDE_CUT: b'1'})
        values = []
        board.clip.read(values.append)
        while board.requested[-1] != KDE_CUT:
            board.reply()
        board.replace({'text/plain': b'New clipboard owner'})
        board.drain()
        self.assertEqual(values, [None])
        self.assertEqual(board.writes, [])

    def test_owner_change_during_consume_cannot_clear_new_clipboard(self):
        board = ClipboardHarness({GNOME: ('cut\n' + ONE).encode()})
        initial = board.read()
        values = []
        board.clip.consume(initial['token'], [ONE], values.append)
        board.replace({'text/plain': b'Keep this text'})
        board.drain()
        self.assertEqual(values, [None])
        self.assertEqual(board.writes, [])

    def test_direct_uri_decode_defaults_to_copy_even_for_exported_cut(self):
        value = {'mode': 'move', 'uris': [ONE], 'token': 'original'}
        self.assertEqual(decode_clipboard(URI_LIST, encode_clipboard(value)[URI_LIST])['mode'], 'copy')


if __name__ == '__main__':
    unittest.main()
