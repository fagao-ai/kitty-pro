#!/usr/bin/env bash
# Wrap an `.app` in a drag-to-Applications disk image, and prove the result opens.
#
# NSH packages a macOS app as an `.app` and stops there. Its own `package --help`
# (0.5.0) spells the target out as "an ad hoc signed `.app`", and says `.tar.gz` for
# Linux and `.zip` for Windows — macOS is the one platform with no archive, let alone
# an image. So the release has shipped `Kitty Pro.app` in a zip this workflow wrote
# itself, which asks a user to unzip and drag by hand. A dmg is the same two and a
# half steps with the drop target drawn: mount, drag `Kitty Pro.app` onto
# `Applications`, done.
#
# The image is built from a copy rather than a symlinked staging directory, because
# `hdiutil` does not resolve the link into the image: staging the bundle as a link
# writes a 16 KB image containing a path that dangles the moment it is mounted
# somewhere else. Measured, both ways, against this app's bundle.
#
# `ditto` rather than `cp -R` because the zip published beside this image is made
# with `ditto`: one tool copying one bundle is what lets the release say the two
# packages hold the same app. This bundle carries no symlinks of its own — counted,
# zero — so the two tools agree here by fact rather than by that argument, and the
# signature check below is what would notice if a copy ever changed that.
#
# Mounting the finished image and reading the executable back out is the point of
# this script over one `hdiutil` line in the workflow. An image whose signature broke
# on the way in, or whose volume came out empty, is a release asset that installs
# nothing, and the failure belongs to whoever built the image rather than to a user
# three days later. `codesign --verify --deep --strict` is run against the *mounted*
# bundle: the app is ad hoc signed, and an archive that corrupted a sealed resource
# answers here instead of at first launch, where Gatekeeper's complaint is about the
# developer's certificate and not about the build.
#
# Needs: macOS (`hdiutil`, `codesign`). Nothing else; no third-party dmg tool and no
# network, so it runs the same on a runner and on a laptop.
#
# Usage: scripts/build-dmg.sh [-o OUT.dmg] PATH/TO/App.app
set -euo pipefail

out=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        -o)
            out="$2"
            shift 2
            ;;
        -*)
            echo "unknown argument: $1" >&2
            exit 2
            ;;
        *)
            break
            ;;
    esac
done

bundle="${1:-}"
[[ -n "$bundle" ]] || {
    echo "usage: scripts/build-dmg.sh [-o OUT.dmg] PATH/TO/App.app" >&2
    exit 2
}
[[ -d "$bundle" ]] || {
    echo "error: no such bundle: $bundle" >&2
    exit 1
}
if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: a dmg needs macOS, which is where hdiutil lives" >&2
    exit 1
fi

# Absolute, so the staging directory can be anywhere and the volume name comes from
# the bundle's own name. That name is the volume a user sees when the image mounts,
# which is why it is not the archive's name: `Kitty-Pro-macos-aarch64` would mount a
# volume no one would look for, and `Kitty Pro` is the app they are installing.
bundle="$(cd "$bundle" && pwd -P)"
app_name="$(basename "$bundle" .app)"
[[ "$bundle" == *.app ]] || {
    echo "error: $bundle is not an .app bundle" >&2
    exit 1
}
[[ -n "$app_name" ]] || {
    echo "error: $bundle names a bundle with nothing in front of .app" >&2
    exit 1
}

if [[ -z "$out" ]]; then
    out="$PWD/$app_name.dmg"
fi
mkdir -p "$(dirname "$out")"
out="$(cd "$(dirname "$out")" && pwd -P)/$(basename "$out")"

staging="$(mktemp -d "${TMPDIR:-/tmp}/kitty-dmg.XXXXXX")"
mount_point="$(mktemp -d "${TMPDIR:-/tmp}/kitty-dmg-mnt.XXXXXX")"
# Unmount before removing the mount point: detaching a directory that has already
# been deleted is a leak nobody sees, and a volume left mounted outlives the step
# that made it. Both are `|| true` because this runs on a failed build too, where
# there may be nothing to take back.
cleanup() {
    hdiutil detach -quiet "$mount_point" >/dev/null 2>&1 || true
    rm -rf "$staging" "$mount_point"
}
trap cleanup EXIT

ditto "$bundle" "$staging/$(basename "$bundle")"
# The drop target. A relative link would name nothing once the image is mounted at a
# different path, so it points where every Mac keeps its applications.
ln -s /Applications "$staging/Applications"

# UDZO is the compressed read-only form: a dmg nobody can write to is what an
# installer should be, and compression is what makes the image smaller than the
# bundle inside it. Measured against this app's bundle: UDZO 39 MB in 5s, ULFO 35 MB
# in 5s, UDBZ 33 MB in 9s. Bzip is the smallest and the one not chosen: six
# megabytes off a thirty-nine megabyte download is not worth doubling the build's
# time, and nothing about mounting or installing the image is faster for it.
hdiutil create -quiet -volname "$app_name" -srcfolder "$staging" -format UDZO -ov "$out"

# --- verify the image a user would receive ---------------------------------------

# `-mountpoint` rather than the volume name: the volume is `Kitty Pro`, and a path
# with a space in it is exactly the thing that survives quoting in a script and not
# in the parse of `hdiutil attach`'s own tab-separated report.
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mount_point" "$out"

mounted="$(find "$mount_point" -maxdepth 1 -type d -name '*.app' -print -quit)"
[[ -n "$mounted" ]] || {
    echo "error: the image mounted with no .app inside: $out" >&2
    exit 1
}
# The executable carries the engine, the same thing `Verify desktop packages` insists
# on for the zip. An image that mounts and shows an empty volume is the failure mode
# worth failing on, and it is not hypothetical: it is what a staging directory built
# out of symlinks produces.
executable="$(find "$mounted/Contents/MacOS" -type f -perm -u+x -print -quit)"
[[ -n "$executable" ]] || {
    echo "error: $(basename "$mounted") in the image carries no executable" >&2
    exit 1
}
[[ -L "$mount_point/Applications" ]] || {
    echo "error: the image has no Applications shortcut to drag onto" >&2
    exit 1
}
codesign --verify --deep --strict "$mounted"

printf 'dmg: %s (%s bytes)\n' "$out" "$(wc -c <"$out" | tr -d ' ')"
printf '  volume %s · %s · signature verified\n' "$app_name" "$executable"
