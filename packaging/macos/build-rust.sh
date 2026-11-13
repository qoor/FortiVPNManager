#!/bin/sh
# Xcode pre-build script: builds one Rust staticlib package for Apple Silicon and puts lib<name>.a in
# BUILT_PRODUCTS_DIR, where the module map's `link` directive finds it. Usage: build-rust.sh <package>
set -eu
package=$1
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
cargo build --manifest-path "$root/Cargo.toml" --package "$package" \
    --target aarch64-apple-darwin --target-dir "$target_dir" "$@"
built="$target_dir/aarch64-apple-darwin/$profile/$lib"
# Copy only on change so an unchanged library does not force a relink.
cmp -s "$built" "$BUILT_PRODUCTS_DIR/$lib" || install -m 644 "$built" "$BUILT_PRODUCTS_DIR/$lib"
