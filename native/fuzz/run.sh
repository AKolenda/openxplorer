#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# run.sh TARGET SECONDS [libFuzzer options…]: fuzzes TARGET for SECONDS with
# the limits the fuzzing job uses (a 2 s hang, 1 GiB of memory), keeping
# new inputs in corpus/TARGET and failures in artifacts/TARGET, with the log
# in logs/TARGET.log. They are kept in $OX_FUZZ_WORK when it names a folder
# (outside the checkout, so a long run's files stay out of the tree the
# checks copy), else here.
set -eu
target=$1
seconds=$2
shift 2
here=$(cd "$(dirname "$0")" && pwd)
cd "${OX_FUZZ_WORK:-$here}"
mkdir -p "corpus/$target" "artifacts/$target" logs
exec "$here/target/x86_64-unknown-linux-gnu/release/$target" \
    -timeout=2 -rss_limit_mb=1024 -malloc_limit_mb=1024 -max_len=65536 \
    -max_total_time="$seconds" -print_final_stats=1 \
    -artifact_prefix="artifacts/$target/" "$@" "corpus/$target" \
    >>"logs/$target.log" 2>&1
