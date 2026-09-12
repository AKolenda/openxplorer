#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Public entry point; legacy storage/application IDs are preserved."""
from pathlib import Path
import runpy
if __name__ == '__main__':
    runpy.run_path(str(Path(__file__).resolve().with_name('winspace.py')), run_name='__main__')
