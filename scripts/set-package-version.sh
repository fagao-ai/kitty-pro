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
# Usage: set-package-version.sh <version>
set -euo pipefail

version="${1:-}"

if [[ -z "$version" ]]; then
    echo "usage: $0 <version>" >&2
    exit 2
fi

# The same grammar the release workflow accepts for a tag, and the same one
# `proxy_core::ReleaseVersion` can order.
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "unsupported package version $version" >&2
    exit 1
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$root/Cargo.toml"

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

# Cargo.lock records the workspace members' versions. `--offline` because this
# only re-derives what the manifests already say: a release job must not be able to
# move a registry dependency while it is moving a version number.
cargo update --offline --workspace

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

echo "workspace version set to $version"
