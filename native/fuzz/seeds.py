#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Writes the seed corpora of the fuzz targets into ``corpus/<target>/``.

Every seed is a small valid file made here, never a user's file. The
archive seeds start with the option byte that ``ox_core::fuzzing::archive``
reads: its low three bits pick the name the archive is opened under, bit 3
asks for the password ``lighthouse``.

Run from ``native/fuzz``: ``python3 seeds.py`` (into ``$OX_FUZZ_WORK/corpus``
when ``OX_FUZZ_WORK`` is set, as for ``run.sh``). The 7z and Zstandard seeds
need ``seed-maker`` (built on the first run with the stable toolchain).
"""

import bz2
import gzip
import hashlib
import io
import lzma
import os
import struct
import subprocess
import sys
import tarfile
import zipfile
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Where run.sh keeps the corpora: $OX_FUZZ_WORK, else here.
CORPUS = Path(os.environ.get('OX_FUZZ_WORK') or HERE) / 'corpus'
SEED_MAKER = HERE / 'seed-maker' / 'target' / 'release' / 'ox-fuzz-seed-maker'

# The option byte's name index (see NAMES in ox_core::fuzzing::archive).
ZIP, SEVEN, TAR, TGZ, TBZ, TXZ, TZST, OTHER = range(8)
PASSWORD = 0x08


def write(target, name, data):
    folder = CORPUS / target
    folder.mkdir(parents=True, exist_ok=True)
    (folder / name).write_bytes(data)


def archive_seed(name, option, data):
    for target in ('archive_list', 'archive_extract'):
        write(target, name, bytes([option]) + data)
    # archive_fixup takes plain TARs, compressing them itself, and 7z.
    kind = option & 7
    if kind == TAR:
        for compression in range(5):
            write('archive_fixup', f'{name}-{compression}', bytes([option & ~7, compression]) + data)
    elif kind == SEVEN:
        write('archive_fixup', name, bytes([option & ~7, 0]) + data)


def zip_with(method, members, **options):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w', compression=method, **options) as archive:
        for member, data in members:
            if member.endswith('/'):
                archive.writestr(zipfile.ZipInfo(member), b'')
            else:
                archive.writestr(member, data)
    return buffer.getvalue()


MEMBERS = [
    ('Docs/', b''),
    ('Docs/Guide.txt', b'hello world, ' * 20),
    ('Docs/Notes.md', b'# Notes\n\n- one\n- two\n'),
    ('Pictures/photo.png', b'\x89PNG\r\n\x1a\n' + b'\0' * 32),
    ('readme.txt', b'hello'),
]


# The traditional PKWARE cipher, to write one encrypted member.
def _crc_table():
    table = []
    for byte in range(256):
        value = byte
        for _ in range(8):
            value = (value >> 1) ^ 0xEDB88320 if value & 1 else value >> 1
        table.append(value)
    return table


CRC = _crc_table()


class ZipCipher:
    def __init__(self, password):
        self.keys = [0x12345678, 0x23456789, 0x34567890]
        for byte in password:
            self.update(byte)

    def update(self, byte):
        k0, k1, k2 = self.keys
        k0 = (k0 >> 8) ^ CRC[(k0 ^ byte) & 0xFF]
        k1 = (k1 + (k0 & 0xFF)) & 0xFFFFFFFF
        k1 = (k1 * 134775813 + 1) & 0xFFFFFFFF
        k2 = (k2 >> 8) ^ CRC[(k2 ^ (k1 >> 24)) & 0xFF]
        self.keys = [k0, k1, k2]

    def encrypt(self, data):
        out = bytearray()
        for byte in data:
            temp = (self.keys[2] | 2) & 0xFFFF
            out.append(byte ^ (((temp * (temp ^ 1)) >> 8) & 0xFF))
            self.update(byte)
        return bytes(out)


def encrypted_zip(name, data, password=b'lighthouse', deflate=False):
    crc = zlib.crc32(data)
    if deflate:
        compressor = zlib.compressobj(6, zlib.DEFLATED, -15)
        body = compressor.compress(data) + compressor.flush()
        method = 8
    else:
        body = data
        method = 0
    header = bytes(range(11)) + bytes([crc >> 24 & 0xFF])
    payload = ZipCipher(password).encrypt(header + body)
    raw_name = name.encode()
    local = struct.pack('<IHHHHHIIIHH', 0x04034B50, 20, 1, method, 0, 0x21,
                        crc, len(payload), len(data), len(raw_name), 0) + raw_name + payload
    central = struct.pack('<IHHHHHHIIIHHHHHII', 0x02014B50, 20, 20, 1, method, 0, 0x21,
                          crc, len(payload), len(data), len(raw_name), 0, 0, 0, 0, 0o100644 << 16, 0) + raw_name
    end = struct.pack('<IHHHHIIH', 0x06054B50, 0, 0, 1, 1, len(central), len(local), 0)
    return local + central + end


def zip_seeds():
    for method, label in [(zipfile.ZIP_STORED, 'stored'), (zipfile.ZIP_DEFLATED, 'deflate'),
                          (zipfile.ZIP_BZIP2, 'bzip2'), (zipfile.ZIP_LZMA, 'lzma')]:
        archive_seed(f'zip-{label}', ZIP, zip_with(method, MEMBERS))
    archive_seed('zip-zip64', ZIP, zip_with(zipfile.ZIP_DEFLATED, MEMBERS[:2], allowZip64=True))
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        info = zipfile.ZipInfo('big.txt')
        with archive.open(info, 'w', force_zip64=True) as member:
            member.write(b'zip64 ' * 10)
        archive.writestr('Ünïcödé/naïve.txt', 'unicode')
        archive.comment = b'a comment'
    archive_seed('zip-force64-unicode', ZIP, buffer.getvalue())
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        link = zipfile.ZipInfo('link')
        link.external_attr = (0o120777 << 16)
        archive.writestr(link, 'Docs/Guide.txt')
        archive.writestr('Docs/Guide.txt', 'guide')
        archive.writestr('../escape.txt', 'up')
        archive.writestr('/absolute.txt', 'root')
        archive.writestr('back\\slash.txt', 'win')
    archive_seed('zip-odd-names', ZIP, buffer.getvalue())
    archive_seed('zip-encrypted', ZIP | PASSWORD, encrypted_zip('secret.txt', b'secret text ' * 10))
    archive_seed('zip-encrypted-deflate', ZIP | PASSWORD,
                 encrypted_zip('Docs/secret.txt', b'secret text ' * 10, deflate=True))
    archive_seed('zip-empty', ZIP, zip_with(zipfile.ZIP_STORED, []))
    archive_seed('zip-as-bin', OTHER, zip_with(zipfile.ZIP_DEFLATED, MEMBERS[1:3]))


def tar_bytes(fmt):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode='w', format=fmt) as archive:
        folder = tarfile.TarInfo('Docs')
        folder.type = tarfile.DIRTYPE
        archive.addfile(folder)
        for name, data in [('Docs/Guide.txt', b'hello world ' * 30),
                           ('Docs/' + ('n' * 150 if fmt != tarfile.USTAR_FORMAT else 'deep/' * 20 + 'x') + '.txt', b'abc'),
                           ('Ünïcödé.txt', b'unicode')]:
            member = tarfile.TarInfo(name)
            member.size = len(data)
            member.mtime = 1_700_000_000
            archive.addfile(member, io.BytesIO(data))
        link = tarfile.TarInfo('Docs/link')
        link.type = tarfile.SYMTYPE
        link.linkname = 'Guide.txt'
        archive.addfile(link)
        hard = tarfile.TarInfo('hard')
        hard.type = tarfile.LNKTYPE
        hard.linkname = 'Docs/Guide.txt'
        archive.addfile(hard)
    return buffer.getvalue()


def tar_seeds(work):
    for fmt, label in [(tarfile.USTAR_FORMAT, 'ustar'), (tarfile.GNU_FORMAT, 'gnu'),
                       (tarfile.PAX_FORMAT, 'pax')]:
        plain = tar_bytes(fmt)
        archive_seed(f'tar-{label}', TAR, plain)
        archive_seed(f'tgz-{label}', TGZ, gzip.compress(plain, mtime=0))
        archive_seed(f'tbz-{label}', TBZ, bz2.compress(plain))
        archive_seed(f'txz-{label}', TXZ, lzma.compress(plain))
        plain_path = work / f'{label}.tar'
        plain_path.write_bytes(plain)
        packed = work / f'{label}.tar.zst'
        subprocess.run([str(SEED_MAKER), 'zst', str(plain_path), str(packed)], check=True)
        archive_seed(f'tzst-{label}', TZST, packed.read_bytes())
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode='w', format=tarfile.USTAR_FORMAT) as archive:
        for name, kind, target in [('up', tarfile.SYMTYPE, '../../etc'), ('abs', tarfile.SYMTYPE, '/etc'),
                                   ('fifo', tarfile.FIFOTYPE, ''), ('dev', tarfile.CHRTYPE, '')]:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.linkname = target
            archive.addfile(member)
    archive_seed('tar-odd-links', TAR, buffer.getvalue())


def seven_zip_seeds(work):
    members = ['Docs/', 'Docs/Guide.txt=hello world ', 'Docs/Notes.md=# Notes ', 'readme.txt=hello']
    for mode in ['solid', 'plain', 'lzma', 'bzip2', 'copy']:
        out = work / f'{mode}.7z'
        subprocess.run([str(SEED_MAKER), '7z', str(out), mode, *members], check=True)
        archive_seed(f'7z-{mode}', SEVEN, out.read_bytes())


def core_text_seeds():
    """Seeds of the core_text target: the selector byte, then the input."""
    seeds = {
        0: ['/home/alex/Documents\nsmb://nas.example/share/Folder/\n~/Music\n..\\Downloads\n'
            'file:///home/alex/%C3%A9t%C3%A9\nwinspace-archive:///home/alex/a.zip#Docs/a.txt\nnetwork:///\n'
            'smb://user@[fe80::1%25eth0]:445/share\nC:\\Users\nrecent:///\nsftp://host/path',
            'smb://SERVER\n\\\\server\\share\\folder\nmtp://Phone/Internal/DCIM'],
        1: ['{"mode": "cut", "uris": ["file:///home/alex/a.txt"], "token": "0123abcd"}',
            'copy\nfile:///home/alex/a.txt\nfile:///home/alex/b%20c.txt',
            'file:///home/alex/a.txt\r\nsmb://nas.example/share/b.txt\r\n\x001'],
        2: ["{'multiple': <true>, 'directory': <false>, 'accept_label': <'_Open'>, "
            "'filters': <[('PDF', [(uint32 0, '*.pdf'), (1, 'application/pdf')]), ('All', [(0, '*')])]>, "
            "'current_filter': <('PDF', [(uint32 0, '*.pdf')])>, "
            "'current_folder': <b'/home/alex/Documents'>, 'current_file': <b'/home/alex/a.txt'>, "
            "'current_name': <'Report.pdf'>, 'files': <[b'/home/alex/a.txt']>, "
            "'choices': <[('encoding', 'Encoding', [('utf8', 'UTF-8'), ('latin1', 'Latin-1')], 'utf8'), "
            "('reencode', 'Reencode', @a(ss) [], 'true')]>}"],
        3: ['file:///home/alex/Documents\nsmb://nas.example/share/a.txt'],
        4: ['36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue\n'
            '29 1 0:26 / /media/alex/USB\\040Stick rw shared:1 - vfat /dev/sdb1 rw\n'],
        5: ['36 35 98:0 / /home rw - ext4 /dev/sda2 rw\n40 36 0:50 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n'],
        6: ['/dev/sdb1 /media/usb vfat user,noauto 0 0\nUUID=1234-ABCD /mnt/data ext4 defaults 0 2\n'
            'SRC=/dev/sdc1 TARGET=/media/alex/Disk ROOT=/ OPTS=uhelper=udisks2\n'],
        7: ['//nas.example/share /mnt/share cifs rw,relatime,vers=3.1.1,username=alex 0 0\n'
            'gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw,nosuid 0 0\n'],
        8: ['XDG_DESKTOP_DIR="$HOME/Desktop"\nXDG_DOWNLOAD_DIR="$HOME/Downloads"\n'
            'XDG_MUSIC_DIR="/srv/music"\n# comment\nXDG_PICTURES_DIR="$HOME/Pictures\\"x"\n'],
        11: ['file:///home/alex/Projects Projects\nfile:///home/alex/Music\nsmb://nas.example/share Share\n'],
        12: ['[Desktop Entry]\nType=Service\nMimeType=text/plain;all/allfiles;\nX-KDE-MinNumberOfUrls=1\n'
             'Actions=view;edit\n[Desktop Action view]\nName=View safely\nExec=/usr/bin/cat %F\n'
             '[Desktop Action edit]\nName=Edit\nExec=sh -c "echo %u" %U %f\nIcon=edit\n'],
        13: ['Report 2024.final.pdf\nNew folder\n.hidden\nCON\na:b?c\nname.tar.gz\n' + 'x' * 300],
    }
    for selector, texts in seeds.items():
        for index, text in enumerate(texts):
            write('core_text', f'seed-{selector:02}-{index}', bytes([selector]) + text.encode())
    # A compiled catalog with two messages, one plural.
    originals = [b'', b'Copy', b'item\0items']
    translations = [b'Content-Type: text/plain; charset=UTF-8\nPlural-Forms: nplurals=2; plural=(n != 1);\n',
                    b'Kopieren', b'Element\0Elemente']
    count = len(originals)
    header = 28
    tables = header + 16 * count
    data = b''
    offsets = []
    for text in originals + translations:
        offsets.append((len(text), tables + len(data)))
        data += text + b'\0'
    mo = struct.pack('<IIIIIII', 0x950412DE, 0, count, header, header + 8 * count, 0, 0)
    for length, offset in offsets:
        mo += struct.pack('<II', length, offset)
    write('core_text', 'seed-09-mo', bytes([9]) + mo + data)
    snapshot = '@GMT-2024.01.02-03.04.05\0@GMT-2024.02.03-04.05.06\0\0'.encode('utf-16-le')
    write('core_text', 'seed-10-snapshots', bytes([10]) + struct.pack('<III', 2, 2, len(snapshot)) + snapshot)


def picture_files():
    """Small pictures in every format the app reads, made with Pillow."""
    from PIL import Image, PngImagePlugin
    pictures = {}
    image = Image.new('RGB', (12, 8), (200, 40, 40))
    for x in range(12):
        image.putpixel((x, x % 8), (10, 200, 30))

    def saved(picture, fmt, **options):
        buffer = io.BytesIO()
        picture.save(buffer, fmt, **options)
        return buffer.getvalue()

    text = PngImagePlugin.PngInfo()
    text.add_text('Title', 'Harbor at dawn')
    text.add_itxt('Description', 'Ünïcödé description', lang='en', tkey='Description')
    text.add_text('Thumb::URI', 'file:///home/alex/Pictures/harbor.png')
    text.add_text('Thumb::MTime', '1700000000')
    text.add_text('XML:com.adobe.xmp', '<x:xmpmeta><rdf:Description xmp:Rating="4"/></x:xmpmeta>')
    pictures['png'] = saved(image, 'PNG', pnginfo=text)
    pictures['png-alpha'] = saved(image.convert('RGBA'), 'PNG')
    pictures['png-palette'] = saved(image.convert('P'), 'PNG')
    exif = Image.Exif()
    exif[0x0112] = 6  # orientation
    exif[0x010F] = 'Example camera maker'
    exif[0x0110] = 'Model 1'
    exif[0x0131] = 'Fuzz seeds'
    exif[0x9C9B] = 'Title'.encode('utf-16-le') + b'\0\0'
    ifd = exif.get_ifd(0x8769)
    ifd[0x9003] = '2024:05:06 07:08:09'
    ifd[0x829A] = (1, 250)
    ifd[0x829D] = (28, 10)
    ifd[0x8827] = 200
    ifd[0x9209] = 16
    ifd[0x920A] = (50, 1)
    ifd[0x9286] = b'ASCII\0\0\0A comment'
    pictures['jpeg-exif'] = saved(image, 'JPEG', exif=exif, quality=80)
    pictures['jpeg-progressive'] = saved(image, 'JPEG', progressive=True)
    pictures['jpeg-plain'] = saved(image.convert('L'), 'JPEG')
    frames = [image, image.transpose(Image.Transpose.FLIP_LEFT_RIGHT)]
    pictures['gif'] = saved(frames[0], 'GIF', save_all=True, append_images=frames[1:], loop=0, duration=50)
    pictures['gif-still'] = saved(image.convert('P'), 'GIF')
    pictures['bmp'] = saved(image, 'BMP')
    pictures['webp-lossy'] = saved(image, 'WEBP', quality=50)
    pictures['webp-lossless'] = saved(image, 'WEBP', lossless=True)
    pictures['webp-extended'] = saved(image.convert('RGBA'), 'WEBP', exif=exif)
    pictures['tiff'] = saved(image, 'TIFF')
    pictures['ico'] = saved(image, 'ICO')
    pictures['tga'] = saved(image, 'TGA')
    pictures['tga-rle'] = saved(image, 'TGA', compression='tga_rle')
    pictures['ppm'] = saved(image, 'PPM')
    pictures['pgm'] = saved(image.convert('L'), 'PPM')
    pictures['pbm'] = saved(image.convert('1'), 'PPM')
    pictures['ppm-ascii'] = b'P3\n2 2\n255\n255 0 0  0 255 0\n0 0 255  255 255 255\n'
    pictures['xbm'] = saved(image.convert('1'), 'XBM')
    pictures['xpm'] = (b'/* XPM */\nstatic char *fuzz[] = {\n"4 2 2 1",\n"  c None",\n". c #FF0000",\n'
                       b'" .. ",\n".  ."\n};\n')
    pictures['icns'] = saved(image.resize((16, 16)), 'ICNS')
    cursor = bytearray(saved(image.resize((16, 16)), 'ICO'))
    cursor[2:4] = b'\x02\x00'
    pictures['cur'] = bytes(cursor)
    icon = saved(image.resize((16, 16)), 'ICO')
    anih = struct.pack('<9I', 36, 1, 1, 0, 0, 0, 0, 10, 1)
    frames = b'LIST' + struct.pack('<I', 4 + 8 + len(icon)) + b'fram' + b'icon' + struct.pack('<I', len(icon)) + icon
    body = b'ACON' + b'anih' + struct.pack('<I', len(anih)) + anih + frames
    pictures['ani'] = b'RIFF' + struct.pack('<I', len(body)) + body
    pictures['jp2'] = saved(image, 'JPEG2000')
    pictures['svg'] = (b'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="10">'
                       b'<rect width="16" height="10" fill="#c33"/><circle cx="5" cy="5" r="3"/></svg>')
    return pictures


def picture_seeds():
    cuts = struct.pack('<4H', 3, 17, 30, 200)
    pictures = picture_files()
    for name, data in pictures.items():
        write('picture_headers', name, cuts + data)
        write('picture_decode', name, cuts + data)
        write('media_details', name, data)
    # The decoder's answers: a 2x1 RGB picture, a refusal, and a framed answer.
    magic = b'\0OXPIX1\0'
    pixels = bytes([255, 0, 0, 0, 255, 0])
    good = (magic + b'\0' + struct.pack('<5i', 2, 1, 2, 1, 6) + b'\0' + struct.pack('<I', 1)
            + struct.pack('<I', 17) + b'tEXt::Thumb::URI' [:17] + struct.pack('<I', 4) + b'file' + pixels)
    write('picture_headers', 'answer', good)
    write('picture_headers', 'answer-large', magic + b'\x01' + struct.pack('<2i', 9000, 9000))
    write('picture_headers', 'answer-framed', b'noise\n\0OXFRM1\0' + struct.pack('<Q', len(good)) + good)
    write('picture_headers', 'request', b'\0OXREQ1\0' + struct.pack('<2i3Q', 256, 256, 1 << 26, 1 << 28, 10000))
    write('picture_headers', 'vector-answer', b'OX-VECTOR-PICTURE-1\n\0' + struct.pack('<2i', 16, 10)
          + pictures['png'])


def media_seeds():
    details = {
        'title': 'Harbor', 'subject': 'Boats', 'rating': 4, 'rating_readable': True, 'tags': 'sea; dawn',
        'comments': 'A comment', 'width': 4000, 'height': 3000, 'duration_ns': 125_000_000_000,
        'bitrate': 320000, 'frame_rate': [30000, 1001], 'channels': 2, 'sample_rate': 44100,
        'exposure_time': [1, 250], 'f_number': 2.8, 'iso': 200, 'flash': 16,
    }
    import json
    write('media_details', 'details.json', json.dumps(details).encode())
    write('media_details', 'date', b'2024:05:06 07:08:09')
    write('media_details', 'thumbnailer', b'[Thumbnailer Entry]\nTryExec=sh\nExec=sh -c "convert %i %o" %u %s\n'
          b'MimeType=image/png;image/x-portable-pixmap;\n')


def text_seeds():
    texts = {
        'utf8': '# Notes\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n'.encode(),
        'utf8-bom': b'\xef\xbb\xbfH\xc3\xa9llo\r\nline two\r\n',
        'utf16le': b'\xff\xfe' + 'Ünïcödé text\nline'.encode('utf-16-le'),
        'utf16be': b'\xfe\xff' + 'Ünïcödé text\nline'.encode('utf-16-be'),
        'latin1': 'caf\xe9 cr\xe8me\n'.encode('latin-1'),
        'long-line': b'x' * 3000 + b'\n' + b'y\n' * 300,
        'code': b'#include <stdio.h>\nint main(void) { printf("%d\\n", 1); }\n\t\x1b[0m',
        'rtf': (rb'{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Calibri;}}{\colortbl;\red255\green0\blue0;}'
                rb'\f0\fs22 Hello {\b bold} \u8364? caf\'e9 \uc2\u55357\'3f\'3f\u56832\'3f\'3f\par'
                rb'{\*\generator Fuzz;}\line\tab end\par}'),
        'rtf-cp': rb'{\rtf1\ansi\ansicpg1251 \'cf\'f0\'e8\par{\pict\pngblip 89504e47}}',
        'desktop-link': b'[Desktop Entry]\nType=Link\nName=Projects\nURL=file:///home/alex/Projects\n',
        'desktop-web': b'[Desktop Entry]\nType=Link\nName[de]=Seite\nURL=https://example.org/a?b=c\nIcon=web\n',
    }
    for name, data in texts.items():
        write('text_preview', name, data)


def core_formats_seeds():
    import json
    release = {
        'tag_name': 'v2.0.2', 'draft': False, 'prerelease': False, 'body': '## Changes\n- Fixes',
        'assets': [{'name': 'openxplorer_2.0.2_amd64.deb', 'size': 1234567,
                    'digest': 'sha256:' + '0' * 64,
                    'browser_download_url': 'https://github.com/AKolenda/openxplorer/releases/download/v2.0.2/'
                                            'openxplorer_2.0.2_amd64.deb'}],
    }
    tab = {'uri': 'file:///home/alex/Documents', 'history': ['file:///home/alex', 'file:///home/alex/Documents'],
           'index': 1, 'selection': ['file:///home/alex/Documents/a.txt'], 'scroll': 12.5,
           'view': 'details', 'sort': 'name'}
    seeds = {
        0: 'smb://nas.example/Share/Folder/Sub\nsmb://192.168.1.20/Media$\nsmb://nas/My Share/a%20b/..\n'
           '\\\\nas\\share\\folder\nsmb://user@nas:445/share',
        1: '--share\0//nas.example/Share\0--remove\0--plan\nalex:x:1000:1000:Alex:/home/alex:/bin/bash',
        2: '--item\x002049:1234567\x00--\x00/home/alex/Old folder',
        3: json.dumps(release),
        4: 'https://api.github.com/repos/AKolenda/openxplorer/releases/latest\n'
           'https://github.com/AKolenda/openxplorer/releases/download/v2.0.2/a.deb\n2.0.10\nv2.1.0-rc1',
        5: 'report kind:document date:>2024-01-01 size:<10MB "exact words" *.pdf -draft\nReport 2024 final.PDF',
        6: 'root:x:0:0:root:/root:/bin/bash\nalex:x:1000:1000:Alex,,,:/home/alex:/bin/bash\n'
           '[Music]\npath=/home/alex/Music\ncomment=Songs\nusershare_acl=Everyone:R,\nguest_ok=n\n',
        7: json.dumps({'version': 1, 'tabs': [tab], 'active': 0}),
        8: '*.tx?[!a-c]',
    }
    for selector, text in seeds.items():
        write('core_formats', f'seed-{selector:02}', bytes([selector]) + text.encode())
    write('core_formats', 'seed-07-tab', bytes([7]) + json.dumps(tab).encode())


def app_protocol_seeds():
    seeds = {
        'player': 'ox-player 1\nready 1500000 1 1 1\nposition 250000\nframe 4 2 16\nseeked\nseek-failed\n'
                  'ended\nfailed No decoder for this file\nplay\npause\nseek 1000\nvolume 0.5 0\nquit\n',
        'parent': 'x11:0x3a00007',
        'wayland': 'wayland:abc-123_handle',
        'names': '"Report.pdf" "Notes 2024.txt" "a b.c"',
        'search': 'dark mode  file  extensions hidden items',
        'arguments': '--new-window\0--select\0file:///home/alex/a.txt\0--\0/home/alex/Documents\0smb://nas/share',
    }
    for name, text in seeds.items():
        write('app_protocols', name, text.encode())


def media_stream_seeds(work):
    """Tiny tagged audio and video files, made with ffmpeg when it is installed."""
    import shutil
    if not shutil.which('ffmpeg'):
        print('ffmpeg is missing: no media_streams seeds', file=sys.stderr)
        return
    tags = ['-metadata', 'title=Harbor', '-metadata', 'artist=Example Artist', '-metadata', 'album=Seeds',
            '-metadata', 'date=2024', '-metadata', 'track=3', '-metadata', 'genre=Ambient',
            '-metadata', 'composer=Someone', '-metadata', 'comment=A comment']
    audio = ['-f', 'lavfi', '-i', 'sine=frequency=440:duration=0.2:sample_rate=8000']
    video = ['-f', 'lavfi', '-i', 'testsrc=size=16x16:rate=5:duration=0.4']
    outputs = {
        'mp3-id3.mp3': audio + ['-ac', '1', '-b:a', '8k', '-id3v2_version', '3'],
        'audio.m4a': audio + ['-ac', '1', '-c:a', 'aac', '-b:a', '8k'],
        'audio.ogg': audio + ['-ac', '1', '-c:a', 'libvorbis', '-q:a', '0'],
        'audio.opus': audio + ['-ac', '1', '-c:a', 'libopus', '-b:a', '6k'],
        'audio.flac': audio + ['-ac', '1'],
        'audio.wav': audio + ['-ac', '1', '-c:a', 'pcm_u8'],
        'video.mp4': video + audio + ['-c:v', 'mpeg4', '-q:v', '31', '-c:a', 'aac', '-b:a', '8k', '-shortest'],
        'video.mkv': video + ['-c:v', 'mpeg4', '-q:v', '31'],
        'video.webm': video + ['-c:v', 'libvpx', '-b:v', '20k'],
        'video.avi': video + ['-c:v', 'mjpeg', '-q:v', '31'],
    }
    for name, arguments in outputs.items():
        out = work / name
        done = subprocess.run(['ffmpeg', '-v', 'error', '-y', *arguments, *tags, str(out)],
                              capture_output=True)
        if done.returncode == 0 and out.stat().st_size <= 65536:
            write('media_streams', name, out.read_bytes())
        else:
            print('ffmpeg could not write', name, done.stderr.decode()[-200:], file=sys.stderr)


def build_seed_maker():
    if not SEED_MAKER.exists():
        subprocess.run(['cargo', '+stable', 'build', '--release', '--manifest-path',
                        str(HERE / 'seed-maker' / 'Cargo.toml')], check=True)


def main():
    import tempfile
    build_seed_maker()
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        zip_seeds()
        tar_seeds(work)
        seven_zip_seeds(work)
        media_stream_seeds(work)
    core_text_seeds()
    core_formats_seeds()
    picture_seeds()
    media_seeds()
    text_seeds()
    app_protocol_seeds()
    for extra in sys.argv[1:]:
        print('unknown argument', extra, file=sys.stderr)
    for folder in sorted(CORPUS.iterdir()):
        print(folder.name, len(list(folder.iterdir())), 'seeds')


if __name__ == '__main__':
    main()
