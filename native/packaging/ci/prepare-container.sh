#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Prepares a distribution container for native/tools/check.py and the package
# build of .github/workflows/native-distros.yml; run it as root from the
# repository root. Usage: prepare-container.sh fedora|opensuse|arch|debian
#
# It installs, from the distribution itself: the C toolchain and the GTK 4,
# SQLite and libsoup 3 development files the app builds against; the private
# display and session bus check.py runs tests on (xvfb-run, Xvfb, xauth,
# dbus-run-session); GVfs with its SMB backend, PyGObject, GNOME Keyring and
# libsecret for the Recycle Bin, share and keyring tests; Node.js, which the
# location fixture test runs desktop/ui/app.js with; a font and an icon theme
# for GTK; the desktop validators; and the family's package tools.
# Rust itself comes from rustup (install-rust.sh), except the distribution
# compiler RPM builds use. Then it adds the unprivileged user "builder",
# because check.py must not run as root: root can read files whose
# permissions deny it, which the permission tests rely on, and makepkg
# refuses to run as root.
set -eu

family=${1:?usage: prepare-container.sh fedora|opensuse|arch|debian}

case "$family" in
    fedora)
        dnf -y install gcc pkgconf-pkg-config 'pkgconfig(gtk4)' 'pkgconfig(sqlite3)' \
            'pkgconfig(libsoup-3.0)' /usr/bin/glib-compile-resources python3 git-core \
            /usr/bin/xvfb-run /usr/bin/Xvfb /usr/bin/xauth /usr/bin/dbus-run-session \
            gvfs gvfs-smb python3-gobject gnome-keyring libsecret dejavu-sans-fonts \
            adwaita-icon-theme /usr/bin/node desktop-file-utils appstream rpm-build cargo rust \
            cpio
        ;;
    opensuse)
        # The container ships BusyBox's awk, which conflicts with the gawk
        # that desktop-file-utils and rpm-build need; --force-resolution lets
        # zypper replace it instead of stopping to ask.
        zypper --non-interactive install --force-resolution \
            gcc pkgconf-pkg-config 'pkgconfig(gtk4)' \
            'pkgconfig(sqlite3)' 'pkgconfig(libsoup-3.0)' glib2-tools python3 git-core curl \
            xvfb-run xorg-x11-server-Xvfb xauth dbus-1 gvfs gvfs-backends gvfs-backend-samba \
            python3-gobject typelib-1_0-Secret-1 gnome-keyring dejavu-fonts adwaita-icon-theme \
            /usr/bin/node desktop-file-utils AppStream rpm-build cargo rust cpio
        ;;
    arch)
        # rustup provides cargo for makepkg and the pinned compiler for check.py.
        pacman -Syu --noconfirm --needed base-devel git rustup gtk4 sqlite libsoup3 glib2 \
            python xorg-server-xvfb xorg-xauth dbus gvfs gvfs-smb python-gobject \
            gnome-keyring libsecret ttf-dejavu adwaita-icon-theme nodejs desktop-file-utils \
            appstream
        ;;
    debian)
        # The packages .github/workflows/native-checks.yml installs, and the
        # Debian package tools.
        apt-get update
        DEBIAN_FRONTEND=noninteractive apt-get install -y build-essential pkg-config curl \
            ca-certificates git python3 libgtk-4-dev libsqlite3-dev libsoup-3.0-dev xvfb \
            xauth dbus-x11 gvfs gvfs-backends python3-gi gir1.2-glib-2.0 gnome-keyring \
            gir1.2-secret-1 fonts-dejavu-core adwaita-icon-theme nodejs desktop-file-utils \
            appstream dpkg-dev
        ;;
    *)
        echo "Unknown distribution family: $family" >&2
        exit 2
        ;;
esac

useradd --create-home builder
chown -R builder: .
