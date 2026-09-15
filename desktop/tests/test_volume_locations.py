# SPDX-License-Identifier: AGPL-3.0-only
"""Portable-device volume serialization with fake GIO objects; no USB access."""
from types import SimpleNamespace as NS
import unittest

from volume_locations import locations, volume_id


class Root:
    def __init__(self, uri): self.uri = uri
    def get_uri(self): return self.uri


class Mount:
    def __init__(self, name, uri, *, shadowed=False, unmount=True):
        self.name, self.root, self.shadowed, self.unmount = name, Root(uri), shadowed, unmount
    def get_name(self): return self.name
    def get_root(self): return self.root
    def is_shadowed(self): return self.shadowed
    def can_unmount(self): return self.unmount


class Volume:
    def __init__(self, name, *, mounted=None, can_mount=True, uuid=None,
                 device=None, activation=None):
        self.name, self.mounted, self.mountable = name, mounted, can_mount
        self.uuid, self.device = uuid, device
        self.activation = Root(activation) if activation else None
    def get_name(self): return self.name
    def get_mount(self): return self.mounted
    def can_mount(self): return self.mountable
    def get_uuid(self): return self.uuid
    def get_identifier(self, kind): return self.device if kind == 'unix-device' else None
    def get_activation_root(self): return self.activation


class VolumeLocationTests(unittest.TestCase):
    def test_mounted_mtp_phone_and_afc_device_are_visible(self):
        monitor = NS(get_mounts=lambda: [
            Mount('Pixel 9', 'mtp://[usb:001,010]/'),
            Mount('iPhone', 'afc://00008020-001C/'),
            Mount('Disk', 'file:///media/example/Disk'),
        ], get_volumes=lambda: [])
        rows = locations(monitor)
        self.assertEqual([row['label'] for row in rows], ['Pixel 9', 'iPhone', 'Disk'])
        self.assertEqual([row['kind'] for row in rows], ['device', 'device', 'drive'])
        self.assertTrue(all(row['mounted'] for row in rows))

    def test_unmounted_phone_is_click_to_connect_device(self):
        phone = Volume('Android Phone', activation='mtp://[usb:001,011]/')
        monitor = NS(get_mounts=lambda: [], get_volumes=lambda: [phone])
        self.assertEqual(locations(monitor), [{
            'id': 'mtp://[usb:001,011]/', 'label': 'Android Phone',
            'mounted': False, 'kind': 'device'}])
        self.assertEqual(volume_id(phone), 'mtp://[usb:001,011]/')

    def test_unsupported_and_shadowed_mounts_stay_hidden(self):
        monitor = NS(get_mounts=lambda: [
            Mount('Web', 'https://example.invalid/files'),
            Mount('Shadow', 'mtp://[usb:001,012]/', shadowed=True),
        ], get_volumes=lambda: [Volume('Unavailable', can_mount=False)])
        self.assertEqual(locations(monitor), [])


if __name__ == '__main__': unittest.main()
