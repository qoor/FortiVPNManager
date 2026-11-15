#!/bin/bash
# Adds the macOS installer package to the GitHub Release of this version and its SHA-256 to the
# Release's list. The Release workflow makes that Release from the tag v<VERSION> with the Windows
# installer; the package is built here, as the app's signing certificate is only in this Mac's
# Keychain. Run it on the tagged commit with no changes, after the workflow has finished.
# Usage: scripts/release-pkg.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
version=$(cat VERSION)
tag=v$version

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "release-pkg: uncommitted changes; build the release from the tagged commit only" >&2
    exit 1
fi
if [ "$(git rev-parse HEAD)" != "$(git rev-parse --verify --quiet "$tag^{commit}")" ]; then
    echo "release-pkg: HEAD is not the tag $tag" >&2
    exit 1
fi
# The Release MSI has the client from the repository secret; the package takes this Mac's copy.
if [ ! -f packaging/GoogleOAuthClient.json ]; then
    echo "release-pkg: packaging/GoogleOAuthClient.json not found; a release has the built-in client" >&2
    exit 1
fi
gh release view "$tag" >/dev/null

make pkg
pkg=FortiVPNManager-$version.pkg
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
gh release download "$tag" --pattern SHA256SUMS.txt --dir "$work"
{
    grep -v "  $pkg\$" "$work/SHA256SUMS.txt" || true
    (cd .build && shasum -a 256 "$pkg")
} >"$work/sums"
mv "$work/sums" "$work/SHA256SUMS.txt"
gh release upload "$tag" ".build/$pkg" "$work/SHA256SUMS.txt" --clobber
echo "release-pkg: $pkg added to $tag"
