# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Properties, installed-app selection, templates, and exposed snapshots via GIO."""
from __future__ import annotations
import os
import stat
from pathlib import Path
from urllib.parse import unquote, urlsplit
import uuid
import gi
gi.require_version('Gio', '2.0')
from gi.repository import Gio
from native_opening import local_path
from app_catalog import unique_applications
from core import normalise_location, validate_name, is_smb_server
from gio_backend import inspect, raw, entry_from_info, ATTRIBUTES, error_payload

PROPERTY_ATTRS = ATTRIBUTES + ',time::created,time::access,time::changed,access::can-read,access::can-write,access::can-execute,owner::user,owner::group,unix::mode,standard::symlink-target,standard::allocated-size'
PRESETS = {
    'text': ('Text document', 'New document.txt', b''),
    'markdown': ('Markdown document', 'New document.md', b'# New document\n'),
    'csv': ('CSV file', 'New spreadsheet.csv', b''),
    'json': ('JSON file', 'New file.json', b'{}\n'),
    'html': ('HTML document', 'New page.html', b'<!doctype html>\n<html lang="en"><head><meta charset="utf-8"><title>New page</title></head><body></body></html>\n'),
    'empty': ('Empty file', 'New file', b''),
}


def properties(uri, cancel=None):
    file = Gio.File.new_for_uri(normalise_location(uri))
    info = file.query_info(PROPERTY_ATTRS, Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, raw(cancel))
    entry = entry_from_info(file, info)
    def uint(name): return info.get_attribute_uint64(name) if info.has_attribute(name) else None
    def flag(name): return info.get_attribute_boolean(name) if info.has_attribute(name) else None
    parent = file.get_parent()
    entry.update({'parentUri': parent.get_uri() if parent else None,
                  'contentType': info.get_content_type(), 'created': uint('time::created'),
                  'accessed': uint('time::access'), 'metadataChanged': uint('time::changed'),
                  'allocatedSize': uint('standard::allocated-size'),
                  'canRead': flag('access::can-read'), 'canWrite': flag('access::can-write'),
                  'canExecute': flag('access::can-execute'),
                  'owner': info.get_attribute_string('owner::user'),
                  'group': info.get_attribute_string('owner::group'),
                  'mode': oct(info.get_attribute_uint32('unix::mode') & 0o7777) if info.has_attribute('unix::mode') else None,
                  'linkTarget': info.get_symlink_target(),
                  'recursiveSizeCalculated': False})
    if not entry['isDir'] and entry['contentType']:
        app = Gio.AppInfo.get_default_for_type(entry['contentType'], False)
        entry['defaultApp'] = app.get_display_name() if app else None
    return entry


def list_applications(uri, cancel=None, all_apps=False):
    p = properties(uri, cancel)
    if p.get('symlink'):
        raise ValueError('Open the link target first to choose an application.')
    content_type = ('inode/directory' if p['isDir'] else p['contentType']) or 'application/octet-stream'
    default = Gio.AppInfo.get_default_for_type(content_type, False)
    default_id = default.get_id() if default else None
    recommended = Gio.AppInfo.get_all_for_type(content_type)
    all_ids = {a.get_id() for a in recommended}
    apps = Gio.AppInfo.get_all() if all_apps or p['isDir'] else recommended
    path = local_path(p['uri'])
    seen, rows = set(), []
    for app in unique_applications(apps, default_id):
        identifier = app.get_id()
        if not identifier or identifier in seen or identifier == 'io.winspace.Development.desktop' or not app.should_show(): continue
        if not app.supports_files() and not app.supports_uris(): continue
        seen.add(identifier)
        rows.append({'id': identifier, 'name': app.get_display_name(), 'default': identifier == default_id,
                     'recommended': identifier in all_ids, 'supportsUris': app.supports_uris(),
                     'available': bool(path or app.supports_uris())})
    rows.sort(key=lambda a: (not a['default'], not a['recommended'], a['name'].casefold()))
    return {'apps': rows, 'contentType': content_type, 'name': p['name'], 'uri': p['uri'], 'hasLocalPath': bool(path)}


def prepare_launch(uri, app_id, cancel):
    allowed = list_applications(uri, cancel, all_apps=True)
    if not any(a['id'] == app_id and a['available'] for a in allowed['apps']):
        raise ValueError('That installed application is unavailable for this file/location.')
    app = next((a for a in Gio.AppInfo.get_all() if a.get_id() == app_id), None)
    if app is None: raise ValueError('That application is no longer installed.')
    file = Gio.File.new_for_uri(normalise_location(uri))
    path = local_path(uri)
    return app, Gio.File.new_for_path(path) if path else file, allowed['contentType'], inspect(uri, cancel)


def list_templates(directory, cancel=None):
    rows = [{'id': key, 'name': item[0], 'suggestedName': item[1], 'builtin': True} for key, item in PRESETS.items()]
    folder = Gio.File.new_for_path(str(directory))
    if not folder.query_exists(raw(cancel)):
        return {'templates': rows, 'directory': str(directory), 'limit': 100}
    en = folder.enumerate_children(ATTRIBUTES, Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, raw(cancel))
    count = 0
    try:
        while count < 100:
            if cancel: cancel.check()
            info = en.next_file(raw(cancel))
            if info is None: break
            if info.get_file_type() != Gio.FileType.REGULAR or info.get_is_symlink() or info.get_is_hidden(): continue
            name = info.get_name()
            if name.endswith('.desktop') or info.get_size() > 16 * 1024**2: continue
            count += 1
            rows.append({'id': 'user:' + name, 'name': name, 'suggestedName': name, 'builtin': False})
    finally: en.close(None)
    return {'templates': rows, 'directory': str(directory), 'limit': 100}


def create_from_template(uri, name, template, directory, cancel):
    validate_name(name)
    uri = normalise_location(uri)
    if is_smb_server(uri): raise ValueError('Open a share before creating a file.')
    parent = Gio.File.new_for_uri(uri)
    target = parent.get_child(name)
    if target.query_exists(raw(cancel)): raise ValueError('An item with that name already exists. Nothing was overwritten.')
    if template in PRESETS:
        data = PRESETS[template][2]
    elif isinstance(template, str) and template.startswith('user:'):
        basename = validate_name(template[5:])
        if not any(t['id'] == template for t in list_templates(directory, cancel)['templates']):
            raise ValueError('This template is unavailable, too large, or not a regular template file.')
        source = Path(directory) / basename
        # User templates are local/XDG paths (including kernel/FUSE mounts).
        # O_NOFOLLOW plus fstat closes the check/open symlink race and refuses
        # FIFOs/devices. PRIVATE staging below never preserves execute bits.
        fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise ValueError('Templates must be regular files, not links or devices.')
            chunks, count = [], 0
            while True:
                cancel.check()
                block = os.read(fd, 65536)
                if not block: break
                count += len(block)
                if count > 16 * 1024**2: raise ValueError('Template exceeds 16 MiB.')
                chunks.append(block)
            data = b''.join(chunks)
        finally: os.close(fd)
    else: raise ValueError('Choose an available template or Empty file.')
    stage = parent.get_child('.winspace-new-' + uuid.uuid4().hex)
    created = False
    try:
        out = stage.create(Gio.FileCreateFlags.PRIVATE, raw(cancel)); created = True
        try:
            out.write_all(data, raw(cancel))
        finally: out.close(None)
        stage.move(target, Gio.FileCopyFlags.NO_FALLBACK_FOR_MOVE, raw(cancel), None, None)
        created = False
        return {'uri': target.get_uri()}
    finally:
        if created:
            try: stage.delete(None)
            except Exception: pass


class SnapshotProvider:
    def children(self, uri, cancel, limit=100):
        file = Gio.File.new_for_uri(normalise_location(uri))
        en = file.enumerate_children(ATTRIBUTES, Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, raw(cancel))
        rows = []
        try:
            for _ in range(limit + 1):
                cancel.check()
                info = en.next_file(raw(cancel))
                if info is None: break
                rows.append(entry_from_info(en.get_child(info), info))
        finally: en.close(None)
        return rows[:limit], len(rows) > limit

    def inspect(self, uri, cancel):
        try: return properties(uri, cancel)
        except Exception as exc:
            if error_payload(exc)['code'] == 'not-found':
                error = FileNotFoundError('Not present in this snapshot.'); error.code = 'not-found'; raise error from exc
            raise
