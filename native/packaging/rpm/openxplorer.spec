# SPDX-License-Identifier: AGPL-3.0-only
# The RPM of the native app for Fedora and openSUSE. It builds the preview
# (openxplorer-native) unless rpmbuild is given
#   --define 'app_id io.winspace.Development'
# for the stable app, which the release replacing the Python app ships as
# openxplorer. The sources are the two archives native/tools/source_archive.py
# --vendor writes, so the build never uses the network.
# native/packaging/README.md explains the dependency grouping.

%{!?app_id: %global app_id io.winspace.Development.Native}
%if "%{app_id}" == "io.winspace.Development"
%global package_name openxplorer
%global summary_text Explorer-style local and SMB file manager
%else
%global package_name openxplorer-native
%global summary_text Preview of the native OpenXplorer file manager
%endif

# Cargo's release profile has no debug information, so there is nothing for
# a debuginfo package to hold.
%global debug_package %{nil}

Name:           %{package_name}
Version:        2.0.0
Release:        1%{?dist}
Summary:        %{summary_text}
# The program is AGPL-3.0-only; the Rust crates compiled into it are MIT,
# Apache-2.0 or both, and their licences are installed beside it
# (rust-crates/INDEX.txt).
License:        AGPL-3.0-only AND MIT AND Apache-2.0
URL:            https://openxplorer.app
Source0:        openxplorer-%{version}.tar.gz
Source1:        openxplorer-%{version}-vendor.tar.gz

BuildRequires:  cargo >= 1.92
BuildRequires:  rust >= 1.92
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(sqlite3)
BuildRequires:  pkgconfig(libsoup-3.0)
BuildRequires:  /usr/bin/glib-compile-resources
BuildRequires:  python3
BuildRequires:  /usr/bin/desktop-file-validate
BuildRequires:  /usr/bin/appstreamcli

# The linked libraries are found by RPM from the program itself.
Requires:       hicolor-icon-theme
# GVfs: SMB shares, phones, the Recycle Bin and the drive list.
Recommends:     gvfs
Recommends:     gvfs-fuse
%if 0%{?suse_version}
Recommends:     gvfs-backends
%else
Recommends:     gvfs-smb
Recommends:     gvfs-mtp
%endif
# The Secret Service that remembers SMB passwords.
Recommends:     gnome-keyring
# xdg-mime, which makes OpenXplorer the default file manager on request.
Recommends:     xdg-utils
# Open in archive manager.
Recommends:     file-roller
%if "%{app_id}" == "io.winspace.Development"
# mount.cifs, which the persistent SMB mount helper's units run.
Recommends:     cifs-utils
%endif

%description
A file manager with the look of Windows 11 File Explorer, drawn with native
GTK 4 widgets: tabs, a resizable sidebar, SMB shares and phones through GVfs,
indexed filename search and ZIP browsing. Installing it changes no default
applications, mounts or user data.

%prep
%autosetup -n openxplorer-%{version} -a 1
# Cargo takes every crate from the vendored folder instead of crates.io.
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

%build
export OX_APP_ID=%{app_id}
cargo build --release --locked --offline --manifest-path native/Cargo.toml \
    --package ox-app --bin openxplorer-native \
    --package ox-core --bin openxplorer-mount-share

%install
# The shared install layout of every package format (native/tools/package_data.py).
python3 native/tools/package_data.py --app-id %{app_id} --layout fhs \
    --program native/target/release/openxplorer-native \
    --mount-helper native/target/release/openxplorer-mount-share --destdir %{buildroot}

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{app_id}.desktop
appstreamcli validate --no-net %{buildroot}%{_datadir}/metainfo/%{app_id}.metainfo.xml

%files
%{_bindir}/%{name}
%{_datadir}/applications/%{app_id}.desktop
%{_datadir}/metainfo/%{app_id}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg
%{_datadir}/dbus-1/services/%{app_id}.service
%{_datadir}/licenses/%{name}/
%if "%{app_id}" == "io.winspace.Development"
# The Python package's command names and the persistent SMB mount helper.
%{_bindir}/winspace
%{_bindir}/openxplorer-mount-share
%{_bindir}/winspace-mount-share
%endif

%changelog
* Mon Sep 28 2026 OpenXplorer contributors <openxplorer@users.noreply.github.com> - 2.0.0-1
- OpenXplorer 2.0.0: the native GTK 4 app replaces the Python app.

* Mon Sep 28 2026 OpenXplorer contributors <openxplorer@users.noreply.github.com> - 0.1.0-1
- First packaged native preview.
