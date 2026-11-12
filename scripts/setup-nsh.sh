#!/usr/bin/env bash
# Resolve the NSH installation, prove it agrees with the crates the app builds
# against, and install the platform runtimes a build is about to need.
#
# Three things happen here because they are three ways the same mistake shows up:
# a machine whose `nsh` is not the NSH the Cargo.lock pins. `nsh package` compiles
# the app's crate against its own runtime and copies that runtime's classes into
# the result, so a crate at 0.5 packaged by a CLI at 0.4 produces an APK whose
# libnsh.so and classes.dex disagree — which no build step can see, and a phone
# answers by crashing at startup, after the release is public. Web fails the same
# way and later still: the page it ships is a different NSH than the one the
# native server half was built beside.
#
# The version comparison is major.minor. A patch release of the CLI is expected
# to stay compatible with the same crate series, and demanding an exact match
# would turn every NSH bugfix into a blocked app build.
#
# NSH 0.5 stopped shipping every platform's runtime inside its release archive;
# another platform's runtime is a component the CLI downloads the first time a
# command needs it. Asking here rather than letting `nsh package` fetch it keeps
# the failure legible: a runner with no network stops on this line instead of
# half-building an artifact from a runtime that never arrived.
#
# Usage: setup-nsh.sh [component...]   e.g. setup-nsh.sh android web
#
# Prints the NSH installation directory, and nothing else on stdout, so it can be
# used as `nsh_home=$(scripts/setup-nsh.sh android)`. `nsh` finds its runtimes next
# to its own binary, so the directory is the caller's link to them: assuming
# `$HOME/.nsh` would break any install placed elsewhere with NSH_INSTALL_DIR.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

die() {
    echo "error: $*" >&2
    exit 1
}

# `nsh.exe`, because the Windows installer writes that name and a job's bash does
# not resolve a bare `nsh` to it. Both spellings are tried rather than one, so the
# script runs the same way on a laptop and on every runner.
nsh_bin="$(command -v nsh || command -v nsh.exe || true)"
[[ -n "$nsh_bin" ]] ||
    die "the nsh CLI is not installed: curl -fsSL https://nsh.dev/install.sh | sh"

# The link is followed one hop at a time because `readlink -f` is a GNU extension
# and macOS's is not, and a resolution that quietly failed would hand back the
# directory holding the symlink — a path that exists and holds no runtime.
nsh_path="$nsh_bin"
while [[ -L $nsh_path ]]; do
    target="$(/bin/ls -l -- "$nsh_path")"
    target="${target##* -> }"
    [[ $target == /* ]] || target="$(dirname -- "$nsh_path")/$target"
    nsh_path="$target"
done
nsh_home="$(cd -- "$(dirname -- "$nsh_path")/.." && pwd)"
[[ -d "$nsh_home/runtime" ]] ||
    die "$nsh_home is not an NSH installation (no runtime directory); point PATH at
the nsh from https://nsh.dev/install.sh, not a standalone binary"

command -v jq >/dev/null || die "jq is not installed; it reads the crate requirements"

nsh_cli="$("$nsh_bin" --version | awk '{print $2}')"
for crate in nsh-core nsh-base; do
    req="$(cargo metadata --manifest-path "$root/Cargo.toml" --locked --no-deps \
        --format-version 1 |
        jq -er --arg crate "$crate" '
            .packages[] | select(.name == "nsh-app-kitty") |
            .dependencies[] | select(.name == $crate) | .req' |
        tr -d '^~=')"
    [[ "${req%.*}" == "${nsh_cli%.*}" ]] ||
        die "packages/nsh-app builds against NSH $req ($crate) but the nsh on PATH is
$nsh_cli ($nsh_bin). Package with the matching CLI, or move the crate: an artifact
whose runtime and Rust half are different NSH releases fails on the device."
done

for component in "$@"; do
    "$nsh_bin" component add "$component" >/dev/null ||
        die "nsh could not install its $component runtime (nsh component add $component)"
done

printf '%s\n' "$nsh_home"
