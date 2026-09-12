#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Reproduce and regress GTK's fallback app-menu row using real GTK 3.

Run: dbus-run-session -- xvfb-run -a python3 tests/native_window_menu.py
No PyGObject/WebKit needed: ctypes invokes the installed GTK shared library.
This is a window-policy harness, NOT a native OpenXplorer/WebKit launch.
It executes the actual host's early ApplicationWindow/menubar-policy statements
through a thin ctypes adapter, with a simple label as the content widget.
"""
import ast
import ctypes as C
import ctypes.util
import json
from pathlib import Path
import time
from types import SimpleNamespace

P = C.c_void_p
I = C.c_int
S = C.c_char_p
GTYPE = C.c_size_t

def library(name):
    path = ctypes.util.find_library(name)
    if not path:
        raise RuntimeError(f'Missing system library: {name}')
    return C.CDLL(path)

def bind(lib, name, result, *args):
    f = getattr(lib, name)
    f.restype, f.argtypes = result, list(args)
    return f

gtk, gio, gobj, glib = map(library, ('gtk-3', 'gio-2.0', 'gobject-2.0', 'glib-2.0'))
init = bind(gtk, 'gtk_init_check', I, P, P)
new_app = bind(gtk, 'gtk_application_new', P, S, I)
register = bind(gio, 'g_application_register', I, P, P, P)
set_app_menu = bind(gtk, 'gtk_application_set_app_menu', None, P, P)
get_app_menu = bind(gtk, 'gtk_application_get_app_menu', P, P)
new_menu = bind(gio, 'g_menu_new', P)
append = bind(gio, 'g_menu_append', None, P, S, S)
new_action = bind(gio, 'g_simple_action_new', P, S, P)
add_action = bind(gio, 'g_action_map_add_action', None, P, P)
lookup_action = bind(gio, 'g_action_map_lookup_action', P, P, S)
new_window = bind(gtk, 'gtk_application_window_new', P, P)
set_show = bind(gtk, 'gtk_application_window_set_show_menubar', None, P, I)
get_show = bind(gtk, 'gtk_application_window_get_show_menubar', I, P)
box = bind(gtk, 'gtk_box_new', P, I, I)
set_titlebar = bind(gtk, 'gtk_window_set_titlebar', None, P, P)
get_titlebar = bind(gtk, 'gtk_window_get_titlebar', P, P)
set_title = bind(gtk, 'gtk_window_set_title', None, P, S)
get_title = bind(gtk, 'gtk_window_get_title', S, P)
get_decorated = bind(gtk, 'gtk_window_get_decorated', I, P)
get_resizable = bind(gtk, 'gtk_window_get_resizable', I, P)
set_size = bind(gtk, 'gtk_window_set_default_size', None, P, I, I)
add = bind(gtk, 'gtk_container_add', None, P, P)
new_label = bind(gtk, 'gtk_label_new', P, S)
show_all = bind(gtk, 'gtk_widget_show_all', None, P)
destroy = bind(gtk, 'gtk_widget_destroy', None, P)
visible = bind(gtk, 'gtk_widget_get_visible', I, P)
height = bind(gtk, 'gtk_widget_get_allocated_height', I, P)
container_type = bind(gtk, 'gtk_container_get_type', GTYPE)
is_a = bind(gobj, 'g_type_check_instance_is_a', I, P, GTYPE)
type_name = bind(gobj, 'g_type_name_from_instance', S, P)
CALLBACK = C.CFUNCTYPE(None, P, P)
forall = bind(gtk, 'gtk_container_forall', None, P, CALLBACK, P)
settings = bind(gtk, 'gtk_settings_get_default', P)
# Varargs: type each value explicitly and terminate with a NULL name.
gobj.g_object_set.argtypes = [P, S]
gobj.g_object_set.restype = None
set_name = bind(glib, 'g_set_application_name', None, S)
iteration = bind(glib, 'g_main_context_iteration', I, P, I)
unref = bind(gobj, 'g_object_unref', None, P)

checks = []
def check(name, condition):
    if not condition:
        raise AssertionError(name)
    checks.append(name)

def pump():
    until = time.monotonic() + .08
    while time.monotonic() < until:
        while iteration(None, 0):
            pass
        time.sleep(.004)

def menus(window):
    result = []
    def visit(widget, _):
        kind = type_name(widget).decode()
        if kind == 'GtkMenuBar':
            result.append({'type': kind, 'visible': bool(visible(widget)), 'height': height(widget)})
        if is_a(widget, container_type()):
            forall(widget, callback, None)
    callback = CALLBACK(visit)
    visit(window, None)
    return result

class NativeWindow:
    def __init__(self, application):
        self.handle = new_window(application)
    def set_show_menubar(self, value):
        set_show(self.handle, int(value))

class Adapter:
    ApplicationWindow = NativeWindow

# Execute just the production constructor and policy call; do not rewrite or
# duplicate the fix in the harness. The baseline omits the policy statement.
host = Path(__file__).resolve().parents[1] / 'winspace.py'
tree = ast.parse(host.read_text())
cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == 'OpenXplorerWindow')
method = next(n for n in cls.body if isinstance(n, ast.FunctionDef) and n.name == 'activate_window')
setup = []
for n in method.body:
    if isinstance(n, ast.Assign) and ast.unparse(n.targets[0]) == 'self.window':
        setup.append(n)
    elif (isinstance(n, ast.Expr) and isinstance(n.value, ast.Call)
          and isinstance(n.value.func, ast.Attribute) and n.value.func.attr == 'set_show_menubar'):
        setup.append(n)
check('Harness found production constructor and menubar policy', len(setup) == 2)

def make(app, fixed):
    controller = SimpleNamespace(app=app)
    code = ast.fix_missing_locations(ast.Module(body=setup if fixed else setup[:1], type_ignores=[]))
    exec(compile(code, str(host), 'exec'), {'Gtk': Adapter, 'self': controller})
    w = controller.window.handle
    title = box(0, 0)
    set_titlebar(w, title)
    set_title(w, b'OpenXplorer')
    set_size(w, 600, 250)
    add(w, new_label(b'Native GTK window-policy test (not WebKit)'))
    show_all(w)
    pump()
    return w, title

if not init(None, None):
    raise SystemExit('A display is required. Run under xvfb-run or a test desktop.')
set_name(b'OpenXplorer')
gobj.g_object_set(settings(), b'gtk-shell-shows-app-menu', I(0), S(b'gtk-shell-shows-menubar'), I(0), P())
app = new_app(b'org.openxplorer.TestMenuPolicy', 32)  # G_APPLICATION_NON_UNIQUE
if not register(app, None, None):
    raise SystemExit('Could not register isolated GTK application.')
menu = new_menu()
for name, label in ((b'new-window', b'New window'), (b'windows', b'Open windows'),
                    (b'settings', b'Settings'), (b'quit', b'Quit OpenXplorer')):
    action = new_action(name, None)
    add_action(app, action)
    unref(action)
    append(menu, label, b'app.' + name)
set_app_menu(app, menu)
baseline, _ = make(app, False)
before = menus(baseline)
check('Baseline reproduces visible fallback GtkMenuBar', any(m['visible'] and m['height'] > 0 for m in before))
fixed, title = make(app, True)
after = menus(fixed)
check('Production fix disables show-menubar', not get_show(fixed))
check('Fixed window has no visible fallback GtkMenuBar', not any(m['visible'] for m in after))
show_all(fixed)
pump()
check('Repeated show_all does not bring the row back', not any(m['visible'] for m in menus(fixed)))
second, _ = make(app, True)
check('Second window also has no fallback menu', not get_show(second) and not any(m['visible'] for m in menus(second)))
for exported in (1, 0):
    gobj.g_object_set(settings(), b'gtk-shell-shows-app-menu', I(exported), S(b'gtk-shell-shows-menubar'), I(exported), P())
    pump()
    check(f'No menu after desktop-export setting changes to {exported}', not any(m['visible'] for m in menus(fixed)))
# Execute the actual startup statement that removes the fallback menu model.
app_class=next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name=='OpenXplorer')
startup=next(n for n in app_class.body if isinstance(n, ast.FunctionDef) and n.name=='startup')
remove=next(n for n in startup.body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='set_app_menu')
exec(compile(ast.fix_missing_locations(ast.Module(body=[remove],type_ignores=[])),str(host),'exec'), {'self':SimpleNamespace(set_app_menu=lambda value:set_app_menu(app,None if value is None else value))})
pump()
check('No fallback application-menu model remains registered', not get_app_menu(app))
third,_=make(app,True)
check('No menu model or visible row in third window',not get_app_menu(app) and not any(m['visible'] for m in menus(third)))
destroy(third)
check('All four application actions still registered', all(lookup_action(app, n) for n in (b'new-window', b'windows', b'settings', b'quit')))
check('Window title remains available to desktop', get_title(fixed) == b'OpenXplorer')
check('Custom client-side titlebar retained', get_titlebar(fixed) == title)
check('Decorations and resizable flag retained', bool(get_decorated(fixed)) and bool(get_resizable(fixed)))
for w in (baseline, fixed, second):
    destroy(w)
pump()
unref(menu)
unref(app)
print(json.dumps({'success': True, 'checks': checks, 'count': len(checks),
                  'baselineMenu': before, 'fixedMenu': after,
                  'scope': 'Real GTK 3 via ctypes on a virtual X11 display; production early-window statements. Not native OpenXplorer/WebKit, Wayland, Zorin or SMB testing.'}, indent=2))
