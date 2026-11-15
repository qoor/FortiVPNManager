#!/bin/sh
# Builds the macOS installer package from a Release build of the app, versioned by the app's own
# version: a component package that puts the app in /Applications only, inside a product package
# with the install checks, the welcome text and the close-the-app step (pkg/Distribution.xml,
# docs/DESIGN.md §5.8). The package is not signed, as that needs a paid Developer ID Installer
# certificate; the app inside keeps its own signature. Prints the package's path.
# Usage: build-pkg.sh <app> <output folder>
set -eu
app=$1 out_dir=$2
pkg=$(cd "$(dirname "$0")" && pwd)/pkg
version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")
out=$out_dir/FortiVPNManager-$version.pkg
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/root" "$work/packages"
# Without extended attributes: the package would carry them, such as the quarantine mark of a
# downloaded Google OAuth client file, and Installer would put them on the installed files. Cleared
# after the copy, as ditto's options leave a cloned file's attributes; macOS keeps
# com.apple.provenance, which is harmless. The app's signature is in its files.
ditto "$app" "$work/root/FortiVPNManager.app"
ditto "$pkg/scripts" "$work/scripts"
xattr -cr "$work/root" "$work/scripts"
codesign --verify --strict --deep "$work/root/FortiVPNManager.app"
pkgbuild --quiet --root "$work/root" --component-plist "$pkg/component.plist" \
    --identifier com.qoor.FortiVPNManager.pkg --version "$version" --install-location /Applications \
    --scripts "$work/scripts" "$work/packages/FortiVPNManager.pkg"
sed "s/@VERSION@/$version/g" "$pkg/Distribution.xml" >"$work/Distribution.xml"
mkdir -p "$out_dir"
productbuild --quiet --distribution "$work/Distribution.xml" --resources "$pkg/Resources" \
    --package-path "$work/packages" "$out"
echo "$out"
