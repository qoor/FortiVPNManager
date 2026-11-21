#!/bin/sh
# Xcode pre-build script: builds one Rust package for Apple Silicon. A staticlib goes to lib<name>.a in
# BUILT_PRODUCTS_DIR, where the module map's `link` directive finds it; a binary goes to the app's
# Contents/MacOS, signed. Usage: build-rust.sh <package> [<binary> <bundle ID>]
set -eu
package=$1
binary=${2:-}
bundle_id=${3:-}
lib="lib$(echo "$package" | tr - _).a"
root=$(cd "$(dirname "$0")/../.." && pwd)
# Xcode does not run scripts with the login shell's PATH.
export PATH="$HOME/.cargo/bin:$PATH"
# Under DerivedData rather than ./target: `make clean` removes it, and Xcode builds do not wait on
# the build-directory lock held by terminal cargo or rust-analyzer.
target_dir="$OBJROOT/cargo"
if [ "$CONFIGURATION" = Release ]; then
    profile=release
    set -- --release
else
    profile=debug
    set --
fi
if [ -z "$binary" ]; then
    cargo build --manifest-path "$root/Cargo.toml" --package "$package" \
        --target aarch64-apple-darwin --target-dir "$target_dir" "$@"
    built="$target_dir/aarch64-apple-darwin/$profile/$lib"
    # Copy only on change so an unchanged library does not force a relink.
    cmp -s "$built" "$BUILT_PRODUCTS_DIR/$lib" || install -m 644 "$built" "$BUILT_PRODUCTS_DIR/$lib"
    exit 0
fi
plist="$target_dir/$binary-Info.plist"
mkdir -p "$target_dir"
cat >"$plist.new" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>$binary</string>
	<key>CFBundleIdentifier</key>
	<string>$bundle_id</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>$binary</string>
	<key>CFBundleShortVersionString</key>
	<string>$MARKETING_VERSION</string>
	<key>CFBundleSupportedPlatforms</key>
	<array>
		<string>MacOSX</string>
	</array>
	<key>CFBundleVersion</key>
	<string>$CURRENT_PROJECT_VERSION</string>
	<key>LSMinimumSystemVersion</key>
	<string>$MACOSX_DEPLOYMENT_TARGET</string>
</dict>
</plist>
PLIST
cmp -s "$plist.new" "$plist" && rm "$plist.new" || mv "$plist.new" "$plist"
FVM_BUILD="$CURRENT_PROJECT_VERSION" FVM_INFO_PLIST="$plist" \
    cargo build --manifest-path "$root/Cargo.toml" --package "$package" --bin "$binary" \
    --target aarch64-apple-darwin --target-dir "$target_dir" "$@"
destination="$TARGET_BUILD_DIR/$EXECUTABLE_FOLDER_PATH/$binary"
mkdir -p "$(dirname "$destination")"
install -m 755 "$target_dir/aarch64-apple-darwin/$profile/$binary" "$destination"
if [ "$CODE_SIGNING_ALLOWED" != YES ]; then
    exit 0
fi
if [ "$CODE_SIGN_INJECT_BASE_ENTITLEMENTS" = YES ]; then
    entitlements="$target_dir/$binary.entitlements"
    cat >"$entitlements" <<ENTITLEMENTS
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>com.apple.security.get-task-allow</key>
	<true/>
</dict>
</plist>
ENTITLEMENTS
    set -- --entitlements "$entitlements" --generate-entitlement-der
else
    set --
fi
codesign --force --sign "$EXPANDED_CODE_SIGN_IDENTITY" --identifier "$bundle_id" \
    --options runtime --timestamp=none "$@" "$destination"
