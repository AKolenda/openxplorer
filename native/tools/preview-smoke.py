#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Arrow through hundreds of real-world-style and broken files with the
Preview pane, then the Details pane, and fail on any sign of a crash.

The run is private: its own Xvfb display, session bus and temporary HOME and
XDG folders, so it never reaches the user's desktop or files. It makes its
own sample files (pictures, SVGs, PDFs, MP3s, MP4s, HEIC-like files, ZIPs,
7z archives, text and RTF, and cut-short or mangled copies of them), starts
the built app on that folder, turns on the Preview pane (Alt+P), presses Down
through every file, does the same with the Details pane (Alt+Shift+P), and
then checks that the app is still running and that its standard error holds
no "Bail out", CRITICAL, "panicked" or signal report.

    python3 native/tools/preview-smoke.py [--binary PATH] [--files 300]

Needs Xvfb, xvfb-run, dbus-run-session and xdotool; ffmpeg and ImageMagick's
convert make the media and pictures (files they cannot make are left out).
"""
from __future__ import annotations

import argparse
import io
import os
from pathlib import Path
import random
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
import zipfile
import zlib

NATIVE = Path(__file__).resolve().parents[1]
DEFAULT_BINARY = NATIVE / 'target' / 'debug' / 'openxplorer-native'
BAD_OUTPUT = re.compile(r'Bail out|CRITICAL|panicked|\bSIG[A-Z]+\b|Segmentation fault|Aborted|core dumped')
INNER = '--inner'


def run_quiet(*command: str) -> bool:
    """Run command, reporting only whether it worked."""
    try:
        return subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                              timeout=60, check=False).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def png_bytes(width: int, height: int, seed: int) -> bytes:
    """A PNG of a gradient, written with zlib alone."""
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = b''.join(b'\0' + bytes(((x * 7 + y * 3 + seed) % 256) for x in range(width * 3))
                    for y in range(height))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def pdf_bytes(pages: int) -> bytes:
    """A small, valid PDF of `pages` pages of text."""
    objects = ['<< /Type /Catalog /Pages 2 0 R >>',
               '<< /Type /Pages /Kids [%s] /Count %d >>' % (
                   ' '.join(f'{3 + 2 * page} 0 R' for page in range(pages)), pages)]
    for page in range(pages):
        objects.append('<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents %d 0 R '
                       '/Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica '
                       '>> >> >> >>' % (4 + 2 * page))
        text = f'BT /F1 18 Tf 40 300 Td (Page {page + 1} of a sample) Tj ET'
        objects.append(f'<< /Length {len(text)} >>\nstream\n{text}\nendstream')
    out = io.BytesIO()
    out.write(b'%PDF-1.4\n')
    offsets = []
    for number, body in enumerate(objects, start=1):
        offsets.append(out.tell())
        out.write(f'{number} 0 obj\n{body}\nendobj\n'.encode())
    start = out.tell()
    out.write(f'xref\n0 {len(objects) + 1}\n0000000000 65535 f \n'.encode())
    for offset in offsets:
        out.write(f'{offset:010d} 00000 n \n'.encode())
    out.write(f'trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n'.encode())
    return out.getvalue()


def seven_zip_bytes(name: str, data: bytes) -> bytes:
    """A 7z holding one file stored with the Copy method, written by hand."""
    names = (name.encode('utf-16-le') + b'\0\0')
    def size(value: int) -> bytes:
        if value < 0x80:
            return bytes([value])
        if value < 0x4000:
            return bytes([0x80 | (value >> 8), value & 0xFF])
        return bytes([0xC0 | (value >> 16), value & 0xFF, (value >> 8) & 0xFF])
    header = (b'\x01\x04'
              + b'\x06\x00\x01\x09' + size(len(data)) + b'\x00'
              + b'\x07\x0b\x01\x00\x01\x01\x00\x0c' + size(len(data)) + b'\x00'
              + b'\x08\x0a\x01' + struct.pack('<I', zlib.crc32(data)) + b'\x00'
              + b'\x00'
              + b'\x05\x01'
              + b'\x11' + size(len(names) + 1) + b'\x00' + names
              + b'\x00'
              + b'\x00')
    next_offset = len(data)
    start = struct.pack('<QQI', next_offset, len(header), zlib.crc32(header))
    return (b"7z\xbc\xaf\x27\x1c\x00\x04" + struct.pack('<I', zlib.crc32(start)) + start + data + header)


def mangle(data: bytes, rng: random.Random) -> bytes:
    """A broken copy: cut short, or with bytes flipped and inserted."""
    if not data:
        return b'\xff'
    if rng.random() < 0.5:
        return data[:rng.randrange(0, max(1, len(data)))]
    out = bytearray(data)
    for _ in range(rng.randrange(1, 40)):
        at = rng.randrange(0, len(out))
        if rng.random() < 0.7:
            out[at] = rng.randrange(256)
        else:
            out[at:at] = bytes([rng.randrange(256)]) * rng.randrange(1, 64)
    return bytes(out)


def make_samples(folder: Path, work: Path) -> dict[str, bytes]:
    """One good sample of each kind, keyed by extension."""
    samples: dict[str, bytes] = {'png': png_bytes(320, 200, 5), 'pdf': pdf_bytes(3)}
    tools = {
        'jpg': ['convert', '-size', '640x480', 'gradient:#336699-#ffcc00', 'JPG'],
        'gif': ['convert', '-size', '64x64', 'plasma:', 'GIF'],
        'tif': ['convert', '-size', '120x80', 'xc:#3366aa', 'TIFF'],
        'webp': ['convert', '-size', '120x80', 'xc:#aa3366', 'WEBP'],
        'bmp': ['convert', '-size', '90x60', 'xc:#33aa66', 'BMP'],
        'ico': ['convert', '-size', '48x48', 'xc:#123456', 'ICO'],
        'mp3': ['ffmpeg', '-v', 'quiet', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=3',
                '-metadata', 'title=Sample tone', '-metadata', 'artist=Example Band', '-c:a', 'libmp3lame'],
        'mp4': ['ffmpeg', '-v', 'quiet', '-f', 'lavfi', '-i', 'testsrc=duration=2:size=320x240:rate=15',
                '-f', 'lavfi', '-i', 'sine=duration=2', '-shortest', '-c:v', 'libx264', '-c:a', 'aac',
                '-pix_fmt', 'yuv420p'],
    }
    for extension, command in tools.items():
        out = work / f'sample.{extension}'
        if command[0] == 'convert':
            command = command[:-1] + [f'{command[-1]}:{out}']
        else:
            command = command + [str(out)]
        if shutil.which(command[0]) and run_quiet(*command) and out.exists():
            samples[extension] = out.read_bytes()
    samples['svg'] = (b'<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120">'
                      b'<rect width="200" height="120" fill="#246"/><circle cx="60" cy="60" r="40" '
                      b'fill="#fc0"/></svg>')
    # HEIC-like: an ISO box header naming heic, then no real image data.
    samples['heic'] = (struct.pack('>I', 24) + b'ftypheic' + b'\0\0\0\0mif1heic'
                       + struct.pack('>I', 16) + b'meta' + bytes(8) + os.urandom(2000))
    archive = io.BytesIO()
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as zipped:
        zipped.writestr('Docs/Guide.txt', 'hello world\n' * 40)
        zipped.writestr('Pictures/tiny.png', png_bytes(8, 8, 1))
    samples['zip'] = archive.getvalue()
    samples['7z'] = seven_zip_bytes('Guide.txt', b'hello from a 7z archive\n' * 10)
    samples['txt'] = 'Meeting notes\n'.encode() * 50
    samples['rtf'] = rb'{\rtf1\ansi{\fonttbl{\f0 Arial;}}\f0 Quarterly \b summary\b0 \par Second line}'
    samples['md'] = b'# Plan\n\n- one\n- two\n'
    samples['rs'] = b'fn main() {\n    println!("hello");\n}\n'
    return samples


def make_corpus(folder: Path, count: int, work: Path) -> int:
    """Write `count` files into folder: good samples and broken copies."""
    rng = random.Random(20261119)
    samples = make_samples(folder, work)
    kinds = sorted(samples)
    for index in range(count):
        extension = kinds[index % len(kinds)]
        data = samples[extension]
        broken = index % 3 == 2
        if broken:
            data = mangle(data, rng)
        name = f'{index:03d} {"broken " if broken else ""}sample.{extension}'
        (folder / name).write_bytes(data)
    return len(kinds)


def xdotool(*arguments: str) -> str:
    """Run xdotool, returning its output."""
    return subprocess.run(['xdotool', *arguments], capture_output=True, text=True,
                          timeout=30, check=False).stdout.strip()


def arrow_through(window: str, count: int, delay: float) -> None:
    """Press Home, then Down count times."""
    xdotool('key', '--window', window, 'Home')
    for _ in range(count):
        xdotool('key', '--window', window, 'Down')
        time.sleep(delay)


def inner(arguments: argparse.Namespace) -> int:
    """The run itself, inside the private display and session bus."""
    root = Path(tempfile.mkdtemp(prefix='ox-preview-smoke-'))
    try:
        for name in ('home', 'config', 'cache', 'data', 'state', 'runtime'):
            (root / name).mkdir(mode=0o700)
        environment = dict(os.environ, HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'config'),
                           XDG_CACHE_HOME=str(root / 'cache'), XDG_DATA_HOME=str(root / 'data'),
                           XDG_STATE_HOME=str(root / 'state'), XDG_RUNTIME_DIR=str(root / 'runtime'),
                           GTK_A11Y='none', NO_AT_BRIDGE='1')
        for variable in ('WAYLAND_DISPLAY', 'GTK_USE_PORTAL'):
            environment.pop(variable, None)
        folder = root / 'home' / 'Mixed files'
        folder.mkdir()
        work = root / 'work'
        work.mkdir()
        kinds = make_corpus(folder, arguments.files, work)
        print(f'Made {arguments.files} files of {kinds} kinds in a private folder.')
        log_path = root / 'stderr.log'
        with open(log_path, 'wb') as log:
            app = subprocess.Popen([str(arguments.binary), str(folder)], env=environment,
                                   stdout=log, stderr=log, start_new_session=True)
        try:
            window = ''
            deadline = time.monotonic() + 60
            while not window and time.monotonic() < deadline and app.poll() is None:
                window = xdotool('search', '--onlyvisible', '--class', 'winspace').split('\n')[-1]
                time.sleep(0.5)
            if not window:
                print('The window did not appear:\n' + log_path.read_text(errors='replace')[-3000:])
                return 1
            time.sleep(3)
            xdotool('windowactivate', '--sync', window)
            xdotool('windowfocus', '--sync', window)
            for pane, keys in (('Preview', 'alt+p'), ('Details', 'alt+shift+p')):
                xdotool('key', '--window', window, keys)
                time.sleep(1)
                started = time.monotonic()
                arrow_through(window, arguments.files, arguments.delay)
                time.sleep(3)
                if arguments.screenshot:
                    # A picture of the screen, for a look that the pane shows.
                    run_quiet('import', '-window', 'root', f'{arguments.screenshot}-{pane.lower()}.png')
                alive = app.poll() is None
                print(f'{pane} pane: {arguments.files} files in {time.monotonic() - started:.0f} s, '
                      f'app {"running" if alive else "ENDED with " + str(app.returncode)}')
                if not alive:
                    break
            # Back and forth once more, fast, with the selection jumping.
            if app.poll() is None:
                for key in ['End', 'Home', 'Page_Down', 'Page_Down', 'Page_Up', 'End'] * 5:
                    xdotool('key', '--window', window, key)
                    time.sleep(0.05)
                time.sleep(3)
            alive = app.poll() is None
        finally:
            if app.poll() is None:
                os.killpg(app.pid, signal.SIGTERM)
                try:
                    app.wait(10)
                except subprocess.TimeoutExpired:
                    os.killpg(app.pid, signal.SIGKILL)
        output = log_path.read_text(errors='replace')
        bad = [line for line in output.splitlines() if BAD_OUTPUT.search(line)]
        crash_log = root / 'cache' / 'winspace' / 'crash.log'
        print(f'stderr: {len(output.splitlines())} lines, {len(bad)} bad')
        for line in bad[:40]:
            print('  ' + line)
        if crash_log.exists():
            print('crash.log:\n' + crash_log.read_text(errors='replace')[:4000])
        if arguments.keep_log:
            shutil.copy(log_path, arguments.keep_log)
        ok = alive and not bad and not crash_log.exists()
        print('PASS' if ok else 'FAIL')
        return 0 if ok else 1
    finally:
        shutil.rmtree(root, ignore_errors=True)


def main() -> int:
    """Parse the command line and run inside a private session."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--binary', type=Path, default=DEFAULT_BINARY)
    parser.add_argument('--files', type=int, default=300)
    parser.add_argument('--delay', type=float, default=0.05, help='seconds between key presses')
    parser.add_argument('--keep-log', type=Path, help='copy the app\'s standard error here')
    parser.add_argument('--screenshot', help='save a picture of the screen after each pass, named PREFIX-pane.png')
    parser.add_argument(INNER, action='store_true', help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.inner:
        return inner(arguments)
    for tool in ('xvfb-run', 'Xvfb', 'dbus-run-session', 'xdotool'):
        if not shutil.which(tool):
            print(f'{tool} is required.', file=sys.stderr)
            return 2
    if not arguments.binary.exists():
        print(f'Build the app first: {arguments.binary} is missing.', file=sys.stderr)
        return 2
    environment = {key: value for key, value in os.environ.items()
                   if key not in ('DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS')}
    command = ['xvfb-run', '-a', '-s', '-screen 0 1440x1000x24', 'dbus-run-session', '--',
               sys.executable, str(Path(__file__).resolve()), INNER, '--binary', str(arguments.binary),
               '--files', str(arguments.files), '--delay', str(arguments.delay)]
    if arguments.keep_log:
        command += ['--keep-log', str(arguments.keep_log.resolve())]
    if arguments.screenshot:
        command += ['--screenshot', str(Path(arguments.screenshot).resolve())]
    return subprocess.run(command, env=environment, check=False).returncode


if __name__ == '__main__':
    sys.exit(main())
