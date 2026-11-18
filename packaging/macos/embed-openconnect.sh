#!/bin/sh
# Xcode post-build script: puts the OpenConnect that the Helper runs into the app (MacOS/openconnect,
# Resources/vpnc-script) with its license notices, and signs it with the app's identity.
set -eu
zip=$("$SRCROOT/../openconnect/build.sh" macos)
contents=$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH

ditto -x -k "$zip" "$contents"
rm -rf "$contents/Resources/OpenConnect-licenses"
cp -R "$SRCROOT/OpenConnect-licenses" "$contents/Resources/OpenConnect-licenses"
codesign --force --sign "$EXPANDED_CODE_SIGN_IDENTITY" --options runtime --timestamp=none \
    --generate-entitlement-der --strip-disallowed-xattrs "$contents/MacOS/openconnect"
