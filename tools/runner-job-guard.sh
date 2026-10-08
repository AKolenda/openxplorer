#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Install this file root-owned at /usr/local/libexec/openxplorer-runner-job-guard.sh
# and set ACTIONS_RUNNER_HOOK_JOB_STARTED to that path in the runner's .env.
# Anyone with write access can push a branch whose workflow targets this
# runner. Refuse every job except checks.yml as committed on main, started by
# a push or a manual dispatch; a failing job-started hook fails the job before
# any of its steps run.
set -eu
expected="${GITHUB_REPOSITORY:?}/.github/workflows/checks.yml@refs/heads/main"
case "${GITHUB_EVENT_NAME:-}" in
    push|workflow_dispatch) ;;
    *) echo "Refusing ${GITHUB_EVENT_NAME:-unknown} event on the release runner." >&2; exit 1 ;;
esac
if [ "${GITHUB_WORKFLOW_REF:-}" != "$expected" ] || [ "${GITHUB_REF:-}" != refs/heads/main ]; then
    echo "Refusing ${GITHUB_WORKFLOW_REF:-unknown workflow}: only $expected may run here." >&2
    exit 1
fi
