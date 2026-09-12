#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Verify package structure, checksums, permissions and source syntax, without installing.

This is not a native GUI/SMB runtime test. Use winspace --check on the target
machine and read TEST-REPORT.md for the native validation boundary.
"""
from __future__ import annotations
import argparse
import ast
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from core import VERSION, DEBIAN_VERSION
from tools.build_deb import REQUIRED


def verify(package: Path) -> dict:
    checks = []
    def check(name, condition):
        if not condition:
            raise ValueError(name)
        checks.append(name)
    fields = subprocess.check_output(['dpkg-deb', '--field', str(package)], text=True)
    control = dict(line.split(': ', 1) for line in fields.splitlines() if ': ' in line and not line.startswith(' '))
    check('Package identity', control.get('Package') == 'openxplorer')
    check('Version matches the source', control.get('Version') == DEBIAN_VERSION)
    check('Replaces legacy package without deleting user data', 'winspace-explorer' in control.get('Replaces', '') and 'winspace-explorer' in control.get('Conflicts', ''))
    check('Official project homepage in package', control.get('Homepage') == 'https://openxplorer.app')
    check('Architecture-independent application', control.get('Architecture') == 'all')
    dependencies = control.get('Depends', '')
    check('Native runtime dependencies declared', all(x in dependencies for x in ('python3-gi', 'gir1.2-gtk-3.0', 'gir1.2-webkit2-4.1', 'gir1.2-secret-1', 'gvfs-backends', 'gvfs-fuse', 'xdg-utils', 'xdg-user-dirs')))
    payload = subprocess.check_output(['dpkg-deb', '--fsys-tarfile', str(package)])
    with tarfile.open(fileobj=io.BytesIO(payload), mode='r:') as archive:
        members = archive.getmembers()
        check('No absolute/traversing paths or link payloads', all(not m.name.startswith('/') and '..' not in PurePosixPath(m.name).parts and not m.issym() and not m.islnk() for m in members))
        check('All payload entries are owned by root:root', all(m.uid == 0 and m.gid == 0 for m in members))
        check('Payload is not group/world writable', all(not (m.mode & 0o022) for m in members))
        paths = {m.name.removeprefix('./'): m for m in members}
        required_app = [name for name in REQUIRED if not name.startswith('tools/')]
        check('All required application files included', all('opt/openxplorer/' + name in paths for name in required_app))
        check('Both installed launchers are executable', all(paths.get('usr/bin/' + name) and paths['usr/bin/' + name].mode & 0o111 for name in ('openxplorer', 'openxplorer-mount-share', 'winspace', 'winspace-mount-share')))
        check('Folder icon and desktop registration included', all(name in paths for name in ('usr/share/applications/io.winspace.Development.desktop', 'usr/share/icons/hicolor/scalable/apps/io.winspace.Development.svg')))
        check('No personal settings, cache or bytecode bundled', all(not name.startswith(('home/', 'root/', 'run/')) and not name.endswith(('.pyc', '.sqlite3')) and 'settings.json' not in name for name in paths))
    with tempfile.TemporaryDirectory(prefix='winspace-verify-') as tmp:
        stage = Path(tmp)
        subprocess.run(['dpkg-deb', '--extract', str(package), str(stage)], check=True)
        subprocess.run(['dpkg-deb', '--control', str(package), str(stage / 'DEBIAN')], check=True)
        app = stage / 'opt/openxplorer'
        for source in app.glob('*.py'):
            ast.parse(source.read_text(), filename=source.name, feature_version=(3, 10))
        check('All packaged Python sources parse as Python 3.10+', True)
        check('Packaged UI bytes match this release source', all(
            (app / 'ui' / name).read_bytes() == (ROOT / 'ui' / name).read_bytes()
            for name in ('index.html', 'bootstrap.js', 'type-select.js', 'snapshot-meta.js', 'text-size.js', 'app.js', 'style.css', 'winspace.svg')))
        check('Packaged Python bytes match this release source', all(source.read_bytes() == (ROOT / source.name).read_bytes() for source in app.glob('*.py')))
        host_tree = ast.parse((app / 'winspace.py').read_text())
        window_class = next(n for n in host_tree.body if isinstance(n, ast.ClassDef) and n.name == 'OpenXplorerWindow')
        activate = next(n for n in window_class.body if isinstance(n, ast.FunctionDef) and n.name == 'activate_window')
        policy = [n for n in activate.body if isinstance(n, ast.Expr) and isinstance(n.value, ast.Call)
                  and isinstance(n.value.func, ast.Attribute) and n.value.func.attr == 'set_show_menubar']
        check('Packaged window disables duplicate fallback menu before showing',
              len(policy) == 1 and ast.unparse(policy[0].value.func.value) == 'self.window'
              and len(policy[0].value.args) == 1 and isinstance(policy[0].value.args[0], ast.Constant)
              and policy[0].value.args[0].value is False
              and all(policy[0].lineno < n.lineno for n in ast.walk(activate)
                      if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == 'show_all'))
        html = (app / 'ui/index.html').read_text()
        check('Type selector script loads before its consumer',
              html.index('<script src="type-select.js">') < html.index('<script src="app.js">'))
        check('Text-size module loads before its consumer',
              html.index('<script src="text-size.js">') < html.index('<script src="app.js">'))
        check('ZIP extraction and destination writer shipped',
              (app/'zip_extraction.py').is_file() and 'def exclusive_output(' in (app/'gio_backend.py').read_text())
        check('Terminal action and validated native launcher included',
              (app/'terminal_integration.py').is_file() and "elif method == 'openTerminal':" in (app/'winspace.py').read_text()
              and 'Open in Terminal' in (app/'ui/app.js').read_text())
        check('Native tab transport and acknowledged merge broker shipped',
              (app/'native_tab_drag.py').is_file() and (app/'tab_transfers.py').is_file()
              and all(('elif method == '+repr(action)+':') in (app/'winspace.py').read_text()
                      for action in ('beginTabDrag','moveTabToWindow','tabTransferReady','tabDragLayout')))
        check('Native tab payload restricted to this application',
              'Gtk.TargetFlags.SAME_APP' in (app/'native_tab_drag.py').read_text()
              and 'secrets.token_hex(32)' in (app/'tab_transfers.py').read_text())
        file_drag = (app/'native_file_drag.py').read_text()
        check('Native file drag exports GTK file targets and is copy-only',
              'self.targets.add_uri_targets(URI_INFO)' in file_drag
              and 'data.set_uris(list(self.files.exported))' in file_drag
              and 'self.Gdk.DragAction.COPY' in file_drag
              and 'self.Gdk.DragAction.MOVE' not in file_drag
              and "view.stop_emission_by_name('drag-data-delete')" in file_drag)
        check('Native file drag and receiving bridge shipped',
              (app/'native_file_drop.py').is_file()
              and all(('elif method == ' + repr(action) + ':') in (app/'winspace.py').read_text()
                      for action in ('beginFileDrag', 'fileDragLayout')))
        drag_source = (app/'native_tab_drag.py').read_text()
        check('Desktop tear-out target replies without exposing the merge token',
              "ROOT_MIME = 'application/x-rootwindow-drop'" in drag_source
              and "data.set(self.root_atom, 8, b'')" in drag_source)
        check('Native drag distinguishes cancellation and explicit body tear-out',
              "return 'detach', None" in drag_source
              and 'self.Gdk.DragAction(0)' in drag_source
              and "self.c.emit('tabDetachRequested'" in drag_source)
        ui_source = (app/'ui/app.js').read_text()
        check('Middle-click uses WebKit-compatible release and background tabs',
              'function bindMiddleClick(' in ui_source and "addEventListener('mouseup'" in ui_source
              and "{background:!e.shiftKey}" in ui_source)
        check('Private state-file validation helper included',
              (app/'private_storage.py').is_file() and 'O_NOFOLLOW' in (app/'private_storage.py').read_text())
        native_source=(app/'winspace.py').read_text()
        check('WebKit subprocess sandbox explicitly enabled before WebView',
              native_source.index('context.set_sandbox_enabled(True)') < native_source.index('WebKit2.WebView(web_context='))
        local_modules = {p.stem for p in app.glob('*.py')}
        missing = set()
        for source in app.glob('*.py'):
            tree = ast.parse(source.read_text())
            for node in ast.walk(tree):
                if isinstance(node, ast.ImportFrom) and node.module:
                    name = node.module.split('.')[0]
                    if name not in sys.stdlib_module_names and name not in local_modules and name not in {'gi'}:
                        missing.add(name)
        check('Application local import dependencies are complete', not missing)
        for name in ('openxplorer', 'openxplorer-mount-share', 'winspace', 'winspace-mount-share'):
            subprocess.run(['sh', '-n', str(stage / 'usr/bin' / name)], check=True)
        for name in ('postinst', 'postrm'):
            script = stage / 'DEBIAN' / name
            subprocess.run(['sh', '-n', str(script)], check=True)
            text = script.read_text()
            check(name + ' contains no automatic default/mount/user-data operations', not any(x in text for x in ('xdg-mime default', 'mount.cifs', 'rm -rf', 'settings.json', '/home/')))
        check('Launchers and maintainer scripts pass shell syntax checks', True)
        for line in (stage / 'DEBIAN/md5sums').read_text().splitlines():
            digest, relative = line.split('  ', 1)
            actual = hashlib.md5((stage / relative).read_bytes(), usedforsecurity=False).hexdigest()
            if digest != actual:
                raise ValueError('Package checksum mismatch: ' + relative)
        check('Every listed payload checksum matches', True)
        desktop = (stage / 'usr/share/applications/io.winspace.Development.desktop').read_text()
        check('Desktop launcher and icon identifiers match', 'Exec=openxplorer %U' in desktop and 'Icon=io.winspace.Development' in desktop and 'StartupWMClass=io.winspace.Development' in desktop)
        check('ZIP MIME support declared with independent opt-in actions',
              all(v in desktop for v in ('application/zip;', 'application/x-zip;', 'application/x-zip-compressed;'))
              and "elif method in ('zipDefault', 'zipRestore'):" in native_source)
        meta = ET.parse(stage/'usr/share/metainfo/io.winspace.Development.metainfo.xml').getroot()
        check('AppStream desktop ID and stock folder icon match', meta.findtext('id')=='io.winspace.Development' and meta.findtext('launchable')=='io.winspace.Development.desktop' and meta.findtext('icon')=='io.winspace.Development')
        check('AppStream declares AGPL-3.0-only project license and metadata license',meta.findtext('project_license')=='AGPL-3.0-only' and meta.findtext('metadata_license')=='CC0-1.0')
        check('AppStream release matches package version',meta.find('releases/release').attrib['version']==VERSION)
        check('File Explorer launcher search metadata', 'GenericName=File Explorer' in desktop and 'Keywords=File Explorer;' in desktop)
        catalog=ET.parse(stage/'usr/share/swcatalog/xml/openxplorer.xml').getroot()
        item=catalog.find('component')
        check('AppStream catalog maps exact package name to the desktop application', item.findtext('pkgname')=='openxplorer' and item.findtext('id')==meta.findtext('id'))
        check('Catalog publishes project license and official homepage', item.findtext('project_license')=='AGPL-3.0-only' and item.findtext("url[@type='homepage']")=='https://openxplorer.app')
        icon_path=item.findtext("icon[@type='local']")
        check('Catalog folder icon actually installed',bool(icon_path) and (stage/icon_path.lstrip('/')).is_file())
        check('Machine-readable Debian copyright and full AGPL text included','License: AGPL-3.0-only' in (stage/'usr/share/doc/openxplorer/copyright').read_text() and (stage/'usr/share/doc/openxplorer/AGPL-3.0.txt').is_file())
        check('Runtime version guard and safe restart included',(app/'runtime_guard.py').is_file() and 'require_current(session, RUNTIME' in native_source and '--restart' in native_source)
        check('No fallback menu model registered','self.set_app_menu(None)' in native_source and 'self.set_app_menu(menu)' not in native_source)
        check('UI and native request versions are compared',"request.get('release')" in native_source and 'state.env.version!==UI_RELEASE' in (app/'ui/app.js').read_text())
        check('Desktop actions expose Settings and existing windows',all(v in desktop for v in ('Exec=openxplorer --windows','Exec=openxplorer --settings','Actions=NewWindow;Windows;Settings;')))
        check('No system-wide FileManager1 override installed automatically',not (stage/'usr/share/dbus-1/services/org.freedesktop.FileManager1.service').exists())
        if shutil.which('desktop-file-validate'):
            subprocess.run(['desktop-file-validate', str(stage / 'usr/share/applications/io.winspace.Development.desktop')], check=True)
            check('desktop-file-validate', True)
        if shutil.which('node'):
            for script in ('app.js', 'bootstrap.js', 'type-select.js', 'snapshot-meta.js', 'text-size.js'):
                subprocess.run(['node', '--check', str(app / 'ui' / script)], check=True)
            check('Packaged JavaScript passes Node syntax checks', True)
    return {'success': True, 'package': package.name, 'version': VERSION,
            'bytes': package.stat().st_size, 'sha256': hashlib.sha256(package.read_bytes()).hexdigest(),
            'checks': checks, 'count': len(checks),
            'nativeRuntimeTested': False,
            'scope': 'Package metadata, structure, permissions, dependency completeness, checksums and source syntax; no install or native launch.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    parser.add_argument('--json', type=Path)
    args = parser.parse_args()
    try:
        result = verify(args.package.resolve())
    except (OSError, ValueError, SyntaxError, subprocess.CalledProcessError) as exc:
        parser.exit(1, 'Package verification failed: ' + str(exc) + '\n')
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
