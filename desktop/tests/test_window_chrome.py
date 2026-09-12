# SPDX-License-Identifier: AGPL-3.0-only
"""Native-window policy regressions. No GI or desktop session required."""
import ast
from pathlib import Path
import unittest

HOST = Path(__file__).resolve().parents[1] / 'winspace.py'
TREE = ast.parse(HOST.read_text())
WINDOW = next(n for n in TREE.body if isinstance(n, ast.ClassDef) and n.name == 'OpenXplorerWindow')
ACTIVATE = next(n for n in WINDOW.body if isinstance(n, ast.FunctionDef) and n.name == 'activate_window')
APP = next(n for n in TREE.body if isinstance(n, ast.ClassDef) and n.name == 'OpenXplorer')


def calls(node, name):
    return [n for n in ast.walk(node) if isinstance(n, ast.Call)
            and isinstance(n.func, ast.Attribute) and n.func.attr == name]


class WindowChromeTests(unittest.TestCase):
    def test_fallback_menubar_disabled_on_actual_window(self):
        found = calls(ACTIVATE, 'set_show_menubar')
        self.assertEqual(len(found), 1)
        self.assertEqual(ast.unparse(found[0].func.value), 'self.window')
        self.assertEqual([ast.literal_eval(a) for a in found[0].args], [False])

    def test_policy_is_unconditional_before_window_is_shown(self):
        found = calls(ACTIVATE, 'set_show_menubar')[0]
        self.assertTrue(any(isinstance(n, ast.Expr) and n.value is found for n in ACTIVATE.body))
        for call in calls(ACTIVATE, 'show_all'):
            self.assertLess(found.lineno, call.lineno)

    def test_no_undecorated_window_workaround(self):
        self.assertFalse(calls(WINDOW, 'set_decorated'))
        self.assertTrue(calls(ACTIVATE, 'set_titlebar'))

    def test_title_and_application_identity_kept(self):
        self.assertIn('OpenXplorer', [ast.literal_eval(c.args[0]) for c in calls(ACTIVATE, 'set_title')])
        self.assertIn("application_id='io.winspace.Development'", HOST.read_text())

    def test_actions_remain_but_fallback_model_is_removed(self):
        self.assertEqual(len(calls(APP, 'set_app_menu')), 1)
        startup = next(n for n in APP.body if isinstance(n, ast.FunctionDef) and n.name == 'startup')
        source = ast.get_source_segment(HOST.read_text(), startup)
        self.assertIsNone(ast.literal_eval(calls(APP, 'set_app_menu')[0].args[0]))
        for name in ("'new-window'", "'windows'", "'settings'", "'quit'"):
            self.assertIn(name, source)

    def test_all_new_windows_use_same_creation_path(self):
        create = next(n for n in APP.body if isinstance(n, ast.FunctionDef) and n.name == 'create_window')
        self.assertIn('OpenXplorerWindow(self', ast.get_source_segment(HOST.read_text(), create))
        self.assertEqual(len(calls(create, 'activate_window')), 1)
        self.assertEqual(len(calls(TREE, 'ApplicationWindow')), 1)


if __name__ == '__main__':
    unittest.main()
