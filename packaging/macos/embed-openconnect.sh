#!/bin/sh
# Xcode post-build script: puts the OpenConnect that the Helper runs into the app (MacOS/openconnect,
# Frameworks/<libraries>, Resources/vpnc-script) with its license notices, and signs it with the app's
# identity. The files come as a zip from this repository's release openconnect-<version>, which this
# downloads once into .build/macos and checks against its SHA-256. For a new version, update the
# version and the SHA-256 that build-openconnect.sh prints for the zip it built.
set -eu
version=9.21
url=https://github.com/qoor/FortiVPNManager/releases/download/openconnect-$version/openconnect-$version-macos-arm64.zip
sha256=307c29ba05596a7dad7c350daf0d2ec52a19233e7d4876942cab003242d71cd6

root=$(cd "$(dirname "$0")/../.." && pwd)
zip=$root/.build/macos/openconnect-$version-macos-arm64.zip
contents=$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH

if [ ! -f "$zip" ]; then
    curl --fail --silent --show-error --location --create-dirs --output "$zip.download" "$url" ||
        { echo "error: downloading $url failed" >&2; exit 1; }
    mv "$zip.download" "$zip"
fi
hash=$(shasum -a 256 "$zip" | cut -d ' ' -f 1)
if [ "$hash" != "$sha256" ]; then
    echo "error: $zip has SHA-256 $hash, not $sha256 of the OpenConnect $version files; delete it to download it again" >&2
    exit 1
fi
ditto -x -k "$zip" "$contents"
rm -rf "$contents/Resources/OpenConnect-licenses"
cp -R "$SRCROOT/OpenConnect-licenses" "$contents/Resources/OpenConnect-licenses"
for file in $(zipinfo -1 "$zip" 'Frameworks/*.dylib') MacOS/openconnect; do
    codesign --force --sign "$EXPANDED_CODE_SIGN_IDENTITY" --options runtime --timestamp=none \
        --generate-entitlement-der --strip-disallowed-xattrs "$contents/$file"
done
