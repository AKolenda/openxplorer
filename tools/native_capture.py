# SPDX-License-Identifier: AGPL-3.0-only
"""Take pictures of the native app in an isolated session with fictional files.

Shared by tools/capture-native-tour.py (the website's click-through tour) and
tools/capture-screenshots.py (the website's screenshots). Each picture comes
from the app's snapshot hook (OPENXPLORER_SNAPSHOT and the variables beside it,
documented in native/crates/ox-app/src/snapshot.rs): the real program draws its
real window, which is saved as a PNG.

Isolation follows docs/PRIVACY.md and native/README.md:

- bubblewrap (bwrap) hides /home, /media, /mnt and /run/user behind empty
  folders, so the app can neither read the user's files nor reach the user's
  session bus, mounts or phones. It also gives the app no network and hides
  the system bus and Avahi's socket (Avahi answers over them with the servers
  it has seen on the LAN), so the Network page discovers nothing, and its own
  process namespace,
  so no helper outlives the picture. The fictional demo tree is mounted as
  /home/demo, the home folder the published examples use.
- The app runs on its own X display (xvfb-run) and D-Bus session
  (dbus-run-session), never on the live desktop. DISPLAY, WAYLAND_DISPLAY and
  DBUS_SESSION_BUS_ADDRESS are not passed on; the environment is built from
  scratch.
- Settings, caches and the keyring start empty in the demo home for every
  picture, and everything is deleted afterwards.

The app is built with the stable application ID (OX_APP_ID), so the pictures
show what the released packages show.
"""
from __future__ import annotations

from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import zlib

ROOT = Path(__file__).resolve().parents[1]
NATIVE = ROOT / 'native'
# The stable application ID: the released packages' name for the app.
STABLE_APP_ID = 'io.winspace.Development'
DEMO_HOME = '/home/demo'
# The size of every picture, as the website shows it.
WINDOW_SIZE = '1440x900'
# One fixed time for every file, so listings do not change between captures:
# 14 September 2026, 09:30 UTC, and older items a few days earlier.
DEMO_TIME = 1789378200
DAY = 86400
# Where the desktop's installed applications are listed.
APPLICATION_FOLDERS = ('/usr/share/applications', '/usr/local/share/applications',
                       '/var/lib/flatpak/exports/share/applications')
# The system bus and Avahi's socket. They stay reachable without a network,
# and Avahi answers over them with the servers it has already seen on the LAN.
SYSTEM_SERVICE_FOLDERS = ('/run/dbus', '/run/avahi-daemon')
# A snapshot hook run takes a few seconds; one that hangs is stopped.
CAPTURE_TIMEOUT = 120


class CaptureError(Exception):
    """A picture could not be taken; the message says why."""


@dataclass(frozen=True)
class Picture:
    """What one run of the snapshot hook shows: the hook's variables."""

    start: str = DEMO_HOME
    theme: str = 'light'
    view: str = ''
    settings: str = ''
    search: str = ''
    scene: tuple[str, ...] = ()
    # Whether the settings hold the mapped sample share \\studio-nas\Projects.
    with_share: bool = False

    def variables(self) -> dict[str, str]:
        """Return the snapshot hook's variables for this picture."""
        variables = {
            'OPENXPLORER_START': self.start,
            'OPENXPLORER_THEME': self.theme,
            'OPENXPLORER_SIZE': WINDOW_SIZE,
            'OPENXPLORER_VIEW': self.view,
            'OPENXPLORER_SETTINGS': self.settings,
            'OPENXPLORER_SEARCH': self.search,
            'OPENXPLORER_SCENE': ';'.join(self.scene),
        }
        return {name: value for name, value in variables.items() if value}


# The fictional demo tree: folder -> files with their sizes in bytes and age
# in days. Names follow docs/PRIVACY.md: nothing here names a person, a real
# server or a customer project.
DEMO_TREE: dict[str, dict[str, tuple[int, int]]] = {
    'Desktop': {},
    'Documents': {
        'Meeting notes.md': (2_310, 1),
        'Quarterly report.odt': (48_640, 3),
        'Project brief.pdf': (182_400, 6),
        'Expenses 2026.ods': (21_504, 9),
    },
    'Documents/Launch planning': {
        'Timeline.odt': (36_864, 2),
        'Budget.ods': (19_456, 4),
        'Venue options.pdf': (412_160, 5),
        'Guest list.csv': (1_920, 8),
    },
    'Documents/Invoices': {
        'Invoice 2026-014.pdf': (88_064, 12),
        'Invoice 2026-015.pdf': (90_112, 7),
    },
    'Downloads': {
        'Course notes.zip': (0, 2),
        'Wallpaper pack.zip': (0, 10),
        'Setup guide.pdf': (264_192, 15),
    },
    'Music': {},
    'Pictures': {},
    'Pictures/Holiday 2025': {},
    'Pictures/Screenshots': {},
    'Videos': {},
}
# Pictures drawn as plain colour gradients: (name, start colour, end colour).
DEMO_IMAGES = (
    ('Pictures/Beach.png', (52, 136, 196), (240, 214, 160)),
    ('Pictures/Forest.png', (34, 98, 60), (160, 200, 120)),
    ('Pictures/Sunset.png', (220, 96, 60), (60, 40, 110)),
    ('Pictures/Holiday 2025/Harbour.png', (40, 90, 160), (200, 220, 235)),
)
# The settings every picture starts with: Launch planning pinned to Quick
# access. Pictures that ask for it also have a mapped share on the fictional
# studio-nas, which the sidebar and the Network page show offline: the app
# has no network.
DEMO_PIN = {'uri': 'file:///home/demo/Documents/Launch%20planning', 'label': 'Launch planning'}
DEMO_SHARE = {'uri': 'smb://studio-nas/Projects', 'label': 'Projects'}
# Previous versions of the files in Launch planning, as a NAS exposes them: a
# .snapshot folder beside them, with Windows-style @GMT names.
DEMO_SNAPSHOTS = ('@GMT-2026.09.07-18.00.00', '@GMT-2026.09.12-18.00.00')


def write_demo_tree(home: Path, with_share: bool) -> None:
    """Create the fictional demo files and settings under home, with fixed dates."""
    for folder, files in DEMO_TREE.items():
        directory = home / folder
        directory.mkdir(parents=True, exist_ok=True)
        for name, (size, age) in files.items():
            path = directory / name
            if path.suffix == '.zip':
                write_zip(path)
            else:
                path.write_bytes(b'Fictional sample file.\n'.ljust(size, b' '))
            set_time(path, age)
    for name, start, end in DEMO_IMAGES:
        path = home / name
        path.write_bytes(gradient_png(320, 200, start, end))
        set_time(path, 20)
    for age, snapshot in zip((18, 12), DEMO_SNAPSHOTS):
        copy = home / 'Documents' / 'Launch planning' / '.snapshot' / snapshot
        copy.mkdir(parents=True)
        for name, (size, _age) in DEMO_TREE['Documents/Launch planning'].items():
            (copy / name).write_bytes(b'Fictional earlier version.\n'.ljust(size // 2, b' '))
            set_time(copy / name, age)
        set_time(copy, age)
    settings = home / '.config' / 'winspace' / 'settings.json'
    settings.parent.mkdir(parents=True)
    data = {'version': 2, 'pins': [DEMO_PIN], 'shares': [DEMO_SHARE] if with_share else []}
    settings.write_text(json.dumps(data, indent=2) + '\n', encoding='utf-8')
    # Folders last, so writing their files does not move their dates.
    for folder in sorted(DEMO_TREE, key=len, reverse=True):
        set_time(home / folder, 1)


def write_zip(path: Path) -> None:
    """Write a small ZIP archive of two fictional text files."""
    import zipfile  # Only the demo archives need it.
    with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for name in ('Read me.txt', 'Chapter 1.txt'):
            info = zipfile.ZipInfo(name, date_time=(2026, 9, 1, 9, 0, 0))
            archive.writestr(info, 'Fictional sample text.\n' * 40)


def set_time(path: Path, age_days: int) -> None:
    """Give path the demo time, age_days earlier."""
    moment = DEMO_TIME - age_days * DAY
    os.utime(path, (moment, moment))


def gradient_png(width: int, height: int, start: tuple[int, int, int],
                 end: tuple[int, int, int]) -> bytes:
    """Return an RGB PNG that fades from start at the top to end at the bottom."""
    rows = bytearray()
    for y in range(height):
        mix = y / (height - 1)
        colour = bytes(round(a + (b - a) * mix) for a, b in zip(start, end))
        rows += b'\x00' + colour * width
    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body))
    header = struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header)
            + chunk(b'IDAT', zlib.compress(bytes(rows), 9)) + chunk(b'IEND', b''))


def build_app() -> Path:
    """Build the release program with the stable application ID and return its path."""
    for tool in ('cargo', 'bwrap', 'xvfb-run', 'dbus-run-session'):
        if shutil.which(tool) is None:
            raise CaptureError(f'{tool} is required to capture the native app.')
    environment = dict(os.environ, OX_APP_ID=STABLE_APP_ID)
    subprocess.run(['cargo', 'build', '--locked', '--release', '--bin', 'openxplorer-native'],
                   cwd=NATIVE, env=environment, check=True)
    target = Path(os.environ.get('CARGO_TARGET_DIR', NATIVE / 'target'))
    program = target / 'release' / 'openxplorer-native'
    if not program.is_file():
        raise CaptureError(f'Cargo did not build {program}.')
    return program


def locale_name() -> str:
    """Return en_US.UTF-8 when the system has it, else C.UTF-8."""
    available = subprocess.run(['locale', '-a'], capture_output=True, text=True).stdout.split()
    return 'en_US.UTF-8' if 'en_US.utf8' in available else 'C.UTF-8'


@contextmanager
def demo_session() -> Iterator[Path]:
    """Yield a temporary folder holding the demo home; delete it afterwards."""
    with tempfile.TemporaryDirectory(prefix='openxplorer-capture-') as temporary:
        yield Path(temporary)


def capture(program: Path, workspace: Path, picture: Picture, png: Path,
            with_hotspots: bool = False) -> dict | None:
    """Save picture as png with a fresh demo home; return its hotspots when asked.

    Every picture starts from a new copy of the demo tree and empty settings,
    so one picture's actions cannot change the next.
    """
    home = workspace / 'home'
    runtime = workspace / 'runtime'
    for directory in (home, runtime):
        shutil.rmtree(directory, ignore_errors=True)
    write_demo_tree(home, picture.with_share)
    runtime.mkdir(mode=0o700)
    output = workspace / 'output'
    output.mkdir(exist_ok=True)
    picture_file = output / 'picture.png'
    hotspot_file = output / 'hotspots.json'
    for stale in (picture_file, hotspot_file):
        stale.unlink(missing_ok=True)
    variables = picture.variables()
    variables['OPENXPLORER_SNAPSHOT'] = str(picture_file)
    if with_hotspots:
        variables['OPENXPLORER_HOTSPOTS'] = str(hotspot_file)
    command = isolated_command(program, home, runtime, variables)
    language = locale_name()
    environment = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'LANG': language,
                   'LC_ALL': language, 'TZ': 'UTC'}
    try:
        result = subprocess.run(command, env=environment, capture_output=True, text=True,
                                timeout=CAPTURE_TIMEOUT)
    except subprocess.TimeoutExpired as error:
        raise CaptureError(f'The app did not save {png.name} in time.') from error
    if result.returncode != 0 or not picture_file.is_file():
        raise CaptureError(f'The app could not save {png.name}:\n{result.stderr[-2000:]}')
    png.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(picture_file, png)
    if not with_hotspots:
        return None
    hotspots: dict = json.loads(hotspot_file.read_text(encoding='utf-8'))
    return hotspots


def isolated_command(program: Path, home: Path, runtime: Path,
                     variables: Mapping[str, str]) -> list[str]:
    """Return the command that runs program isolated, as the module docstring says."""
    runtime_directory = f'/run/user/{os.getuid()}'
    command = ['bwrap', '--dev-bind', '/', '/',
               '--tmpfs', '/home', '--tmpfs', '/media', '--tmpfs', '/mnt', '--tmpfs', '/run/user',
               '--bind', str(home), DEMO_HOME,
               '--bind', str(runtime), runtime_directory,
               # No network, so nothing is discovered or reached on the LAN, and
               # a process namespace, so every helper the app starts ends with it.
               '--unshare-net', '--unshare-pid', '--die-with-parent']
    session = {
        'HOME': DEMO_HOME, 'USER': 'demo', 'LOGNAME': 'demo',
        'XDG_RUNTIME_DIR': runtime_directory,
        'XDG_CONFIG_HOME': f'{DEMO_HOME}/.config', 'XDG_CACHE_HOME': f'{DEMO_HOME}/.cache',
        'XDG_DATA_HOME': f'{DEMO_HOME}/.local/share',
        'XDG_STATE_HOME': f'{DEMO_HOME}/.local/state',
        # No FUSE mounts, no phones or remote volumes, no accessibility bus
        # and no saved desktop settings.
        'GVFS_DISABLE_FUSE': '1', 'GVFS_REMOTE_VOLUME_MONITOR_IGNORE': '1',
        'NO_AT_BRIDGE': '1', 'GSETTINGS_BACKEND': 'memory',
        'GDK_BACKEND': 'x11', 'XDG_DATA_DIRS': '/usr/local/share:/usr/share',
    }
    # No installed applications, so menus do not list this computer's apps
    # and the pictures are the same everywhere.
    for service in SYSTEM_SERVICE_FOLDERS:
        if Path(service).is_dir():
            command += ['--tmpfs', service]
    for applications in APPLICATION_FOLDERS:
        if Path(applications).is_dir():
            command += ['--tmpfs', applications]
    for name, value in {**session, **variables}.items():
        command += ['--setenv', name, value]
    return command + ['dbus-run-session', '--', 'xvfb-run', '--auto-servernum',
                      # Taller than the window, so a menu that opens near its
                      # bottom fits on the screen, as on a real desktop.
                      '--server-args=-screen 0 1600x1400x24', str(program)]
