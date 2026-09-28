#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Runs the native check driver as the unprivileged user, with the rustup
# toolchain install-rust.sh set up. OX_REQUIRE_KEYRING makes the keyring
# tests fail instead of skipping when GNOME Keyring or libsecret is missing,
# as in .github/workflows/native-checks.yml.
set -eu

export PATH="$HOME/.cargo/bin:$PATH"
export OX_REQUIRE_KEYRING=1
python3 native/tools/check.py
