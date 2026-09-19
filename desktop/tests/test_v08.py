# SPDX-License-Identifier: AGPL-3.0-only
"""Rebrand compatibility and licensing checks; no native GUI needed."""
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET
from core import VERSION
from desktop_integration import APP_ID
from reveal_integration import MARKER, SERVICE, AUTOSTART
ROOT=Path(__file__).resolve().parents[1]
class RebrandTests(unittest.TestCase):
    def test_version(self):self.assertEqual(VERSION,'1.1.2')
    def test_desktop_id_retained(self):self.assertEqual(APP_ID,'io.winspace.Development.desktop')
    def test_marker_retained(self):self.assertEqual(MARKER,'# Managed by Winspace: file-manager-integration v1\n')
    def test_previous_service_exact(self):self.assertEqual(SERVICE,MARKER+'[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec=/usr/bin/winspace --filemanager-service\n')
    def test_previous_autostart_recognized(self):self.assertIn('Name=Winspace Show in Folder integration\n',AUTOSTART)
    def test_original_notice_kept(self):self.assertIn('Copyright (c) 2026 Winspace contributors',(ROOT/'licenses/Winspace-MIT.txt').read_text())
    def test_full_agpl(self):
        text=(ROOT/'LICENSE').read_text();self.assertGreater(len(text),30000)
        self.assertIn('certain numbered version of the GNU Affero General\nPublic License',text)
        for marker in ['GNU AFFERO GENERAL PUBLIC LICENSE','13. Remote Network Interaction','17. Interpretation','END OF TERMS AND CONDITIONS']:self.assertIn(marker,text)
    def test_appstream_license_and_name(self):
        meta=ET.parse(ROOT/'packaging/io.winspace.Development.metainfo.xml').getroot()
        self.assertEqual(meta.findtext('name'),'OpenXplorer');self.assertEqual(meta.findtext('project_license'),'AGPL-3.0-only')
    def test_visible_legal_notices(self):
        ui=(ROOT/'ui/app.js').read_text();self.assertIn("label:'License & source'",ui);self.assertIn('No warranty.',ui)
    def test_public_source_runner(self):self.assertIn("with_name('winspace.py')",(ROOT/'openxplorer.py').read_text());self.assertIn("runpy.run_path",(ROOT/'openxplorer.py').read_text())
