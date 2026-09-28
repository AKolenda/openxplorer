# SPDX-License-Identifier: AGPL-3.0-only
"""Inputs that generate_python.py runs through desktop/core.py.

Synthetic addresses only: nothing here names or reads a real file.
"""

# The home folder for every table except EXTERNAL_LOCATIONS.
HOME = '/home/test'

LOCATIONS = [
    # test_core.py cases
    r'\\NAS\Team files\Q3 #1', '//NAS/Projects', 'smb://NAS/Team%20files/100%25.pdf',
    '/tmp/Été #1?.txt',
    'mtp://[usb:001,010]/Internal storage/DCIM', 'gphoto2://[usb:001,002]/DCIM',
    'afc://00008020-001C/',
    'mtp://user@device/DCIM', 'mtp://[usb:001,002/DCIM', 'mtp://[usb:001,002]/DCIM?mode=write',
    'afc:///DCIM',
    'mtp://[usb:001,002]/a%00b', '~/Docs', 'file://localhost/tmp/x', 'smb://u:p@nas/share',
    'smb://u@nas/share',
    r'\\u:p@nas\share', 'smb://u%40nas/share', '', 'http://example.org', 'javascript:alert(1)',
    'file://nas/share',
    'smb:///share', 'smb://nas/a%00b', 'smb://nas/a#b', 'smb://nas/a?b', r'C:\Windows',
    'smb://nas%0a/share',
    # whitespace and controls
    '   ', '\t', ' \u3000/tmp/x\u00a0 ', '\x1c/tmp\x1f', '/tmp/a\x00', '/tmp/a\x7f',
    '/tmp/a\u0085b', 'smb://nas/a\tb',
    # UNC forms
    r'\\nas', '\\\\nas\\', '\\\\', '//', '///tmp/x', '//tmp', r'\\nas:445\share', r'\\nas\a\..\b',
    r'\\nas\a/b',
    r'\\nas\a%20b', r'\\NAS\é', r'\\nas\100%', r'\\[::1]\x', r'\\nas?x\share', r'\\nas#x\share',
    r'\\nas%41\share',
    r'\\[fe80--1]\x', r'\\nas\..\..\x', r'\\nas\.', '\\\\nas\\a\\\\b\\', r"\\nas\it's (1)!*",
    # drive letters and schemes
    'c:/x', 'C:', 'C:x', 'z:\\', '1:/x', 'a:b', 'report:2024.txt', 'ftp://h/x', 'sftp://h/x',
    'dav://h/', 'trash:///',
    'recent:///', 'network:///', 'home:', 'ox:home', 'x-y.z+w:foo',
    # home and relative
    '~', '~/', '~user', '~/../x', '~/./a//b/', '~//etc', '~///etc', 'relative', 'a/../..', '../..',
    '.', '..',
    'a b/c#d?e%20f',
    # absolute local
    '/', '/..', '/tmp/x/', '/tmp/./x', '/tmp/../../..', '/tmp/a b', '/tmp/a%20b',
    "/tmp/it's (1) [x] {y} @ $ & + , ; = ~ ! *", '/tmp/€/ü/日本', '/tmp/a\\b', '/tmp/a:b', '/tmp/😀',
    # file URLs
    'file:///tmp/x', 'FILE:///TMP', 'File://LOCALHOST/x', 'file:tmp', 'file:/tmp/x', 'file:',
    'file://', 'file:///',
    'file:///tmp/a%2Fb', 'file:///tmp/%23', 'file:///tmp/a%3Fb', 'file:///tmp/%FF',
    'file:///tmp/%zz', 'file:///tmp/%4',
    'file:///tmp/a%00b', 'file:///tmp/../..', 'file:///tmp/x?y', 'file:///tmp/x#y',
    'file:///tmp/x?', 'file:///tmp/x#',
    'file://user@localhost/x', 'file:////server/share', 'file:///tmp/%e2%82%ac',
    'file:///tmp/a%5Cb', 'file:///tmp/a b',
    'file://localhost:80/x', 'file://[::1]/x', 'file:///tmp/%C3', 'file:///a/./b/../c/',
    # SMB URLs
    'smb://nas', 'smb://nas/', 'smb://NAS/Share/', 'smb://nas:445/a', 'smb://nas:0445',
    'smb://nas:/a', 'smb://nas:0/a',
    'smb://nas:65535/a', 'smb://nas:65536/a', 'smb://nas:99999/a', 'smb://nas:x/a',
    'smb://nas:-1/a', 'smb://nas:+1/a',
    'smb://nas:١/a', 'smb://[fe80::1]/a', 'smb://[FE80::1]:445/a', 'smb://[fe80::1%eth0]/a',
    'smb://[::1]:445/a',
    'smb://[1.2.3.4]/', 'smb://[v1.x]/share', 'smb://[vz.x]/share', 'smb://[v1.]/share',
    'smb://[nas]/a', 'smb://x[::1]/',
    'smb://[::1]x/', 'smb://[::1', 'smb://::1]/', 'smb://nas/a%5Cb', 'smb://nas/a\\b',
    'smb://nas/a?', 'smb://nas/a#',
    'smb:nas/share', 'smb:/nas/share', 'smb://my nas/a', 'smb://nas\u00a0x/a', 'smb://nas/../..',
    'smb://nas/a/./b/',
    'smb://nas/%E2%82%AC', 'smb://nas/%e2%82%ac', 'smb://nas/a%2Fb', 'smb://nas/a%25b',
    'smb://nas/~user', 'smb://nas/a b',
    'smb://NÄS/x', 'smb://ＮＡＳ/x', 'smb://nas℀/x', 'smb://İ/x', 'smb://nas/%FF', 'smb://nas/%zz',
    'SMB://Nas.Local:139/Share/Sub/', 'smb://10.0.0.1/share', "smb://nas/it's (1)!",
    'smb://nas//a//b', 'smb://nas/a/..',
    'smb://@nas/x', 'smb://:p@nas/x', 'smb://nas:445:1/x', 'smb://[::1]:/x', 'smb://[::1]:x/x',
    'smb:// nas/x',
    'smb://nas /x', 'smb://n%20as/x', 'smb://ǅ/x', 'smb://ß/x',
    # device URIs
    'MTP://[usb:001,010]', 'mtp://[usb:001,010]', 'afc://Device-ID/a/../b/', 'mtp://[usb]x/',
    'mtp://[[usb]]/',
    'mtp://a b/', 'mtp://a%20b/', 'mtp:///x', 'mtp://' + 'x' * 512 + '/',
    'mtp://' + 'x' * 513 + '/',
    'mtp://' + 'é' * 512 + '/', 'mtp://' + 'é' * 513 + '/', 'mtp://x/a\\b',
    'mtp://x/a%2Fb', 'mtp://x/%FF', 'mtp://x/%E2%82%AC', 'mtp://x/a#b', 'mtp://x/a?b', 'mtp:x',
    'mtp:', 'mtp://',
    'gphoto2://[usb:001,002]/', 'GPHOTO2://[usb:001,002]/store_00010001/DCIM/', 'afc://abc:1/x',
    'afc://abc\u3000/x',
    'mtp://x]/', 'mtp://[x/', 'mtp://x/..', 'mtp://x/./a/',
    'mtp://[usb:001,010]/Internal%20storage', 'mtp://x/a%zz',
    'mtp://x/a%00', 'mtp://x\u001c/', 'mtp://[usb:1]]/',
]

BASES = [
    ('Plans', 'file:///home/a'), ('x', 'file:///tmp/a%20b/'), ('..', 'file:///home/a'),
    ('Next plan', 'smb://nas/share'),
    ('../../x', 'smb://nas/share/'), ('a#b', 'smb://nas/share'), ('a?b', 'smb://nas/share'),
    ('a%20b', 'smb://nas/share'),
    ('Docs', 'trash:///'), ('Docs', 'ox:pc'), ('Docs', 'home:'),
    ('DCIM/Camera', 'mtp://[usb:001,010]/Internal%20storage'),
    ('../..', 'mtp://[usb:001,010]/Internal%20storage'), ('x', 'MTP://[usb:001,010]/'),
    ('x', 'SMB://NAS/a'),
    ('x', 'FILE:///tmp'), ('x', ''), ('x', 'file:///tmp/%FF'), ('x', 'file://[bad/x'),
    ('x', 'smb://[bad/'),
    ('x', 'smb://u@nas/a'), ('/abs', 'smb://nas/share'), ('~', 'smb://nas/share'),
    ('~/x', 'smb://nas/share'),
    ('é #', 'smb://nas/s'), ('a\\b', 'smb://nas/s'), ('x', 'file:///tmp/a%23b'),
    ('x', 'file://localhost/tmp'),
    ('x', 'afc://id'), ('x', 'smb://nas'), ('x', 'smb://nas/share?q'), ('.', 'smb://nas/share'),
    ('x', 'file://h/tmp'),
    ('x', 'file:////srv/a'), ('x', 'smb://nas/%FF'), ('C:\\x', 'smb://nas/s'), ('x', 'mtp:x'),
]

NAMES = ['Résumé 2026.txt', '', '.', '..', '../bad', 'a/b', 'a\\b', 'x\x00', 'x\n', 'del\x7f',
         'é' * 200, 'a' * 255,
         'a' * 256, 'x\u0085', ' ', '...', '.hidden', 'é' * 127 + 'a', 'é' * 128]

COPY_NAMES = [
    ('file.pdf', 2, False), ('.env', 2, False), ('Folder.v1', 3, True),
    ('archive.tar.gz', 2, False),
    ('..hidden.txt', 4, False), ('trailing.', 2, False), ('README', 10, False),
    ('é' * 120 + '.txt', 2, False),
    ('é' * 125 + '.txt', 2, False), ('a.' + 'x' * 250, 2, False), ('a/b', 2, False),
    ('...', 2, False),
    ('.a.b', 2, False), ('x' * 255, 2, True), ('x' * 250 + '.txt', 99, False),
    ('日本語' * 28, 12345, False),
    ('a..b', 2, False), ('.', 2, False), ('noext', 0, False), ('😀' * 63 + '.md', 7, False),
]

LABELS = [('  Projects (Z:) ', 'Folder'), (' \t', 'Folder'), ('é' * 120, 'Folder'),
          ('é' * 121, 'Folder'),
          ('a\x01b', 'Folder'), ('', 'Fallback'), ('\x1c', 'F'), ('\u3000x\u00a0', 'F'),
          ('a\u0085', 'F'),
          ('\x1cx\x1d', 'F')]

ITEMS = ['smb://nas/work/Projects', 'smb://nas/', 'smb://nas/work', 'smb://nas/work/',
         'mtp://[usb:001,010]/',
         'mtp://[usb:001,010]/Internal%20storage', 'file:///', '/tmp', 'afc://x',
         'gphoto2://[usb:1]/DCIM',
         r'\\nas\s\x', 'http://x/', 'smb://nas//', 'trash:///', 'smb://nas/work/../x',
         'mtp://x/./']

SHARES = ['smb://nas/', '/tmp', r'\\nas\share', 'smb://nas/share/sub', 'mtp://x/a', 'smb://nas',
          'bad:']

SERVERS = ['smb://nas/', '\\\\nas', 'smb://nas/work', 'file:///', 'http://nas/', 'smb://nas',
           'smb://nas/a/..',
           'mtp://x/', 'bad:', '', 'smb://u@nas/']

SPLITS = ['mtp://[usb:001,010]/Internal%20storage/DCIM', 'GPhoto2://[usb:001,002]',
          'smb://[usb:001,002]/',
          'smb://NAS/Team%20files', 'file:///tmp/x', '/tmp/a:b', 'C:\\Windows', 'smb://nas/a?b#c',
          'mtp:foo',
          'smb://[fe80::1]/share', 'smb://[fe80::1%eth0]:445/share', 'smb://[v1.x]/share',
          'smb://[nas/share',
          'smb://nas]/', 'smb://[1.2.3.4]/', 'smb://x[::1]/', 'smb://[::1]x/', ' mtp://x/',
          '\tsmb://a\tb/c\n',
          'smb://nas℀/x', 'trash:///a', 'ox:home', 'mtp://x/a?b', 'mtp://[a]b/', '', '//x/y',
          'smb://u[@[::1]/',
          'smb://[::1]@x/', 'smb://[::1]:5/', 'smb://a:b@[::1]:5/p', 'x://[::1]',
          'smb://[fe80::1%]/', 'smb://[v1.x\n]/',
          'smb://[::ffff:1.2.3.4]/', 'smb://[01.2.3.4]/', 'smb://[1.2.3.04]/', 'afc://x#y/z',
          '1abc://x', 'a+b-c.d://h/p',
          '\x00\x1f smb://h/p', 'smb://h/p\r\n', 'é://x', 'smb://[fe80::1%a%b]/',
          'smb://[::1%25eth0]/', 'smb://[v1.x]:8/',
          'smb://[V1.x]/', 'smb://[vA.x]/', 'smb://[v.x]/', 'smb://[:::]/', 'smb://[1::2::3]/',
          'smb://ｘ℀/',
          'smb://a℀@b/', 'mtp://ａ℀/x']

# The home folder for EXTERNAL_LOCATIONS.
EXTERNAL_HOME = '/home/demo'

# Differential cases for location_external.rs, first captured by hand with
# HOME=/home/demo. Each is normalised, and the result is then checked as an
# item for file operations (require_item_uri).
EXTERNAL_LOCATIONS = [
    # local paths, home and relative names
    '/tmp/file', '/tmp/a/../b/', '/tmp/(a) b', '/tmp/Q3 #1?.txt', '/', '~', '~/', '~/Docs',
    '~//etc', 'Plans',
    '../../etc', './x/./y',
    # UNC paths
    '\\\\NAS\\Team files\\Q3 #1', '//nas/share', '\\\\nas', '\\\\u@nas\\share',
    '\\\\nas:445\\share',
    # file URLs
    'file:///tmp/x', 'file://localhost/tmp/x', 'file://LOCALHOST/tmp', 'file:////tmp/x',
    'file:///tmp/(a)!',
    'file:///tmp/%C3%89t%C3%A9%20%231%3F.txt', 'file:///tmp/Read me.txt', 'file:///',
    'file:///tmp/%zz',
    'file:///a/b/%2F', 'file:', 'file:relative', 'file://nas/share', 'file:///tmp/a%0Ab',
    'file:///tmp/%FF',
    ' file:///tmp/x ', 'file:///tmp/x\x1f',
    # SMB URLs
    'smb://NAS/Team%20files/100%25.pdf', 'smb://ALPHA/', 'smb://nas/work/', 'smb://nas/a/./b/../c',
    'smb://nas:445/share', 'smb://nas:0445/s', 'smb://nas:/s', 'smb://[FE80::1]/share',
    'smb://[FE80::1]:445/s',
    'smb://[::ffff:1.2.3.4]/s', 'smb://nas', 'smb://NAS/a\\b', 'smb://nas/%2e%2e/x',
    'smb://nas/a%2Fb', 'smb://ÄB/x',
    'smb://nas/a?', 'smb://nas/a#', 'smb://nas/Team%20files/%C3%89t%C3%A9', 'smb://u:p@nas/share',
    'smb://u@nas/share', 'smb://u%40nas/share', 'smb:///share', 'smb://nas/a%00b', 'smb://nas/a#b',
    'smb://nas/a?b',
    'smb://nas%0a/share', 'smb://nas:99999/share', 'smb://nas:44x/share', 'smb://nas:+1/s',
    'smb://a:b:c/s',
    'smb://[not-v6]/share', 'smb://[1.2.3.4]/s', 'smb://x[fe80::1]/s', 'smb://[fe80::1]x/s',
    'smb://na s/x',
    'smb://a\uff0fb/x', 'smb:nas/share', 'smb://[fe80::1%25eth0]/s',
    # connected devices
    'mtp://[usb:001,010]/Internal storage/DCIM', 'gphoto2://[usb:001,002]/DCIM',
    'afc://00008020-001C/',
    'MTP://[usb:001,010]', 'mtp://[usb:001,010]//a/../b', 'afc://x', 'mtp://user@device/DCIM',
    'mtp://[usb:001,002/DCIM', 'mtp://[usb:001,002]/DCIM?mode=write', 'afc:///DCIM',
    'mtp://[usb:001,002]/a%00b',
    'mtp://a b/x', 'mtp://[a]b]/x', 'mtp:/x',
    # refused input
    '', '   ', 'http://example.org', 'javascript:alert(1)', 'C:\\Windows', 'c:/x', 'trash:///',
    'archive:///tmp/a.zip/file', 'a:b',
]
