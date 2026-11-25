#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Measures a folder whose picture peeks out of it, in a screenshot.

Windows 11 draws a folder with pictures at Large icons as a back, the first
picture, and a front lower than a plain folder's. Given a screenshot and the
box of one such folder's tile, this prints the folder's outline, the picture's
box and the front's top on its left and right, in pixels and as fractions of
the folder's width or height, so the app's drawing can be compared with the
Windows reference screenshots (reference/ref-12-view-large.png of the status
branch) by numbers rather than by eye:

    measure-folder-previews.py ref-12-view-large.png 285,137,387,232
    measure-folder-previews.py capture-large.png 455,150,545,245

The tile's box should hold the folder and the dark background around it, not
its name. Needs Pillow.
"""

import argparse
import sys
from pathlib import Path

from PIL import Image


def is_background(pixel: tuple[int, int, int]) -> bool:
    """The dark view behind the tiles: gray, darker than 70."""
    red, green, blue = pixel
    return max(red, green, blue) < 70 and max(red, green, blue) - min(red, green, blue) < 12


def is_folder_yellow(pixel: tuple[int, int, int]) -> bool:
    """A folder's yellow or orange, Windows' or the app's, also darkened along
    its outline: red over green over blue, far more red than blue."""
    red, green, blue = pixel
    return red >= green >= blue and red - blue > 60 and red > 120


def is_front(pixel: tuple[int, int, int]) -> bool:
    """A folder front's light yellow, Windows' gradient or the app's flat one."""
    red, green, blue = pixel
    return red > 240 and 195 < green < 240 and blue < 190


def measure(image: Image.Image, box: tuple[int, int, int, int]) -> dict[str, float]:
    """The folder's measures inside box (left, top, right, bottom)."""
    left, top, right, bottom = box
    pixel = image.getpixel
    folder = [(x, y) for y in range(top, bottom) for x in range(left, right) if is_folder_yellow(pixel((x, y)))]
    if not folder:
        raise SystemExit(f'No folder in {box}')
    folder_left, folder_right = min(x for x, _ in folder), max(x for x, _ in folder) + 1
    folder_top, folder_bottom = min(y for _, y in folder), max(y for _, y in folder) + 1
    width, height = folder_right - folder_left, folder_bottom - folder_top

    def front_top(x: int) -> int:
        # From above the front's darker bottom edge, up to its top.
        y = folder_bottom - max(3, round(height * 0.08))
        while y > folder_top and is_front(pixel((x, y))):
            y -= 1
        return y + 1

    left_front, right_front = (front_top(folder_left + round(width * fraction)) for fraction in (0.15, 0.80))
    # The picture: what is neither the folder's yellow nor the background,
    # inside the folder's outline and above the front, taken as the rows and
    # then the columns at least half of which it fills, so the smoothed
    # outline's odd pixels do not count.
    def is_picture(x: int, y: int) -> bool:
        return not is_folder_yellow(pixel((x, y))) and not is_background(pixel((x, y)))

    columns = range(folder_left, folder_right)
    rows = [y for y in range(folder_top, max(left_front, right_front))
            if sum(is_picture(x, y) for x in columns) * 2 >= width]
    if not rows:
        raise SystemExit(f'No picture in the folder at {box}')
    picture_top, picture_bottom = rows[0], rows[-1] + 1
    filled = [x for x in columns if sum(is_picture(x, y) for y in rows) * 2 >= len(rows)]
    picture_left, picture_right = filled[0], filled[-1] + 1
    found = {
        'folder left': folder_left, 'folder top': folder_top,
        'folder width': width, 'folder height': height,
        'picture left': picture_left, 'picture right': picture_right,
        'picture top': picture_top, 'picture bottom': picture_bottom,
        'front top at 15%': left_front,
        'front top at 80%': right_front,
    }
    fractions = {
        'picture left': (picture_left - folder_left) / width,
        'picture right': (picture_right - folder_left) / width,
        'picture top': (picture_top - folder_top) / height,
        'picture bottom': (picture_bottom - folder_top) / height,
        'front top at 15%': (found['front top at 15%'] - folder_top) / height,
        'front top at 80%': (found['front top at 80%'] - folder_top) / height,
    }
    return found | {f'{name} (fraction)': round(value, 3) for name, value in fractions.items()}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('screenshot', type=Path)
    parser.add_argument('box', help='left,top,right,bottom of the tile around the folder')
    arguments = parser.parse_args()
    box = tuple(int(value) for value in arguments.box.split(','))
    if len(box) != 4:
        parser.error('the box is four numbers: left,top,right,bottom')
    image = Image.open(arguments.screenshot).convert('RGB')
    for name, value in measure(image, box).items():
        print(f'{name}: {value}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
