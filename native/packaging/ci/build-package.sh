#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Builds the preview package of one format, verifies it and leaves it in
# dist/native/ for .github/workflows/native-distros.yml to publish. Run it as
# the unprivileged user from the repository root.
# Usage: build-package.sh deb|rpm|arch
set -eu

format=${1:?usage: build-package.sh deb|rpm|arch}
app_id=io.winspace.Development.Native
output=dist/native
version=$(python3 - <<'EOF'
import tomllib
with open('native/Cargo.toml', 'rb') as manifest:
    print(tomllib.load(manifest)['workspace']['package']['version'])
EOF
)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$output" "$work/tree"

# Checks the files of the package unpacked into $work/tree.
verify_tree() {
    python3 native/tools/verify_layout.py --app-id "$app_id" --layout fhs \
        --version "$version" "$work/tree"
}

case "$format" in
    deb)
        export PATH="$HOME/.cargo/bin:$PATH"
        package=$(python3 native/tools/build_deb.py --app-id "$app_id" --output-directory "$output")
        python3 native/tools/verify_deb.py "$package"
        ;;
    rpm)
        # The distribution's own Rust builds the RPM, as its build service
        # would; rustup's only downloads the vendored crates.
        PATH="$HOME/.cargo/bin:$PATH" python3 native/tools/source_archive.py --vendor \
            --output-directory "$HOME/rpmbuild/SOURCES"
        rpmbuild -bb --define "app_id $app_id" native/packaging/rpm/openxplorer.spec
        package=$(find "$HOME/rpmbuild/RPMS" -name 'openxplorer-native-[0-9]*.rpm' | head -n 1)
        cp "$package" "$output/"
        rpm2cpio "$package" | (cd "$work/tree" && cpio -idm --quiet)
        verify_tree
        ;;
    arch)
        python3 native/tools/source_archive.py --output-directory "$work"
        cp native/packaging/arch/PKGBUILD "$work/"
        (cd "$work" && _app_id=$app_id makepkg --noconfirm)
        package=$(find "$work" -maxdepth 1 -name 'openxplorer-native-[0-9]*.pkg.tar.zst' | head -n 1)
        cp "$package" "$output/"
        tar --zstd -x -f "$package" -C "$work/tree"
        verify_tree
        ;;
    *)
        echo "Unknown package format: $format" >&2
        exit 2
        ;;
esac
