#!/usr/bin/env bash
# Stamp one workspace package with the version a release tag names.
#
# Each release shell reports its own `CARGO_PKG_VERSION` to the in-app update
# check, which compares it against the newest published release. Only the
# desktop package used to be stamped from the tag, so the Web server and the APK
# kept answering `0.1.0`: every one of their users was told a newer release
# existed on every launch, forever, with no installable version that would stop
# it.
#
# Usage: set-package-version.sh <package> <version>
set -euo pipefail

package="${1:-}"
version="${2:-}"

if [[ -z "$package" || -z "$version" ]]; then
    echo "usage: $0 <package> <version>" >&2
    exit 2
fi

# The same grammar the release workflow accepts for a tag, and the same one
# `proxy_core::ReleaseVersion` can order.
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "unsupported package version $version" >&2
    exit 1
fi

if ! manifest="$(
    cargo metadata --locked --no-deps --format-version 1 \
        | jq -er --arg package "$package" \
            '.packages[] | select(.name == $package) | .manifest_path'
)"; then
    # `jq -e` exits quietly when it selected nothing, which would otherwise look
    # like an unexplained failure of the whole release job.
    echo "no workspace package named $package" >&2
    exit 1
fi

temporary="${manifest}.version.tmp"
trap 'rm -f "$temporary"' EXIT

# Rewrite only the version line inside this manifest's [package] table, leaving
# dependencies and every other package alone.
awk -v version="$version" '
    /^\[package\][[:space:]]*$/ { in_package = 1 }
    /^\[/ && !/^\[package\][[:space:]]*$/ { in_package = 0 }
    in_package && /^version[[:space:]]*=/ {
        print "version = \"" version "\""
        replaced = 1
        next
    }
    { print }
    END { if (!replaced) { print "no [package] version line found" > "/dev/stderr"; exit 1 } }
' "$manifest" >"$temporary"
mv "$temporary" "$manifest"
trap - EXIT

# Cargo.lock records workspace package versions. Update exactly this entry
# without upgrading anything else, so later --locked builds see the new version.
cargo update --package "$package" --precise "$version"

actual="$(
    cargo metadata --locked --no-deps --format-version 1 \
        | jq -er --arg package "$package" \
            '.packages[] | select(.name == $package) | .version'
)"
if [[ "$actual" != "$version" ]]; then
    echo "$package package version $actual does not match $version" >&2
    exit 1
fi

echo "$package version set to $version"
