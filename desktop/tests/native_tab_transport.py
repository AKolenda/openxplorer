#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Exercise the production NativeTabDrag via real GTK3/X11 events under Xvfb.
A ctypes adapter substitutes only for unavailable PyGObject. Two DrawingAreas
stand in for WebKit: this is NOT a WebKit, Wayland or Zorin integration test.
Run: xvfb-run -a python3 tests/native_tab_drag.py
"""
import ctypes as C
import ctypes.util
from enum import IntFlag, IntEnum
import json
from pathlib import Path
import sys
import time
from types import SimpleNamespace as NS
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from native_tab_drag import NativeTabDrag, MIME, ROOT_MIME, ROOT_INFO
from tab_transfers import TabTransfers
P=C.c_void_p;I=C.c_int;U=C.c_uint;S=C.c_char_p;D=C.c_double
K=C.CDLL(ctypes.util.find_library('gtk-3'));G=C.CDLL(ctypes.util.find_library('gdk-3'));O=C.CDLL(ctypes.util.find_library('gobject-2.0'));L=C.CDLL(ctypes.util.find_library('glib-2.0'));X=C.CDLL(ctypes.util.find_library('X11'));T=C.CDLL(ctypes.util.find_library('Xtst'))
def bind(lib,name,restype,*args):
    f=getattr(lib,name);f.restype=restype;f.argtypes=args;return f
init=bind(K,'gtk_init_check',I,P,P);new_win=bind(K,'gtk_window_new',P,I);new_area=bind(K,'gtk_drawing_area_new',P)
overlay_new=bind(K,'gtk_overlay_new',P);overlay_add=bind(K,'gtk_overlay_add_overlay',None,P,P);eventbox_new=bind(K,'gtk_event_box_new',P);visible_window=bind(K,'gtk_event_box_set_visible_window',None,P,I);halign=bind(K,'gtk_widget_set_halign',None,P,I);valign=bind(K,'gtk_widget_set_valign',None,P,I);margin=bind(K,'gtk_widget_set_margin_start',None,P,I);request=bind(K,'gtk_widget_set_size_request',None,P,I,I);translate=bind(K,'gtk_widget_translate_coordinates',I,P,P,I,I,C.POINTER(I),C.POINTER(I))
size=bind(K,'gtk_window_set_default_size',None,P,I,I);move=bind(K,'gtk_window_move',None,P,I,I);add=bind(K,'gtk_container_add',None,P,P);show=bind(K,'gtk_widget_show_all',None,P);destroy=bind(K,'gtk_widget_destroy',None,P)
iteration=bind(K,'gtk_main_iteration',I);pending=bind(K,'gtk_events_pending',I);allocated=bind(K,'gtk_widget_get_allocated_width',I,P)
connect=bind(O,'g_signal_connect_data',C.c_ulong,P,S,P,P,P,I);disconnect=bind(O,'g_signal_handler_disconnect',None,P,C.c_ulong);stop=bind(O,'g_signal_stop_emission_by_name',None,P,S)
copy_event=bind(G,'gdk_event_copy',P,P);free_event=bind(G,'gdk_event_free',None,P)
coords=bind(G,'gdk_event_get_coords',I,P,C.POINTER(D),C.POINTER(D));button=bind(G,'gdk_event_get_button',I,P,C.POINTER(U));state=bind(G,'gdk_event_get_state',I,P,C.POINTER(U))
atom_new=bind(G,'gdk_atom_intern',P,S,I);atom_name=bind(G,'gdk_atom_name',P,P);free=bind(L,'g_free',None,P)
class List(C.Structure):pass
List._fields_=[('data',P),('next',C.POINTER(List)),('prev',C.POINTER(List))]
class TargetEntry(C.Structure):_fields_=[('target',S),('flags',U),('info',U)]
target_new=bind(K,'gtk_target_list_new',P,C.POINTER(TargetEntry),U);target_add=bind(K,'gtk_target_list_add',None,P,P,U,U)
get_targets=bind(K,'gtk_drag_dest_get_target_list',P,P);set_targets=bind(K,'gtk_drag_dest_set_target_list',None,P,P);dest_set=bind(K,'gtk_drag_dest_set',None,P,I,P,I,I)
add_events=bind(K,'gtk_widget_add_events',None,P,I);threshold=bind(K,'gtk_drag_check_threshold',I,P,I,I,I,I)
begin=bind(K,'gtk_drag_begin_with_coordinates',P,P,P,I,I,P,I,I);icon=bind(K,'gtk_drag_set_icon_name',None,P,S,I,I)
ctx_targets=bind(G,'gdk_drag_context_list_targets',C.POINTER(List),P);source_widget=bind(K,'gtk_drag_get_source_widget',P,P)
status=bind(G,'gdk_drag_status',None,P,I,U);finish=bind(K,'gtk_drag_finish',None,P,I,I,U);get_data=bind(K,'gtk_drag_get_data',None,P,P,P,U)
selection_set=bind(K,'gtk_selection_data_set',None,P,P,I,S,I);selection_get=bind(K,'gtk_selection_data_get_data',P,P);selection_length=bind(K,'gtk_selection_data_get_length',I,P)
widget_window=bind(K,'gtk_widget_get_window',P,P);origin=bind(G,'gdk_window_get_origin',I,P,C.POINTER(I),C.POINTER(I))
xopen=bind(X,'XOpenDisplay',P,S);xflush=bind(X,'XFlush',I,P);motion=bind(T,'XTestFakeMotionEvent',I,P,I,I,I,C.c_ulong);mouse=bind(T,'XTestFakeButtonEvent',I,P,U,I,C.c_ulong);key=bind(T,'XTestFakeKeyEvent',I,P,U,I,C.c_ulong);keysym=bind(X,'XKeysymToKeycode',U,P,C.c_ulong)
widgets={};errors=[];callbacks=[];event_copies=[]
class Atom:
    def __init__(self,h):self.h=h
    @staticmethod
    def intern(name,only):return Atom(atom_new(name.encode(),only))
    def name(self):
        p=atom_name(self.h)
        try:return C.string_at(p).decode()
        finally:free(p)
class Context:
    def __init__(self,h):self.h=h
    def __eq__(self,other):return isinstance(other,Context) and self.h==other.h
    def list_targets(self):
        out=[];p=ctx_targets(self.h)
        while p:out.append(Atom(p.contents.data));p=p.contents.next
        return out
class Event:
    def __init__(self,h):
        self.h=h;x=D();y=D();b=U();s=U();coords(h,C.byref(x),C.byref(y));button(h,C.byref(b));state(h,C.byref(s));self.x=x.value;self.y=y.value;self.button=b.value;self.state=s.value
    def copy(self):
        h=copy_event(self.h);event_copies.append(h);return Event(h)
class Targets:
    def __init__(self,h):self.h=h
    @staticmethod
    def new(rows):
        entries=(TargetEntry*len(rows))(*[TargetEntry(t.encode(),f,i) for t,f,i in rows]);return Targets(target_new(entries,len(rows)))
    def add(self,a,f,i):target_add(self.h,a.h,f,i)
class Selection:
    def __init__(self,h):self.h=h
    def set(self,a,f,data):selection_set(self.h,a.h,f,data,len(data))
    def get_data(self):
        n=selection_length(self.h);return C.string_at(selection_get(self.h),n) if n>=0 else None
class Widget:
    def __init__(self,h):self.h=h;widgets[h]=self
    def connect(self,name,fn):
        sig={
            'button-press-event':(I,(P,P,P)), 'button-release-event':(I,(P,P,P)), 'motion-notify-event':(I,(P,P,P)),
            'drag-motion':(I,(P,P,I,I,U,P)), 'drag-drop':(I,(P,P,I,I,U,P)),
            'drag-leave':(None,(P,P,U,P)), 'drag-end':(None,(P,P,P)), 'drag-failed':(I,(P,P,I,P)),
            'drag-data-get':(None,(P,P,P,U,U,P)), 'drag-data-received':(None,(P,P,I,I,P,U,U,P))}[name]
        def callback(*a):
            try:
                if name.endswith('-event'):return int(bool(fn(self,Event(a[1]))))
                if name in ('drag-motion','drag-drop'):return int(bool(fn(self,Context(a[1]),a[2],a[3],a[4])))
                if name=='drag-leave':return fn(self,Context(a[1]),a[2])
                if name=='drag-end':return fn(self,Context(a[1]))
                if name=='drag-failed':return int(bool(fn(self,Context(a[1]),a[2])))
                if name=='drag-data-get':return fn(self,Context(a[1]),Selection(a[2]),a[3],a[4])
                if name=='drag-data-received':return fn(self,Context(a[1]),a[2],a[3],Selection(a[4]),a[5],a[6])
            except Exception as exc:
                errors.append(name+': '+repr(exc));return 0 if sig[0] is I else None
        cb=C.CFUNCTYPE(sig[0],*sig[1])(callback);callbacks.append(cb);return connect(self.h,name.encode(),cb,None,None,0)
    def disconnect(self,h):disconnect(self.h,h)
    def stop_emission_by_name(self,name):stop(self.h,name.encode())
    def drag_dest_get_target_list(self):
        h=get_targets(self.h);return Targets(h) if h else None
    def drag_dest_set(self,flags,rows,actions):
        dest_set(self.h,flags,None,0,actions)
        if rows:set_targets(self.h,Targets.new(rows).h)
    def drag_dest_set_target_list(self,targets):set_targets(self.h,targets.h)
    def translate_coordinates(self,other,x,y):
        a=I();b=I();ok=translate(self.h,other.h,x,y,C.byref(a),C.byref(b));return (a.value,b.value) if ok else None
    def add_events(self,mask):add_events(self.h,mask)
    def get_allocated_width(self):return allocated(self.h)
    def drag_check_threshold(self,*args):return threshold(self.h,*args)
    def drag_begin_with_coordinates(self,targets,actions,button,event,x,y):return Context(begin(self.h,targets.h,actions,button,event.h,x,y))
    def drag_get_data(self,context,atom,time):get_data(self.h,context.h,atom.h,time)
class Action(IntFlag):DEFAULT=1;COPY=2;MOVE=4
Gtk=NS(TargetList=Targets,TargetEntry=NS(new=lambda *v:v),TargetFlags=NS(SAME_APP=1),
       DragResult=NS(NO_TARGET=1),drag_get_source_widget=lambda c:widgets.get(source_widget(c.h)),
       drag_set_icon_name=lambda c,n,x,y:icon(c.h,n.encode(),x,y),drag_finish=lambda c,ok,delete,t:finish(c.h,ok,delete,t))
Gdk=NS(Atom=Atom,DragAction=Action,ModifierType=NS(BUTTON1_MASK=256),EventMask=NS(BUTTON_PRESS_MASK=256,BUTTON_RELEASE_MASK=512,POINTER_MOTION_MASK=4),drag_status=lambda c,a,t:status(c.h,a,t))

def pump(t=.15):
    end=time.monotonic()+t
    while time.monotonic()<end:
        while pending():iteration()
        time.sleep(.005)

assert init(None,None),'Run this under xvfb-run.'
display=xopen(None);assert display
checks=[];events=[]
def check(label,value):
    assert value,(label,errors,events[-5:]);checks.append(label)
app=NS(controllers=[])
app.tab_transfers=TabTransfers(lambda ident,name,data:next(c for c in app.controllers if c.ident==ident).emit(name,data),lambda ident:ident in (1,2))
class Controller:
    def __init__(self,ident,left):
        self.ident=ident;self.app=app;self.closed=False;self.ui_ready=True;self.writes=0
        self.top=new_win(0);size(self.top,480,320);move(self.top,left,90)
        self.webview=Widget(new_area());overlay=overlay_new();add(overlay,self.webview.h);self.drag_area=Widget(eventbox_new());visible_window(self.drag_area.h,False);halign(self.drag_area.h,1);valign(self.drag_area.h,1);margin(self.drag_area.h,200);request(self.drag_area.h,245,45);overlay_add(overlay,self.drag_area.h);add(self.top,overlay);self.window=NS(get_id=lambda:ident,present=lambda:None)
        self.tab_drag=NativeTabDrag(self,Gtk,Gdk,NS());app.controllers.append(self);show(self.top);pump()
        self.tab_drag.update({'width':480,'height':45,'end':450,'tabs':[{'id':'t1','left':0,'right':180,'close':150}]})
    def emit(self,name,data):
        events.append((self.ident,name,data))
        if name=='tabDragRequest':self.tab_drag.begin(data['id'],{'uri':'file:///tmp/Sample','history':['file:///tmp/Sample'],'index':0})
        if name=='tabReceive':app.tab_transfers.ready(data['token'],self.ident,True)
    def point(self,x,y):
        a=I();b=I();origin(widget_window(self.webview.h),C.byref(a),C.byref(b));return a.value+x,b.value+y
one=Controller(1,30);two=Controller(2,650);pump()
def at(c,x,y):
    px,py=c.point(x,y);motion(display,-1,px,py,0);xflush(display);pump(.04)
def down():mouse(display,1,True,0);xflush(display);pump(.04)
def up():mouse(display,1,False,0);xflush(display);pump(.25)
def start(c):at(c,50,20);down();at(c,72,22);pump(.1)
start(one)
check('A real pointer gesture starts the production GTK drag',one.tab_drag.token is not None)
source_targets=[a.name() for a in one.tab_drag.context.list_targets()]
at(two,70,20);pump(.2);up()
check('Native payload crosses two GTK toplevel windows',any(w==2 and name=='tabReceive' for w,name,d in events))
check('Destination ACK retires the source exactly once',sum(w==1 and n=='tabTransferDone' and d['committed'] for w,n,d in events)==1)
check('No capability remains after acknowledgement',not app.tab_transfers.pending)
check('GTK source and destination callbacks completed without exceptions',not errors)
# A second transfer lands on the native blank-titlebar overlay, not HTML.
events.clear();start(one);at(two,300,20);up()
check('Blank titlebar overlay accepts native tabs beside the existing tabs',any(w==2 and name=='tabReceive' for w,name,d in events))
# Reorder within one window.
events.clear();start(one);at(one,280,20);up()
check('Same-window drop reorders instead of creating a window',any(n=='tabReorder' for w,n,d in events))
check('Same-window reorder does not retire its source',not any(n=='tabTransferDone' and d['committed'] for w,n,d in events))
# Escape cancels; no-target drop requests detach.
events.clear();start(one);esc=keysym(display,0xff1b);key(display,esc,True,0);key(display,esc,False,0);xflush(display);pump(.2);up()
check('Escape keeps the original tab',not any(n=='tabDetachRequested' or n=='tabTransferDone' and d['committed'] for w,n,d in events))
check('Escape discards its capability',not app.tab_transfers.pending)
events.clear();start(one);motion(display,-1,580,480,0);xflush(display);pump(.1);up()
check('Drop outside a tab strip requests a separate window',any(n=='tabDetachRequested' for w,n,d in events))
# Regression: body drops were rejected by rc.3 (only NO_TARGET detached).
events.clear();start(one);at(one,120,190);up()
check('Dropping below the source tab strip requests tear-out once',sum(n=='tabDetachRequested' for w,n,d in events)==1)
check('Explicit tear-out clears its capability without committing a merge',not app.tab_transfers.pending and not any(n=='tabTransferDone' and d['committed'] for w,n,d in events))
check('The user sees a new-window hint over the source body',any(n=='tabTearOutHint' and d['show'] for w,n,d in events))
events.clear();start(one);at(one,120,190);key(display,esc,True,0);key(display,esc,False,0);xflush(display);pump(.2);up()
check('Escape over the tear-out area still keeps the tab',not any(n=='tabDetachRequested' for w,n,d in events))
events.clear();start(one);at(two,120,180);up()
check('A drop into another window body is refused, not detached or merged',not any(n in ('tabDetachRequested','tabReceive') for w,n,d in events))
check('Refused destination leaves no transfer locks',not app.tab_transfers.pending)
# Root-drop protocol receiver: acts like the compositor's empty-data handshake.
# This is real GTK selection transport but NOT a Mutter/Wayland session.
root_top=new_win(0);size(root_top,350,220);move(root_top,500,530);root_area=Widget(new_area());add(root_top,root_area.h)
root_atom=Atom.intern(ROOT_MIME,False);root_area.drag_dest_set(0,[(ROOT_MIME,0,ROOT_INFO)],Action.MOVE)
root_payload=[]
def root_motion(widget,ctx,x,y,t):status(ctx.h,Action.MOVE,t);return True
def root_drop(widget,ctx,x,y,t):widget.drag_get_data(ctx,root_atom,t);return True
def root_received(widget,ctx,x,y,data,info,t):
    root_payload.append(data.get_data());finish(ctx.h,True,False,t)
root_area.connect('drag-motion',root_motion);root_area.connect('drag-drop',root_drop);root_area.connect('drag-data-received',root_received);show(root_top);pump()
events.clear();start(one)
check('Native source advertises the compositor tear-out target',ROOT_MIME in [a.name() for a in one.tab_drag.context.list_targets()])
a=I();b=I();origin(widget_window(root_area.h),C.byref(a),C.byref(b));motion(display,-1,a.value+80,b.value+70,0);xflush(display);pump(.2);up()
check('Root-drop data request completes with an empty payload',root_payload==[b''])
check('Root-drop completion requests exactly one new window',sum(n=='tabDetachRequested' for w,n,d in events)==1)
check('Root-drop clears native drag state and capability',one.tab_drag.context is None and not app.tab_transfers.pending)
destroy(root_top)
events.clear();start(two);at(one,70,20);up()
check('Merge back still works after a tear-out gesture',any(w==1 and n=='tabReceive' for w,n,d in events))
check('No file/URI payload was exposed to external applications',MIME in source_targets and 'text/uri-list' not in source_targets and 'text/plain' not in source_targets)
check('All production signal handlers remained exception free',not errors)
for c in app.controllers:c.tab_drag.close();destroy(c.top)
for event in event_copies:free_event(event)
out=Path(__file__).resolve().parents[1]/'test-results/native-tab-drag.json';out.parent.mkdir(exist_ok=True);out.write_text(json.dumps({'scope':'Production NativeTabDrag through ctypes GTK3 adapter, two DrawingAreas, real X11 pointer events; not WebKit/Wayland/Zorin','passed':len(checks),'checks':checks},indent=2))
print(json.dumps({'passed':len(checks),'checks':checks},indent=2))
