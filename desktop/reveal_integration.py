# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Opt-in per-user FileManager1 activation, with reversible file changes.
Does not kill Nautilus, replace portals, remove packages, or alter system files.
"""
import json
import os
from pathlib import Path
import tempfile

MARKER = '# Managed by Winspace: file-manager-integration v1\n'
SERVICE = MARKER + '[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec=/usr/bin/winspace --filemanager-service\n'
AUTOSTART = MARKER + '''[Desktop Entry]
Type=Application
Name=Winspace Show in Folder integration
Comment=Handle explicit file-reveal requests without opening a window at login
Exec=/usr/bin/winspace --filemanager-service
Icon=io.winspace.Development
NoDisplay=true
X-GNOME-Autostart-enabled=true
'''


def atomic_text(path, value):
    path.parent.mkdir(parents=True,exist_ok=True)
    fd,tmp=tempfile.mkstemp(dir=path.parent,prefix='.winspace-')
    try:
        with os.fdopen(fd,'w',encoding='utf8') as out:
            os.fchmod(out.fileno(),0o600);out.write(value);out.flush();os.fsync(out.fileno())
        os.replace(tmp,path)
    finally:
        if os.path.exists(tmp): os.unlink(tmp)


class RevealRegistration:
    def __init__(self, directory, config_home=None, data_home=None):
        home=Path.home()
        self.config=Path(config_home or os.environ.get('XDG_CONFIG_HOME',home/'.config'))
        self.data=Path(data_home or os.environ.get('XDG_DATA_HOME',home/'.local/share'))
        self.record=Path(directory)/'reveal-integration.json'
        self.files={self.data/'dbus-1/services/org.freedesktop.FileManager1.service':SERVICE,
                    self.config/'autostart/io.winspace.FileManager1.desktop':AUTOSTART}
    def enabled(self):
        try:return all(not p.is_symlink() and p.read_text()==text for p,text in self.files.items())
        except OSError:return False
    def enable(self):
        backups={}
        # Refuse to overwrite symlinks or an unrecorded third-party override.
        for p,text in self.files.items():
            if p.is_symlink(): raise ValueError('Refusing to replace a symlink: '+str(p))
            old=p.read_text() if p.exists() else None
            if old is not None and old!=text:
                raise ValueError('An existing user override needs review before enabling OpenXplorer: '+str(p))
            backups[str(p)]=old
        if not self.record.exists(): atomic_text(self.record,json.dumps({'previous':backups}))
        changed=[]
        try:
            for p,text in self.files.items():atomic_text(p,text);changed.append(p)
        except Exception:
            for p in changed:
                old=backups[str(p)]
                if old is None:p.unlink(missing_ok=True)
                else:atomic_text(p,old)
            raise
        return {'enabled':self.enabled()}
    def disable(self):
        kept=[]
        for p,text in self.files.items():
            if not p.exists() and not p.is_symlink():continue
            if p.is_symlink() or p.read_text()!=text:kept.append(str(p));continue
            p.unlink()
        self.record.unlink(missing_ok=True)
        return {'enabled':False,'preservedModifiedFiles':kept}
