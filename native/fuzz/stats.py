#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Prints each target's executions over every run logged in logs/ (in
$OX_FUZZ_WORK when it is set): the finished runs' totals plus the running
one's latest count."""

import os
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Where run.sh keeps the logs: $OX_FUZZ_WORK, else here.
WORK = Path(os.environ.get('OX_FUZZ_WORK') or HERE)


def executions(log):
    total = 0
    latest = 0
    for line in log.read_text(errors='replace').splitlines():
        finished = re.match(r'stat::number_of_executed_units:\s+(\d+)', line)
        if finished:
            total += int(finished.group(1))
            latest = 0
            continue
        progress = re.match(r'#(\d+)\s', line)
        if progress:
            latest = int(progress.group(1))
    return total + latest


def main():
    for log in sorted((WORK / 'logs').glob('*.log')):
        print(f'{log.stem:18} {executions(log):>14,}')


if __name__ == '__main__':
    main()
