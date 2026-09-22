#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Install this file root-owned at /usr/local/libexec/openxplorer-runner-service.
# LXC's overlaid /proc can prevent nested WebKit sandbox mounts. Give only
# this service a fresh proc mount, then drop privileges before runner code.
set -eu
[ "$(id -u)" -eq 0 ] || { echo 'Service namespace setup requires root.' >&2; exit 1; }
exec /usr/bin/unshare --mount --propagation private /bin/sh -eu -c '
    /usr/bin/mount -t proc -o nosuid,nodev,noexec proc /proc
    exec /usr/bin/setpriv --reuid=openxplorer-runner --regid=openxplorer-runner \
        --init-groups --no-new-privs /usr/bin/env -u XDG_RUNTIME_DIR \
        HOME=/home/openxplorer-runner USER=openxplorer-runner LOGNAME=openxplorer-runner \
        PATH=/usr/local/bin:/usr/bin:/bin /opt/openxplorer-runner/runsvc.sh
'
