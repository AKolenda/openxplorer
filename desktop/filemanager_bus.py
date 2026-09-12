# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""The standard FileManager1 session-bus endpoint, registered only on request."""
from window_state import filemanager_request
from pathlib import Path
NAME='org.freedesktop.FileManager1'
PATH='/org/freedesktop/FileManager1'
XML='''<node><interface name="org.freedesktop.FileManager1">
<method name="ShowFolders"><arg type="as" direction="in"/><arg type="s" direction="in"/></method>
<method name="ShowItems"><arg type="as" direction="in"/><arg type="s" direction="in"/></method>
<method name="ShowItemProperties"><arg type="as" direction="in"/><arg type="s" direction="in"/></method>
</interface></node>'''

class FileManagerBus:
    def __init__(self,Gio,GLib,connection,handler,on_state=lambda:None):
        self.Gio,self.GLib,self.connection=Gio,GLib,connection
        self.handler,self.on_state=handler,on_state
        self.owner_id=0;self.registration=0;self.owned=False
    def enable(self):
        if self.owner_id:return
        self.node=self.Gio.DBusNodeInfo.new_for_xml(XML)
        self.registration=self.connection.register_object(PATH,self.node.interfaces[0],self.call,None,None)
        flags=self.Gio.BusNameOwnerFlags.REPLACE  # Do not silently yield after explicit user opt-in.
        self.owner_id=self.Gio.bus_own_name_on_connection(self.connection,NAME,flags,self.acquired,self.lost)
    def acquired(self,*_):self.owned=True;self.on_state()
    def lost(self,*_):self.owned=False;self.on_state()
    def disable(self):
        if self.owner_id:self.Gio.bus_unown_name(self.owner_id);self.owner_id=0
        if self.registration:self.connection.unregister_object(self.registration);self.registration=0
        self.owned=False;self.on_state()
    def status(self):
        # Read only the daemon's current owner; never auto-start or kill another app.
        result = {'ownerLabel': '', 'owner': None, 'ownedByOpenXplorer': False}
        try:
            def query(method, value):
                return self.connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                    'org.freedesktop.DBus', method, self.GLib.Variant('(s)', (value,)), None,
                    self.Gio.DBusCallFlags.NO_AUTO_START, 1000, None).unpack()[0]
            owner = query('GetNameOwner', NAME)
            result['owner'] = owner
            # Service ownership, not merely the configured MIME handler.
            result['ownedByOpenXplorer'] = owner == self.connection.get_unique_name() and self.owned
            pid = query('GetConnectionUnixProcessID', owner)
            if type(pid) is int and pid > 0:
                try: result['ownerLabel'] = Path(f'/proc/{pid}/comm').read_text().strip()[:80]
                except OSError: pass
            if self.owned: result['ownerLabel'] = 'OpenXplorer'
        except Exception:
            pass
        return result

    def call(self,_connection,_sender,_path,_interface,method,parameters,invocation):
        try:
            uris,startup_id=parameters.unpack()
            request=filemanager_request(method,uris)
            # URIs are validated data, never shell commands. No arbitrary eval or execution.
            self.handler(request, startup_id[:4096] if isinstance(startup_id,str) else '')
            invocation.return_value(self.GLib.Variant('()',()))
        except (TypeError,ValueError) as exc:
            invocation.return_dbus_error('org.freedesktop.DBus.Error.InvalidArgs',str(exc))
        except Exception:
            invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed','OpenXplorer could not open the requested location.')
