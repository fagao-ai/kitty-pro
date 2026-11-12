#!/usr/bin/env bash
# Stamp the workspace with the version a release tag names.
#
# Every shell reports `CARGO_PKG_VERSION` to the in-app update check, which
# compares it against the newest published release. One tag is one release of one
# product, so every package takes the tag's version rather than the release job
# picking a winner per artifact: the desktop job used to stamp only its own crate,
# and the version the check actually answers with is read from `api`, not from the
# shell that displays it, so even a correctly stamped desktop kept reporting 0.1.0.
#
# It also stamps `nsh.json`, which is what NSH puts in a bundle's own version
# fields (`CFBundleShortVersionString`/`CFBundleVersion` on macOS, the APK's
# `versionName`/`versionCode`). Nothing else writes that file, so a release built
# without this shows whatever version the last hand edit left there — Finder said
# 0.1.0 for a 1.0.13 download.
#
# Usage: set-package-version.sh <version>
#        set-package-version.sh --build-number <version>
#                            Print the integer a version implies, and exit. This is
#                            the one definition of that mapping, shared with
#                            `scripts/build-android.sh`, because the manifest NSH
#                            stamps and the APK the script assembles must agree about
#                            which build is newer.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$root/Cargo.toml"
nsh_manifest="$root/packages/nsh-app/nsh.json"

# The integer a version implies: major*1000000 + minor*1000 + patch.
#
# The weights keep it monotonic across every final release the grammar below
# accepts, including the boundary a naive scheme gets wrong (1.0.99 is 1000099,
# 1.1.0 is 1010000). Android refuses to install an update whose versionCode is not
# higher, so a mapping that stalled would strand a phone on the release it first
# installed, silently and forever.
#
# A pre-release is the one case it cannot order: `1.0.0-alpha.1` and
# `1.0.0-beta.1` are the same build number, because an integer has nowhere to put
# the tag. Two prerelease APKs of one version would then be uninstallable over each
# other. `KITTY_VERSION_CODE` is the answer — the release that needs it is a
# deliberate act, and this script cannot know which of the two is newer.
build_number_of() {
    local version="$1" version_number parts part
    version_number="${version%%-*}"
    version_number="${version_number%%+*}"
    IFS=. read -r -a parts <<<"$version_number"
    # Exactly three, so a four-part version cannot silently lose its last number
    # and collide with the release above it.
    if (( ${#parts[@]} != 3 )); then
        echo "version $version is not the three-number version a build number is derived from" >&2
        return 1
    fi
    for part in "${parts[@]}"; do
        [[ "$part" =~ ^[0-9]{1,3}$ ]] ||
            {
                echo "version $version does not fit the 1.2.3 shape a build number is derived from" >&2
                return 1
            }
    done
    printf '%s' "$((10#${parts[0]:-0} * 1000000 + 10#${parts[1]:-0} * 1000 + 10#${parts[2]:-0}))"
}

if [[ "${1:-}" == "--build-number" ]]; then
    version="${2:-}"
    [[ -n "$version" ]] || { echo "usage: $0 --build-number <version>" >&2; exit 2; }
    build_number_of "$version"
    echo
    exit 0
fi

version="${1:-}"

if [[ -z "$version" ]]; then
    echo "usage: $0 <version> | $0 --build-number <version>" >&2
    exit 2
fi

# The same grammar the release workflow accepts for a tag, and the same one
# `proxy_core::ReleaseVersion` can order.
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "unsupported package version $version" >&2
    exit 1
fi

temporary="${manifest}.version.tmp"
trap 'rm -f "$temporary"' EXIT

# The one line to rewrite: `version` inside `[workspace.package]`, which every
# member inherits through `version.workspace = true`. Scoped by the table header
# so a dependency's own `version = ` line can never be the one that moves.
awk -v version="$version" '
    /^\[/ { in_workspace_package = ($0 ~ /^\[workspace\.package\][[:space:]]*$/) }
    in_workspace_package && /^version[[:space:]]*=/ {
        print "version = \"" version "\""
        replaced = 1
        next
    }
    { print }
    END {
        if (!replaced) {
            print "no [workspace.package] version line found" > "/dev/stderr"
            exit 1
        }
    }
' "$manifest" >"$temporary"
mv "$temporary" "$manifest"
trap - EXIT

# NSH's own manifest, which is what a bundle's version fields come from. The
# desktop job passes `--app-version` but no `--build`, so `build` here is what
# lands in `CFBundleVersion`; the Android script overrides both explicitly.
# Stamped from the same version by the same arithmetic, so the two manifests
# cannot disagree about which build is newer.
build_number="$(build_number_of "$version")"
jq --arg version "$version" --argjson build "$build_number" \
    '.version = $version | .build = $build' "$nsh_manifest" >"${nsh_manifest}.tmp"
mv "${nsh_manifest}.tmp" "$nsh_manifest"

# Cargo.lock records the workspace members' versions. `--workspace` restricts it to
# the members: a release job must not move a registry dependency while it is moving
# a version number, and this is what keeps the lock in step with the manifest so the
# `--locked` check below - and every later `--locked` build - agrees.
#
# It is NOT run `--offline`, which is what every release before this one did. Cargo
# still resolves the whole graph to rewrite five versions, and this workspace has one
# git dependency (`sysproxy`, through `api`), whose checkout a cold runner has never
# fetched: `--offline` then dies with "can't checkout from
# 'https://github.com/clash-verge-rev/sysproxy-rs': you are in the offline mode",
# before a single artifact is built. It passed on a development machine because
# `~/.cargo/git` is warm there, so the mistake only ever showed up in CI. What
# actually keeps registry dependencies still is `--workspace`, not `--offline`;
# measured against a cold CARGO_HOME, the pair moves exactly the five members'
# versions and no other line of Cargo.lock.
cargo update --workspace

# Check every member rather than the one the caller named, which is the point:
# a member left behind by an edit to `[workspace.package]` is the bug this whole
# script exists to prevent, and it is silent — the shell just reports an old
# version, forever, and no build fails.
if ! cargo metadata --locked --no-deps --format-version 1 \
    | jq -er --arg version "$version" '
        .packages
        | map(select(.source == null))
        | (map(.name) | join(" ")) as $names
        | if all(.[]; .version == $version)
          then .[0].name
          else error("packages not at \($version): " +
                     (map(select(.version != $version) | "\(.name) \(.version)") | join(", ")) +
                     " (workspace members: \($names))")
          end
' >/dev/null; then
    echo "stamping $manifest did not put every package at $version" >&2
    exit 1
fi

echo "workspace version set to $version (build $build_number)"
