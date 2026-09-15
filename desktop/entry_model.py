# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Pure presentation classification; never performs stat, mounts, or transfers.

GIO DIRECTORY is not the only navigable object. GVfs smb-browse emits MOUNTABLE
shares and SHORTCUT servers with standard::target-uri and inode/directory.
Keep navigability separate from mutability: a share is not a regular directory
that can be renamed or sent to Trash from the server browser.
"""
from __future__ import annotations
from core import normalise_location, split_location


def classify_entry(kind: str, uri: str, content_type: str | None = None,
                   target_uri: str | None = None, is_virtual: bool = False) -> dict:
    virtual_kind = kind in ('mountable', 'shortcut')
    target = None
    if virtual_kind and target_uri:
        try:
            target = normalise_location(target_uri)
        except (ValueError, TypeError, UnicodeError):
            # Never turn backend metadata into executable/external navigation,
            # or persist an address containing credentials.
            target = None
    folder_mime = content_type == 'inode/directory'
    source = split_location(uri)
    # Some SMB backends omit a content type/target. Limit the fallback to an
    # actual mountable SMB share, not all extensionless files or all shortcuts.
    smb_mount = (kind == 'mountable' and source.scheme == 'smb'
                 and bool(source.netloc) and bool(source.path.strip('/'))
                 and (not target_uri or target is not None))
    navigable = (kind == 'directory'
                 or (kind == 'unknown' and folder_mime)
                 or (virtual_kind and bool(target) and folder_mime)
                 or smb_mount)
    virtual = virtual_kind or bool(is_virtual)
    is_share = navigable and kind == 'mountable' and source.scheme == 'smb'
    description = ('Network share' if is_share else
                   'Network location' if navigable and virtual else
                   'File folder' if navigable else None)
    return {'kind': kind, 'isDir': bool(navigable), 'isVirtual': virtual,
            'canOperate': not virtual and kind in ('directory', 'file', 'symlink'),
            'targetUri': (target or uri) if navigable and virtual else None,
            'folderType': description}
