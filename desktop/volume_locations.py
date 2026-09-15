# SPDX-License-Identifier: AGPL-3.0-only
"""Serialize GIO volumes and mounts without starting or mounting anything.

Portable phones and cameras are normally exposed by GVfs as mtp://,
gphoto2:// or afc:// roots. The volume monitor already discovers them; this
module only turns existing monitor state into bounded UI data.
"""
from __future__ import annotations

from core import is_device_location, normalise_location


def volume_id(volume) -> str:
    """Return a stable-enough identifier for a single mount request."""
    activation = volume.get_activation_root()
    return (volume.get_uuid() or volume.get_identifier('unix-device')
            or (activation.get_uri() if activation else None) or volume.get_name())


def locations(monitor) -> list[dict]:
    """Read current GVolumeMonitor state; never mount, probe or enumerate it."""
    if monitor is None:
        return []
    rows = []
    for mount in monitor.get_mounts():
        try:
            if mount.is_shadowed():
                continue
            uri = normalise_location(mount.get_root().get_uri())
            rows.append({'label': mount.get_name(), 'uri': uri, 'mounted': True,
                         'kind': 'device' if is_device_location(uri) else 'drive',
                         'canUnmount': bool(mount.can_unmount())})
        except (AttributeError, TypeError, ValueError, UnicodeError):
            # Ignore roots outside the deliberately supported URI allowlist.
            continue
    for volume in monitor.get_volumes():
        try:
            if volume.get_mount() or not volume.can_mount():
                continue
            activation = volume.get_activation_root()
            activation_uri = normalise_location(activation.get_uri()) if activation else None
            rows.append({'id': volume_id(volume), 'label': volume.get_name(),
                         'mounted': False,
                         'kind': 'device' if activation_uri and is_device_location(activation_uri) else 'drive'})
        except (AttributeError, TypeError, ValueError, UnicodeError):
            continue
    return rows
