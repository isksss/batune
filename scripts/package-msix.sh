#!/bin/sh
# Packs batune.exe into an MSIX for the Microsoft Store, which signs
# it on submission (Smart App Control blocks the unsigned exe from the zip).
# Needs makemsix (docker/msix.Dockerfile).
# Usage: scripts/package-msix.sh <batune.exe> <out.msix>
#
# The Store identity comes from Partner Center (Product identity) through:
#   MSIX_IDENTITY_NAME, MSIX_PUBLISHER (CN=...), MSIX_PUBLISHER_DISPLAY_NAME
# Without them it packs with placeholders, which only checks that packing works:
# the Store rejects a package whose identity does not match the reserved name.
set -eu
exe="$1"
out="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"

if [ -z "${MSIX_IDENTITY_NAME:-}" ] || [ -z "${MSIX_PUBLISHER:-}" ] || [ -z "${MSIX_PUBLISHER_DISPLAY_NAME:-}" ]; then
  echo "MSIX_IDENTITY_NAME・MSIX_PUBLISHER・MSIX_PUBLISHER_DISPLAY_NAME が無いので仮の値で作る(Store には出せない)" >&2
  MSIX_IDENTITY_NAME="batune.dev"
  MSIX_PUBLISHER="CN=batune"
  MSIX_PUBLISHER_DISPLAY_NAME="batune"
fi

# The Store requires the fourth part of the version to be 0, and a pre-release
# suffix is not allowed, so v0.2.1-rc.1 packs as 0.2.1.0 as well.
crate_version="$(sed -n '/^\[package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$root/Cargo.toml")"
version="${crate_version%%-*}.0"

# XML-escape, then escape for the replacement side of sed s|||.
subst() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g' \
    | sed -e 's/[\\|&]/\\&/g'
}

stage="$(mktemp -d)"
trap 'find "$stage" -depth -delete' EXIT
mkdir "$stage/Assets"
cp "$root"/packaging/msix/Assets/*.png "$stage/Assets/"
cp "$exe" "$stage/batune.exe"
cp "$root/LICENSE" "$stage/LICENSE.txt"
cp "$root/fonts/LICENSE" "$stage/LICENSE.HackGen.txt"
sed -e "s|@IDENTITY_NAME@|$(subst "$MSIX_IDENTITY_NAME")|" \
    -e "s|@PUBLISHER@|$(subst "$MSIX_PUBLISHER")|" \
    -e "s|@PUBLISHER_DISPLAY_NAME@|$(subst "$MSIX_PUBLISHER_DISPLAY_NAME")|" \
    -e "s|@VERSION@|$version|" \
    "$root/packaging/msix/AppxManifest.xml" > "$stage/AppxManifest.xml"

rm -f "$out"
makemsix pack -d "$stage" -p "$out"
# makemsix validates the manifest against the schema while packing; unpacking
# also checks the block map (-ss: the Store signs it, so it is unsigned here),
# so a broken package fails here instead of in the Store.
check="$(mktemp -d)"
makemsix unpack -ss -p "$out" -d "$check"
find "$check" -depth -delete
echo "$out ($MSIX_IDENTITY_NAME $version)"
