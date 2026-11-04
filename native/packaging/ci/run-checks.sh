#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Runs the native check driver as the unprivileged user, with the rustup
# toolchain install-rust.sh set up. OX_REQUIRE_KEYRING makes the keyring
# tests fail instead of skipping when GNOME Keyring or libsecret is missing,
# as in .github/workflows/native-checks.yml.
#
# OX_TEST_TRUSTED_BASE names the folder prepare-container.sh made for the
# fixtures of the tests about which symbolic links an administrator
# deletion may follow. In a container job the checkout is a host mount at
# /__w, on another device, so those tests cannot build a fixture in it;
# check.py makes a run where they say so a failure rather than a quiet
# pass, which is why this has to be set here.
set -eu

export PATH="$HOME/.cargo/bin:$PATH"
export OX_REQUIRE_KEYRING=1
export OX_TEST_TRUSTED_BASE="$HOME/ox-fixtures"
python3 native/tools/check.py
