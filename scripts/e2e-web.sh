#!/usr/bin/env bash
# Run scripts/e2e-web.mjs against a real server, on a scratch profile.
#
# The server and the browser have to be started by one script, because the server
# owns the listener the page calls and the profile the page writes to: start it,
# wait for it, run the browser, kill it. Nothing outside this script sees either.
#
# The scratch KITTY_PROFILE_DIR is the reason to run it this way rather than
# against a server someone already left running: the test writes a subscription
# through POST /api/profile, and without it that write lands in the real profile of
# whoever's machine ran the check. The headless suite takes the same precaution for
# the same reason.
#
# Usage: scripts/e2e-web.sh [--site DIR]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
site="$root/target/e2e-site"
build=true
if [[ "${1:-}" == "--site" ]]; then
    site="$2"
    build=false
    shift 2
fi
port="${KITTY_E2E_PORT:-8244}"
cdp_port="${CDP_PORT:-9351}"
profile="$root/target/e2e-profile"

step() { printf '\033[36m==>\033[0m %s\n' "$*"; }
die() {
    echo "error: $*" >&2
    exit 1
}

command -v node >/dev/null ||
    die "node is required; it speaks the DevTools protocol itself, so there is nothing to install"
chrome="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
[[ -x $chrome ]] || die "no Chrome at $chrome (set CHROME)"

# --- the site, and the process that serves it --------------------------------

if $build; then
    step "nsh package --target web"
    mkdir -p "$site"
    (cd "$root/packages/nsh-app" && nsh package --target web -o "$site")
fi
# The packaged directory is named after the app, so find it rather than hard-coding
# "Kitty Pro". A rename would otherwise leave this passing a directory that does not
# exist, and the server answers that by serving no page while still binding the
# port — which reads like a browser failure rather than a wrong path.
web_dir=""
for candidate in "$site"/*; do
    if [[ -f "$candidate/index.html" ]]; then
        web_dir="$candidate"
        break
    fi
done
[[ -n "$web_dir" ]] || die "no packaged site under $site (run it without --site)"

server="$root/target/debug/kitty-pro-web-server"
if [[ ! -x $server ]]; then
    step "cargo build -p kitty-server"
    (cd "$root" && cargo build -p kitty-server >/dev/null)
fi

mkdir -p "$profile"
export KITTY_PROFILE_DIR="$profile"
export KITTY_WEB_DIR="$web_dir"
log="$root/target/e2e-server.log"

# On any exit, including an assertion failing inside the browser test.
cleanup() {
    if [[ -n "${server_pid:-}" ]]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT

step "server on 127.0.0.1:$port (profile: $profile)"
"$server" --port "$port" >"$log" 2>&1 &
server_pid=$!
answered=false
# Up to thirty seconds for a debug server to bind. The probe is quiet because the
# first attempts are refused by design — the process has only just been forked — and
# a curl error per attempt reads like a fault rather than a poll.
for _ in $(seq 1 60); do
    if curl -fsS -o /dev/null "http://127.0.0.1:$port/" 2>/dev/null; then
        answered=true
        break
    fi
    kill -0 "$server_pid" 2>/dev/null || die "the server exited: $(head -3 "$log")"
    sleep 0.5
done
if ! $answered; then
    die "the server never answered on $port; its log:
$(tail -10 "$log")"
fi

# --- the browser -------------------------------------------------------------

step "chrome"
if ! E2E_URL="http://127.0.0.1:$port/" CDP_PORT="$cdp_port" \
    node "$root/scripts/e2e-web.mjs"; then
    echo
    echo "server log:"
    tail -12 "$log" || true
    die "the browser did not drive the server"
fi

# What the page wrote has to be on disk, not merely acknowledged over HTTP: that is
# the difference between the transport answering and the transport reaching the
# same store the desktop app reads.
if [[ -f "$profile/profile.json" ]] && grep -q "E2E 订阅" "$profile/profile.json"; then
    step "profile.json holds what the page wrote"
else
    die "profile.json does not contain the subscription the page saved"
fi
