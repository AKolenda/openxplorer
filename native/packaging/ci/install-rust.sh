#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Installs the Rust toolchains of .github/workflows/native-distros.yml for the
# current user: 1.92.0, the minimum supported version that check.py lints and
# tests with (rust-version in native/Cargo.toml), and stable, which the Arch
# package builds with. rustup comes from the distribution where it has one
# (Arch), else from rustup.rs.
set -eu

if ! command -v rustup >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain none --no-modify-path
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup toolchain install 1.92.0 --profile minimal --component rustfmt --component clippy
rustup toolchain install stable --profile minimal
rustup default 1.92.0
