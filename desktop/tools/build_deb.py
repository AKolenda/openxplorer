#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Build (but never install) the OpenXplorer Debian package from this source tree.

Runtime libraries come from the distribution through APT. No compiler, pip
installation, online build step, or root privileges are needed for this build.
CairoSVG is an optional build-only requirement for raster icons; librsvg through
GdkPixbuf is used when it is missing. The SVG is also included.
"""
from __future__ import annotations

import argparse
import ast
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import copy
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from core import VERSION, DEBIAN_VERSION

NAME = f'openxplorer_{DEBIAN_VERSION}_all.deb'
REQUIRED = (
    'terminal_integration.py','private_storage.py','runtime_guard.py',
    'network_locations.py', 'volume_locations.py', 'window_state.py', 'tab_transfers.py', 'native_tab_drag.py', 'native_file_drag.py', 'native_file_drop.py', 'reveal_integration.py', 'filemanager_bus.py', 'brave_integration.py', 'app_catalog.py', 'folder_sizes.py', 'winspace.py', 'core.py', 'gio_backend.py', 'entry_model.py', 'operations.py',
    'desktop_integration.py', 'auth_bridge.py', 'session_credentials.py',
    'zip_extraction.py', 'activation.py', 'native_opening.py', 'file_clipboard.py', 'archives.py',
    'search_index.py', 'index_service.py', 'local_watch.py', 'file_services.py',
    'folder_locations.py', 'previous_versions.py', 'mount_support.py',
    'mount_share.py', 'ui/index.html', 'ui/app.js', 'ui/bootstrap.js',
    'ui/text-size.js', 'ui/snapshot-meta.js', 'ui/type-select.js', 'ui/style.css', 'ui/winspace.svg', 'openxplorer.py', 'LICENSE', 'README.md', 'UPGRADE.md',
)

DESKTOP = '''[Desktop Entry]
Type=Application
Name=OpenXplorer
GenericName=File Explorer
Keywords=File Explorer;file explorer;File Manager;files;folders;SMB;NAS;network;archives;
Comment=Explorer-style local and SMB file manager
Exec=openxplorer %U
TryExec=openxplorer
Icon=io.winspace.Development
Terminal=false
Categories=System;FileTools;FileManager;
MimeType=inode/directory;x-scheme-handler/smb;application/zip;application/x-zip;application/x-zip-compressed;
StartupNotify=true
StartupWMClass=io.winspace.Development
Actions=NewWindow;Windows;Settings;

[Desktop Action NewWindow]
Name=New window
Exec=openxplorer --new-window

[Desktop Action Windows]
Name=Open windows…
Exec=openxplorer --windows

[Desktop Action Settings]
Name=Settings
Exec=openxplorer --settings
'''

CACHE_SCRIPT = '''#!/bin/sh
set -e
# Refresh application/icon caches only. Never change defaults, user data,
# passwords, folder locations, or mounts during installation or removal.
case "$1" in
    configure|remove|purge)
        if command -v update-desktop-database >/dev/null 2>&1; then
            update-desktop-database -q /usr/share/applications || true
        fi
        if command -v gtk-update-icon-cache >/dev/null 2>&1; then
            gtk-update-icon-cache -q -f -t /usr/share/icons/hicolor || true
        fi
        if command -v appstreamcli >/dev/null 2>&1; then
            appstreamcli refresh-cache --force >/dev/null 2>&1 || true
        fi
        ;;
esac
exit 0
'''



def icon_renderer():
    """Return an offline SVG->PNG rasteriser. CairoSVG when present, otherwise
    librsvg through GdkPixbuf, which the GTK runtime already depends on."""
    try:
        import cairosvg
    except ImportError:
        pass
    else:
        return lambda src, dest, size: cairosvg.svg2png(
            url=str(src), write_to=str(dest), output_width=size, output_height=size)
    try:
        import gi
        gi.require_version('GdkPixbuf', '2.0')
        from gi.repository import GdkPixbuf
    except ImportError as exc:
        raise RuntimeError('Building installer icons requires CairoSVG or GdkPixbuf with SVG support (build dependency only).') from exc
    if not any(f.get_name() == 'svg' for f in GdkPixbuf.Pixbuf.get_formats()):
        raise RuntimeError('Building installer icons requires CairoSVG or a GdkPixbuf SVG loader (librsvg).')
    def render(src, dest, size):
        pixbuf = GdkPixbuf.Pixbuf.new_from_file_at_size(str(src), size, size)
        pixbuf.savev(str(dest), 'png', [], [])
    return render

def build(output: Path) -> Path:
    """Create a deterministic, root-owned package and return its absolute path."""
    if shutil.which('dpkg-deb') is None:
        raise RuntimeError('dpkg-deb is required. On Debian/Ubuntu: sudo apt install dpkg')
    missing = [name for name in REQUIRED if not (ROOT / name).is_file()]
    if missing:
        raise RuntimeError('Incomplete source tree: ' + ', '.join(missing))
    for path in ROOT.glob('*.py'):
        ast.parse(path.read_text(encoding='utf-8'), filename=str(path), feature_version=(3, 10))
    epoch = int(os.environ.get('SOURCE_DATE_EPOCH', '1788742800'))  # 2026-09-07 UTC; distinct from prior releases to invalidate timestamp-based bytecode
    if epoch < 0:
        raise ValueError('SOURCE_DATE_EPOCH must be nonnegative.')
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='winspace-package-') as temporary:
        stage = Path(temporary)
        app = stage / 'opt/openxplorer'
        app.mkdir(parents=True)
        for path in sorted(ROOT.glob('*.py')):
            shutil.copyfile(path, app / path.name)
        for name in ('README.md', 'MANUAL.md', 'UPGRADE.md', 'LICENSE', 'TEST-REPORT.md', 'SECURITY.md', 'CHANGELOG.md', 'ZORIN-SETUP.md', 'HOTFIX-0.9.2.md', 'HOTFIX-RC2.md'):
            if (ROOT / name).is_file():
                shutil.copyfile(ROOT / name, app / name)
        shutil.copytree(ROOT / 'ui', app / 'ui', ignore=shutil.ignore_patterns('__pycache__', '*.pyc', '*.map'))
        commands = stage / 'usr/bin'
        commands.mkdir(parents=True)
        (commands / 'openxplorer').write_text('#!/bin/sh\nexec /usr/bin/python3 /opt/openxplorer/openxplorer.py "$@"\n')
        (commands / 'openxplorer-mount-share').write_text('#!/bin/sh\nexec /usr/bin/python3 -I /opt/openxplorer/mount_share.py "$@"\n')
        (commands / 'winspace').write_text('#!/bin/sh\nexec /usr/bin/openxplorer "$@"\n')
        (commands / 'winspace-mount-share').write_text('#!/bin/sh\nexec /usr/bin/openxplorer-mount-share "$@"\n')
        shutil.copytree(ROOT / 'licenses', app / 'licenses')
        desktops = stage / 'usr/share/applications'
        desktops.mkdir(parents=True)
        (desktops / 'io.winspace.Development.desktop').write_text(DESKTOP)
        metainfo = stage / 'usr/share/metainfo'
        metainfo.mkdir(parents=True)
        shutil.copyfile(ROOT / 'packaging/io.winspace.Development.metainfo.xml', metainfo / 'io.winspace.Development.metainfo.xml')
        # Upstream metainfo is not a repository/package-name mapping. Supply a
        # one-application system catalog as well, so Software can refine the
        # otherwise generic PackageKit package into this desktop application.
        # This is installed metadata, NOT a remote repository or cached database.
        component = ET.parse(metainfo / 'io.winspace.Development.metainfo.xml').getroot()
        catalog = ET.Element('components', {'version':'0.15', 'origin':'openxplorer.app'})
        item = copy.deepcopy(component)
        ET.SubElement(item, 'pkgname').text = 'openxplorer'
        ET.SubElement(item, 'icon', {'type':'local', 'width':'128', 'height':'128'}).text = '/usr/share/icons/hicolor/128x128/apps/io.winspace.Development.png'
        catalog.append(item)
        catalog_dir = stage / 'usr/share/swcatalog/xml'
        catalog_dir.mkdir(parents=True)
        ET.indent(catalog, space='  ')
        ET.ElementTree(catalog).write(catalog_dir / 'openxplorer.xml', encoding='utf-8', xml_declaration=True)

        icons = stage / 'usr/share/icons/hicolor/scalable/apps'
        icons.mkdir(parents=True)
        shutil.copyfile(ROOT / 'ui/winspace.svg', icons / 'io.winspace.Development.svg')
        render = icon_renderer()
        for size in (32, 48, 64, 128, 256):
            dest = stage / f'usr/share/icons/hicolor/{size}x{size}/apps'
            dest.mkdir(parents=True)
            render(ROOT / 'ui/winspace.svg', dest / 'io.winspace.Development.png', size)
        doc = stage / 'usr/share/doc/openxplorer'
        doc.mkdir(parents=True)
        shutil.copyfile(ROOT / 'packaging/copyright', doc / 'copyright')
        shutil.copyfile(ROOT / 'LICENSE', doc / 'AGPL-3.0.txt')
        shutil.copytree(ROOT / 'licenses', doc / 'licenses')
        for name in ('README.md', 'UPGRADE.md', 'TEST-REPORT.md', 'SECURITY.md', 'CHANGELOG.md', 'ZORIN-SETUP.md', 'HOTFIX-0.9.2.md', 'HOTFIX-RC2.md'):
            if (ROOT / name).is_file():
                shutil.copyfile(ROOT / name, doc / name)
        installed = sum(p.stat().st_size for p in stage.rglob('*') if p.is_file()) // 1024 + 1
        control = stage / 'DEBIAN'
        control.mkdir()
        (control / 'control').write_text(f'''Package: openxplorer
Replaces: winspace-explorer (<< 0.8.0)
Conflicts: winspace-explorer (<< 0.8.0)
Provides: winspace-explorer
Version: {DEBIAN_VERSION}
Section: utils
Homepage: https://openxplorer.app
Priority: optional
Architecture: all
Maintainer: OpenXplorer contributors <maintainer@example.invalid>
Installed-Size: {installed}
Depends: python3 (>= 3.10), python3-gi, gir1.2-gtk-3.0, gir1.2-webkit2-4.1 (>= 2.40), gir1.2-secret-1, gvfs-backends, gvfs-fuse, xdg-utils, xdg-user-dirs, desktop-file-utils, hicolor-icon-theme
Recommends: cifs-utils, file-roller, gnome-terminal | x-terminal-emulator
Description: Explorer-style local and SMB file manager
 GTK/WebKitGTK interface with GIO/GVfs filesystem access, shared clipboard,
 session credentials, MIME-based opening, indexed search and local watches.
 Includes Properties, standard-folder locations, ZIP browsing and extraction.
 Free software licensed under AGPL-3.0-only; project: https://openxplorer.app.
 First stable release; developed for Zorin OS, with Ubuntu and Debian as
 secondary compatibility targets.
 No default associations or system mounts change during installation.
''')
        for name in ('postinst', 'postrm'):
            (control / name).write_text(CACHE_SCRIPT)
        checksums = []
        for path in sorted(stage.rglob('*')):
            if path.is_file() and control not in path.parents:
                digest = hashlib.md5(path.read_bytes(), usedforsecurity=False).hexdigest()
                checksums.append(f'{digest}  {path.relative_to(stage).as_posix()}')
        (control / 'md5sums').write_text('\n'.join(checksums) + '\n')
        executables = {*commands.iterdir(), control / 'postinst', control / 'postrm'}
        for path in stage.rglob('*'):
            path.chmod(0o755 if path.is_dir() or path in executables else 0o644)
            os.utime(path, (epoch, epoch))
        stage.chmod(0o755)
        os.utime(stage, (epoch, epoch))
        env = dict(os.environ, SOURCE_DATE_EPOCH=str(epoch))
        subprocess.run(['dpkg-deb', '--root-owner-group', '--uniform-compression',
                        '-Zxz', '-z6', '--build', str(stage), str(output)],
                       check=True, env=env)
    return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist' / NAME)
    args = parser.parse_args()
    try:
        print(build(args.output))
    except (RuntimeError, ValueError, OSError, SyntaxError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f'Build failed: {exc}\n')


if __name__ == '__main__':
    main()
