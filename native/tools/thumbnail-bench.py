#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Time how long a folder of photos takes to show its thumbnails, cold and revisited.

The run is private: its own Xvfb display, session bus and temporary HOME and
XDG folders (so its own ~/.cache/thumbnails), and, when started as root, an
unprivileged user of its own choosing (--user). It makes a folder of photos
(JPEGs of a few megapixels, each file different) and a folder of photos,
videos and PDFs, then, for each folder, opens it in Large icons and reads the
app's trace (OX_THUMBNAIL_TRACE) until every bound tile is settled:

  cold        the first visit, with an empty thumbnail cache;
  revisit     Alt+Up then Alt+Left in the same window;
  restart     the app quit and started again on the folder;
  cache drop  the app quit, the folder's and the cache's pages dropped from
              memory (posix_fadvise DONTNEED, as after a reboot), and started
              again.

For each it prints the time from the launch (or the key press) to the last
visible thumbnail, how many visible tiles were drawn already showing their
thumbnail (on the frame they first appeared in), and how many cache reads,
pictures made, desktop thumbnailers, decoder requests and decoder processes it
took.

    python3 native/tools/thumbnail-bench.py [--binary PATH] [--photos 2000] [--json OUT]

Needs Xvfb, xvfb-run, dbus-run-session and xdotool, Pillow to make the
photos, and ffmpeg for the videos (left out without it).
"""
from __future__ import annotations

import argparse
import io
import json
import os
from pathlib import Path
import random
import shutil
import signal
import subprocess
import sys
import tempfile
import time

NATIVE = Path(__file__).resolve().parents[1]
DEFAULT_BINARY = NATIVE / 'target' / 'release' / 'openxplorer-native'
INNER = '--inner'
# Events the app writes (crates/ox-app/src/thumbnails/trace.rs) that are
# counted for each phase.
COUNTED = ('cache-read', 'cache-batch', 'kept-hit', 'generate', 'thumbnailer', 'service',
           'decode-request', 'decoder-start', 'gio-query', 'cache-check', 'fail-written',
           'fail-hit', 'written')
# How long the trace must stay quiet for a phase to count as settled.
QUIET_SECONDS = 2.5
# How long a phase may take at most.
PHASE_LIMIT = 300.0
# A tile drawn this soon after it was bound showed its thumbnail on its first
# frame (one frame at 60 Hz).
FIRST_FRAME_US = 16_700


def monotonic_us() -> int:
    """CLOCK_MONOTONIC in microseconds, the clock of the app's trace."""
    return time.clock_gettime_ns(time.CLOCK_MONOTONIC) // 1000


def xdotool(*arguments: str) -> str:
    """Run xdotool, returning its output."""
    return subprocess.run(['xdotool', *arguments], capture_output=True, text=True,
                          timeout=30, check=False).stdout.strip()


def make_photos(folder: Path, count: int, seed: int) -> None:
    """count JPEGs of 2400 x 1600 pixels, each file's bytes its own."""
    from PIL import Image, ImageDraw, ImageFilter  # noqa: PLC0415 - only the bench needs Pillow

    rng = random.Random(seed)
    bases = []
    for index in range(16):
        width, height = 2400, 1600
        picture = Image.effect_noise((width // 8, height // 8), 60).convert('RGB')
        picture = picture.resize((width, height), Image.Resampling.BICUBIC)
        draw = ImageDraw.Draw(picture)
        for _ in range(30):
            x, y = rng.randrange(width), rng.randrange(height)
            radius = rng.randrange(40, 400)
            color = tuple(rng.randrange(256) for _ in range(3))
            draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=color)
        picture = picture.filter(ImageFilter.GaussianBlur(2))
        buffer = io.BytesIO()
        picture.save(buffer, 'JPEG', quality=88)
        bases.append(buffer.getvalue())
        del index
    for index in range(count):
        # Bytes after the end of the picture make each file different
        # without changing what it shows.
        data = bases[index % len(bases)] + f'ox-bench-{seed}-{index}'.encode()
        (folder / f'IMG_{index + 1:05d}.jpg').write_bytes(data)


def pdf_bytes(number: int) -> bytes:
    """A small one-page PDF that says its number."""
    text = f'BT /F1 28 Tf 40 300 Td (Sample {number}) Tj ET'
    objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
               '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 4 0 R /Resources '
               '<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >>',
               f'<< /Length {len(text)} >>\nstream\n{text}\nendstream']
    out = io.BytesIO()
    out.write(b'%PDF-1.4\n')
    offsets = []
    for index, body in enumerate(objects, start=1):
        offsets.append(out.tell())
        out.write(f'{index} 0 obj\n{body}\nendobj\n'.encode())
    start = out.tell()
    out.write(f'xref\n0 {len(objects) + 1}\n0000000000 65535 f \n'.encode())
    for offset in offsets:
        out.write(f'{offset:010d} 00000 n \n'.encode())
    out.write(f'trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n'.encode())
    return out.getvalue()


def make_mixed(folder: Path, count: int, work: Path) -> None:
    """A folder of photos, short videos and PDFs, by turns in the order of
    their names, so the first screen shows all three."""
    make_photos(work, count, seed=7)
    for index, photo in enumerate(sorted(work.glob('IMG_*.jpg'))):
        if index % 3 == 0:
            photo.rename(folder / f'Item {index + 1:04d}.jpg')
    video = b''
    if shutil.which('ffmpeg'):
        sample = work / 'sample.mp4'
        subprocess.run(['ffmpeg', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i',
                        'testsrc=size=640x360:rate=24:duration=2', '-pix_fmt', 'yuv420p', str(sample)],
                       check=False, timeout=120)
        if sample.exists():
            video = sample.read_bytes()
    for index in range(count):
        if index % 3 == 1 and video:
            (folder / f'Item {index + 1:04d}.mp4').write_bytes(video + f'ox-bench-{index}'.encode())
        elif index % 3 == 2:
            (folder / f'Item {index + 1:04d}.pdf').write_bytes(pdf_bytes(index + 1))


def drop_pages(*folders: Path) -> int:
    """Drops the pages of every file under folders from memory, as a reboot
    would: how many files."""
    dropped = 0
    for folder in folders:
        for path in folder.rglob('*'):
            if not path.is_file() or path.is_symlink():
                continue
            try:
                descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
            except OSError:
                continue
            try:
                os.posix_fadvise(descriptor, 0, 0, os.POSIX_FADV_DONTNEED)
                dropped += 1
            finally:
                os.close(descriptor)
    return dropped


def read_trace(path: Path, since: int) -> list[tuple[int, str, str]]:
    """The trace's events from since (CLOCK_MONOTONIC microseconds) on."""
    events = []
    try:
        text = path.read_text(errors='replace')
    except OSError:
        return events
    for line in text.splitlines():
        parts = line.split(' ', 2)
        if len(parts) < 2 or not parts[0].isdigit():
            continue
        stamp = int(parts[0])
        if stamp >= since:
            events.append((stamp, parts[1], parts[2] if len(parts) > 2 else ''))
    return events


def wait_until_settled(trace: Path, since: int, app: subprocess.Popen[bytes]) -> list[tuple[int, str, str]]:
    """Waits until the trace has been quiet for QUIET_SECONDS after a bind."""
    deadline = time.monotonic() + PHASE_LIMIT
    last_count = -1
    quiet_since = time.monotonic()
    while time.monotonic() < deadline and app.poll() is None:
        events = read_trace(trace, since)
        if len(events) != last_count:
            last_count = len(events)
            quiet_since = time.monotonic()
        elif any(kind == 'bind' for _, kind, _ in events) and time.monotonic() - quiet_since >= QUIET_SECONDS:
            return events
        time.sleep(0.2)
    return read_trace(trace, since)


def summarize(name: str, events: list[tuple[int, str, str]], since: int, on_screen: int) -> dict[str, object]:
    """What a phase cost, from its events. GTK binds many more tiles than
    fit on the screen (up to thirty rows of a grid), so the first on_screen
    bound tiles in the order of their names, which is the folder's order,
    are the visible ones."""
    bound: dict[str, int] = {}
    shown: dict[str, int] = {}
    # Tiles that showed their thumbnail without first showing their type
    # icon: on the frame they were bound, or after waiting without it.
    no_flash: set[str] = set()
    held: set[str] = set()
    for stamp, kind, detail in events:
        if kind == 'bind':
            bound[detail] = stamp
            shown.pop(detail, None)
            held.discard(detail)
            no_flash.discard(detail)
        elif kind == 'unbind':
            bound.pop(detail, None)
            shown.pop(detail, None)
        elif kind == 'icon-held':
            held.add(detail)
        elif kind == 'icon-revealed':
            held.discard(detail)
        elif kind in ('shown', 'shown-at-bind') and detail in bound and detail not in shown:
            shown[detail] = stamp
            if kind == 'shown-at-bind' or detail in held:
                no_flash.add(detail)
    visible = sorted(bound)[:on_screen]
    with_picture = [uri for uri in visible if uri in shown]
    all_bound_shown = [shown[uri] for uri in bound if uri in shown]
    latencies = sorted(shown[uri] - bound[uri] for uri in with_picture)
    first_frame = sum(1 for latency in latencies if latency <= FIRST_FRAME_US)
    counts = {kind: sum(1 for _, seen, _ in events if seen == kind) for kind in COUNTED}
    # Pictures decoded: one a request, and each picture of a batch.
    counts['decodes'] = counts['decode-request'] + sum(
        int(detail) for _, seen, detail in events if seen == 'decode-batch' and detail.isdigit())
    first_bind = min(bound.values(), default=since)
    last_shown = max((shown[uri] for uri in with_picture), default=since)
    return {
        'phase': name,
        'visible': len(visible),
        'bound': len(bound),
        'bound_with_picture': len(all_bound_shown),
        'all_bound_ms': round((max(all_bound_shown, default=since) - since) / 1000, 1),
        'with_picture': len(with_picture),
        'all_visible_ms': round((last_shown - since) / 1000, 1),
        'all_visible_after_bind_ms': round((last_shown - first_bind) / 1000, 1),
        'first_bind_ms': round((first_bind - since) / 1000, 1),
        'median_bind_to_picture_ms': round(latencies[len(latencies) // 2] / 1000, 1) if latencies else None,
        'on_first_frame': first_frame,
        'no_icon_flash': sum(1 for uri in visible if uri in no_flash),
        **counts,
    }


def launch(binary: Path, folder: Path, environment: dict[str, str], log: Path) -> tuple[subprocess.Popen[bytes], str]:
    """Starts the app on folder and waits for its window, sized the same each run."""
    with open(log, 'ab') as output:
        app = subprocess.Popen([str(binary), str(folder)], env=environment, stdout=output, stderr=output,
                               start_new_session=True)
    window = ''
    deadline = time.monotonic() + 60
    while not window and time.monotonic() < deadline and app.poll() is None:
        window = xdotool('search', '--onlyvisible', '--class', 'winspace').split('\n')[-1]
        time.sleep(0.05)
    if window:
        xdotool('windowsize', '--sync', window, '1400', '950')
        xdotool('windowmove', '--sync', window, '0', '0')
        xdotool('windowactivate', '--sync', window)
    return app, window


def stop(app: subprocess.Popen[bytes]) -> None:
    """Quits the app and everything it started."""
    if app.poll() is None:
        os.killpg(app.pid, signal.SIGTERM)
        try:
            app.wait(10)
        except subprocess.TimeoutExpired:
            os.killpg(app.pid, signal.SIGKILL)
            app.wait(10)


def run_folder(label: str, folder: Path, root: Path, binary: Path, environment: dict[str, str],
               screenshots: Path | None, on_screen: int) -> list[dict[str, object]]:
    """The four phases for one folder."""
    trace = Path(environment['OX_THUMBNAIL_TRACE'])
    log = root / 'stderr.log'
    results = []

    def capture(phase: str) -> None:
        if screenshots:
            subprocess.run(['import', '-window', 'root', str(screenshots / f'{label}-{phase}.png')],
                           check=False, timeout=60)

    since = monotonic_us()
    app, window = launch(binary, folder, environment, log)
    try:
        if not window:
            raise SystemExit('The window did not appear:\n' + log.read_text(errors='replace')[-3000:])
        results.append(summarize('cold', wait_until_settled(trace, since, app), since, on_screen))
        capture('cold')
        xdotool('key', '--window', window, 'alt+Up')
        time.sleep(3)
        since = monotonic_us()
        xdotool('key', '--window', window, 'alt+Left')
        results.append(summarize('revisit', wait_until_settled(trace, since, app), since, on_screen))
        capture('revisit')
    finally:
        stop(app)
    since = monotonic_us()
    app, window = launch(binary, folder, environment, log)
    try:
        results.append(summarize('restart', wait_until_settled(trace, since, app), since, on_screen))
        capture('restart')
    finally:
        stop(app)
    drop_pages(folder, Path(environment['XDG_CACHE_HOME']))
    since = monotonic_us()
    app, window = launch(binary, folder, environment, log)
    try:
        results.append(summarize('cache drop', wait_until_settled(trace, since, app), since, on_screen))
        capture('cache-drop')
    finally:
        stop(app)
    for result in results:
        result['folder'] = label
    return results


def run_scroll(folder: Path, root: Path, binary: Path, environment: dict[str, str]) -> dict[str, object]:
    """Scrolls a folder seen for the first time with Page Down while its
    thumbnails are made, and counts the main loop's stalls (the app's
    `stall` events: no 5 ms timer for 34 ms or more)."""
    trace = Path(environment['OX_THUMBNAIL_TRACE'])
    since = monotonic_us()
    app, window = launch(binary, folder, environment, root / 'stderr.log')
    try:
        if not window:
            raise SystemExit('The window did not appear')
        wait_until_settled(trace, since, app)
        started = monotonic_us()
        for _ in range(40):
            xdotool('key', '--window', window, 'Page_Down')
            time.sleep(0.1)
        ended = monotonic_us()
        events = wait_until_settled(trace, started, app)
    finally:
        stop(app)
    stalls = [int(detail) for stamp, kind, detail in events
              if kind == 'stall' and started <= stamp <= ended and detail.isdigit()]
    made = sum(1 for stamp, kind, _ in events if kind in ('generate', 'thumbnailer') and stamp <= ended)
    return {'phase': 'fast scroll', 'folder': 'photos (new)', 'stalls': len(stalls),
            'longest_stall_ms': max(stalls, default=0), 'made_while_scrolling': made,
            'scroll_ms': round((ended - started) / 1000)}


def print_table(results: list[dict[str, object]]) -> None:
    """The results as a Markdown table."""
    columns = [('folder', 'Folder'), ('phase', 'Visit'), ('first_bind_ms', 'First tile (ms)'),
               ('all_visible_ms', 'All visible (ms)'), ('all_visible_after_bind_ms', 'After first tile (ms)'),
               ('median_bind_to_picture_ms', 'Bind to picture, median (ms)'),
               ('on_first_frame', 'On first frame'), ('no_icon_flash', 'No icon flash'),
               ('with_picture', 'Shown / visible'),
               ('all_bound_ms', 'All bound (ms)'), ('bound_with_picture', 'Bound shown'),
               ('cache-read', 'Cache reads'), ('generate', 'Made'), ('thumbnailer', 'Thumbnailers'),
               ('decodes', 'Decodes'), ('decoder-start', 'Decoder starts')]
    print('| ' + ' | '.join(title for _, title in columns) + ' |')
    print('|' + '---|' * len(columns))
    for result in results:
        cells = []
        for key, _ in columns:
            value = result.get(key)
            if key == 'with_picture':
                value = f"{result['with_picture']} / {result['visible']}"
            if key == 'bound_with_picture':
                value = f"{result['bound_with_picture']} / {result['bound']}"
            if key == 'no_icon_flash':
                value = f"{result['no_icon_flash']} / {result['visible']}"
            if key == 'on_first_frame':
                value = f"{result['on_first_frame']} / {result['visible']}"
            if key == 'cache-read' and result.get('cache-batch'):
                value = f"{result['cache-read']} ({result['cache-batch']} batches)"
            cells.append(str(value))
        print('| ' + ' | '.join(cells) + ' |')


def inner(arguments: argparse.Namespace) -> int:
    """The run itself, inside the private display and session bus."""
    root = Path(tempfile.mkdtemp(prefix='ox-thumbnail-bench-'))
    try:
        for name in ('home', 'config', 'cache', 'data', 'state', 'runtime'):
            (root / name).mkdir(mode=0o700)
        environment = dict(os.environ, HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'config'),
                           XDG_CACHE_HOME=str(root / 'cache'), XDG_DATA_HOME=str(root / 'data'),
                           XDG_STATE_HOME=str(root / 'state'), XDG_RUNTIME_DIR=str(root / 'runtime'),
                           GTK_A11Y='none', NO_AT_BRIDGE='1', GSETTINGS_BACKEND='memory',
                           OX_THUMBNAIL_TRACE=str(root / 'trace.log'))
        for variable in ('WAYLAND_DISPLAY', 'GTK_USE_PORTAL'):
            environment.pop(variable, None)
        # Icons rather than Details, as a new window shows them at Large icons.
        settings = root / 'config' / 'winspace'
        settings.mkdir(mode=0o700)
        (settings / 'settings.json').write_text(json.dumps({'preferences': {'view': 'grid'}}) + '\n')
        (settings / 'settings.json').chmod(0o600)
        work = root / 'work'
        work.mkdir()
        photos = root / 'home' / 'Pictures' / 'Trip photos'
        photos.mkdir(parents=True)
        mixed = root / 'home' / 'Documents' / 'Mixed media'
        mixed.mkdir(parents=True)
        started = time.monotonic()
        make_photos(photos, arguments.photos, seed=1)
        (work / 'mixed').mkdir()
        make_mixed(mixed, 120, work / 'mixed')
        print(f'Made the folders in {time.monotonic() - started:.0f} s.', flush=True)
        screenshots = Path(arguments.screenshots) if arguments.screenshots else None
        results = []
        results += run_folder('photos', photos, root, arguments.binary, environment, screenshots,
                              arguments.on_screen)
        results += run_folder('mixed', mixed, root, arguments.binary, environment, screenshots,
                              arguments.on_screen)
        print_table(results)
        if arguments.scroll:
            scrolled = root / 'home' / 'Pictures' / 'New photos'
            scrolled.mkdir(parents=True)
            make_photos(scrolled, arguments.photos, seed=3)
            result = run_scroll(scrolled, root, arguments.binary, environment)
            results.append(result)
            print(f"| Fast scroll of a new folder | {result['scroll_ms']} ms | stalls of 34 ms or more: "
                  f"{result['stalls']} | longest: {result['longest_stall_ms']} ms | "
                  f"thumbnails made meanwhile: {result['made_while_scrolling']} |")
        if arguments.json:
            Path(arguments.json).write_text(json.dumps(results, indent=2) + '\n')
        if arguments.keep_trace:
            shutil.copy(root / 'trace.log', arguments.keep_trace)
        log = (root / 'stderr.log').read_text(errors='replace')
        bad = [line for line in log.splitlines() if 'CRITICAL' in line or 'panicked' in line]
        for line in bad[:20]:
            print('stderr: ' + line)
        return 1 if bad else 0
    finally:
        shutil.rmtree(root, ignore_errors=True)


def main() -> int:
    """Parse the command line and run inside a private session."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--binary', type=Path, default=DEFAULT_BINARY)
    parser.add_argument('--photos', type=int, default=2000)
    parser.add_argument('--on-screen', type=int, default=30,
                        help='how many tiles the 1400 x 950 window shows (6 columns of 5 rows at Large icons)')
    parser.add_argument('--json', help='write the results here as JSON')
    parser.add_argument('--scroll', action='store_true',
                        help='also scroll a new folder with Page Down and count main-loop stalls')
    parser.add_argument('--keep-trace', help="copy the app's trace here")
    parser.add_argument('--screenshots', help='save a picture of the screen after each visit in this folder')
    parser.add_argument('--user', default='nobody',
                        help='the unprivileged user the run uses when started as root')
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
               sys.executable, str(Path(__file__).resolve()), INNER, '--binary', str(arguments.binary.resolve()),
               '--photos', str(arguments.photos), '--on-screen', str(arguments.on_screen)]
    if arguments.scroll:
        command.append('--scroll')
    for option, value in (('--json', arguments.json), ('--keep-trace', arguments.keep_trace),
                          ('--screenshots', arguments.screenshots)):
        if value:
            command += [option, str(Path(value).resolve())]
    if os.geteuid() == 0:
        # A test of the real thing runs as a user, never as root.
        command = ['runuser', '-u', arguments.user, '--', *command]
        environment['TMPDIR'] = '/tmp'
    return subprocess.run(command, env=environment, check=False).returncode


if __name__ == '__main__':
    sys.exit(main())
