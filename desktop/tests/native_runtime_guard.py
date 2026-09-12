#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Real isolated GApplication/D-Bus restart-contract test via system libgio.
Run: dbus-run-session -- python tests/native_runtime_guard.py
Uses a tiny stand-in GApplication, not full OpenXplorer or PyGObject/WebKit.
No real user session is touched; only child servers on the isolated bus.
"""
import ctypes as C
import ctypes.util
import json
from pathlib import Path
import subprocess
import sys
import time
from types import SimpleNamespace as NS
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from runtime_guard import Session, require_current, APP_ID
P=C.c_void_p;S=C.c_char_p;I=C.c_int
GIO=C.CDLL(ctypes.util.find_library('gio-2.0'));GL=C.CDLL(ctypes.util.find_library('glib-2.0'));OBJ=C.CDLL(ctypes.util.find_library('gobject-2.0'))
def bind(lib,name,result,*args):
    f=getattr(lib,name);f.restype=result;f.argtypes=list(args);return f
new_app=bind(GIO,'g_application_new',P,S,I)
register=bind(GIO,'g_application_register',I,P,P,P)
new_state=bind(GIO,'g_simple_action_new_stateful',P,S,P,P)
new_action=bind(GIO,'g_simple_action_new',P,S,P)
add_action=bind(GIO,'g_action_map_add_action',None,P,P)
connect=bind(OBJ,'g_signal_connect_data',C.c_ulong,P,S,P,P,P,I)
unref=bind(OBJ,'g_object_unref',None,P)
loop_new=bind(GL,'g_main_loop_new',P,P,I)
loop_run=bind(GL,'g_main_loop_run',None,P)
loop_quit=bind(GL,'g_main_loop_quit',None,P)
str_variant=bind(GL,'g_variant_new_string',P,S)
variant_parse=bind(GL,'g_variant_parse',P,P,S,P,P,P)
variant_type=bind(GL,'g_variant_type_new',P,S)
variant_type_free=bind(GL,'g_variant_type_free',None,P)
variant_unref=bind(GL,'g_variant_unref',None,P)
variant_type_string=bind(GL,'g_variant_get_type_string',S,P)
variant_string=bind(GL,'g_variant_get_string',S,P,P)
variant_bool=bind(GL,'g_variant_get_boolean',I,P)
variant_n=bind(GL,'g_variant_n_children',C.c_size_t,P)
variant_child=bind(GL,'g_variant_get_child_value',P,P,C.c_size_t)
variant_variant=bind(GL,'g_variant_get_variant',P,P)
bus_get=bind(GIO,'g_bus_get_sync',P,I,P,P)
bus_call=bind(GIO,'g_dbus_connection_call_sync',P,P,S,S,S,S,P,P,I,I,P,P)
error_remote=bind(GIO,'g_dbus_error_get_remote_error',P,P)
free=bind(GL,'g_free',None,P);error_free=bind(GL,'g_error_free',None,P)
class GError(C.Structure):_fields_=[('domain',C.c_uint),('code',I),('message',S)]
class Error(Exception):
    def __init__(self,msg,remote=None):super().__init__(msg);self.remote=remote
class Variant:
    def __init__(self,signature,values):self.signature=signature;self.values=values

def unpack(v):
    typ=variant_type_string(v).decode()
    if typ in ('s','o','g'):return variant_string(v,None).decode()
    if typ=='b':return bool(variant_bool(v))
    if typ=='v':
        child=variant_variant(v)
        try:return unpack(child)
        finally:variant_unref(child)
    result=[]
    for i in range(variant_n(v)):
        child=variant_child(v,i)
        try:result.append(unpack(child))
        finally:variant_unref(child)
    return tuple(result) if typ.startswith('(') else result
class Connection:
    def __init__(self):self.handle=bus_get(2,None,None)  # G_BUS_TYPE_SESSION = 2
    def call_sync(self,dest,path,interface,method,args,reply,flags,timeout,cancel):
        typ=variant_type(args.signature.encode());error=P()
        try:v=variant_parse(typ,repr(args.values).encode(),None,None,C.byref(error))
        finally:variant_type_free(typ)
        if not v:raise RuntimeError('Could not construct test GVariant')
        result=bus_call(self.handle,dest.encode(),path.encode(),interface.encode(),method.encode(),v,None,1,timeout,None,C.byref(error))
        if not result:
            err=C.cast(error,C.POINTER(GError)).contents;message=err.message.decode();remote=error_remote(error)
            name=C.cast(remote,S).value.decode() if remote else None
            if remote:free(remote)
            error_free(error);raise Error(message,name)
        try:answer=unpack(result)
        finally:variant_unref(result)
        return NS(unpack=lambda:answer)

IDENT={'version':'test-release','protocol':1,'build':'fixture-hash'}
if len(sys.argv)>1 and sys.argv[1]=='--server':
    mode=sys.argv[2];app=new_app(APP_ID.encode(),0);loop=loop_new(None,0);callbacks=[]
    if mode!='legacy':
        action=new_state(b'runtime-info',None,str_variant(json.dumps(IDENT).encode()));add_action(app,action);unref(action)
    action=new_action(b'quit',None)
    CB=C.CFUNCTYPE(None,P,P,P)
    def quit_callback(*_):
        if mode!='busy':loop_quit(loop)
    cb=CB(quit_callback);callbacks.append(cb);connect(action,b'activate',cb,None,None,0);add_action(app,action);unref(action)
    if not register(app,None,None):raise SystemExit(2)
    print('ready',flush=True);loop_run(loop);unref(app);raise SystemExit(0)

checks=[]
def check(name,value):
    if not value:raise AssertionError(name)
    checks.append(name)
session=Session(NS(DBusCallFlags=NS(NO_AUTO_START=1),DBusError=NS(get_remote_error=lambda e:e.remote)),NS(Variant=Variant,Error=Error),connection=Connection())
check('No pre-existing isolated application owner',session.owner() is None)
for mode in ('legacy','current','busy'):
    proc=subprocess.Popen([sys.executable,__file__,'--server',mode],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    try:
        check(mode+' child registered',proc.stdout.readline().strip()=='ready')
        status=session.status(IDENT);check(mode+' actual bus owner found',bool(status['owner']))
        if mode=='legacy':
            check('Legacy process detected without identity action',status['legacyProcess'])
            require_current(session,IDENT,restart=True)
            proc.wait(timeout=4);check('Legacy safe quit releases the bus name',session.owner() is None)
        elif mode=='current':
            check('Runtime identity read through real org.gtk.Actions',status['running']==IDENT)
            require_current(session,IDENT);check('Normal current-version activation leaves process running',proc.poll() is None)
            require_current(session,IDENT,restart=True);proc.wait(timeout=4)
            check('Explicit restart waits for exact owner to exit',session.owner() is None)
        else:
            try:session.stop(status['owner'],timeout=.15)
            except RuntimeError as exc:check('Busy server refuses restart without being killed','No process was killed' in str(exc) and proc.poll() is None)
            else:raise AssertionError('Busy quit unexpectedly succeeded')
    finally:
        if proc.poll() is None:proc.terminate();proc.wait(timeout=3) # ONLY the isolated test child we started
print(json.dumps({'success':True,'count':len(checks),'checks':checks,'scope':'Production runtime_guard.Session with ctypes libgio transport and isolated stand-in GApplication; not native OpenXplorer/PyGObject/WebKit'},indent=2))
