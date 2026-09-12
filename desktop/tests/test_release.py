# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Packaging-preparation regressions; no native GUI or network operations."""
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from index_service import IndexService

class IndexPolicyTests(unittest.TestCase):
    def setUp(self):
        self.service = object.__new__(IndexService)
        self.service.index = SimpleNamespace(directory=Path('/home/test/.cache/winspace'))

    def policy(self, root):
        with patch('index_service.read_mounts', return_value=[]):
            return self.service.policy(root)

    def test_whole_disk_still_excludes_tmp(self):
        excluded = self.policy('file:///')
        self.assertFalse(self.service.allowed('file:///tmp/private', 'file:///', excluded))

    def test_explicit_tmp_subtree_is_indexable(self):
        root = 'file:///tmp/my-project'
        self.assertTrue(self.service.allowed(root+'/notes.txt', root, self.policy(root)))

    def test_selected_var_excludes_its_tmp_child(self):
        root = 'file:///var'
        self.assertFalse(self.service.allowed(root+'/tmp/file', root, self.policy(root)))

    def test_index_database_remains_excluded(self):
        root = 'file:///home/test'
        self.assertFalse(self.service.allowed(root+'/.cache/winspace/search.sqlite3', root, self.policy(root)))

    def test_nested_mounts_remain_excluded(self):
        root = 'file:///mnt/data'
        with patch('index_service.read_mounts', return_value=[{'path':'/mnt/data/other-volume'}]):
            excluded = self.service.policy(root)
        self.assertFalse(self.service.allowed(root+'/other-volume/file', root, excluded))

if __name__ == '__main__':
    unittest.main()
