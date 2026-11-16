#!/bin/sh
# Builds OpenConnect for the macOS app from pinned sources, with its own GnuTLS, Nettle and GMP, for
# macOS 15.0 or later on Apple Silicon: with no Homebrew paths and none of the features the app does
# not use, as the Helper runs it as root. Packs it, signed ad hoc, in the layout of the app's Contents
# folder (MacOS/openconnect, Frameworks/<libraries>, Resources/vpnc-script) as
# .build/macos/openconnect-<version>-macos-arm64.zip, which the app build downloads from this
# repository's release openconnect-<version> and signs again. Prints the zip's path and SHA-256.
# Needs Xcode and pkgconf (brew install pkgconf), which runs during the build only.
# For a new version, update the versions, URLs and SHA-256 values here, run this, upload the zip to a
# release openconnect-<version>, and update the zip's SHA-256 in the app build.
# Usage: packaging/macos/build-openconnect.sh
set -eu
openconnect_version=9.21
openconnect_url=https://www.infradead.org/openconnect/download/openconnect-$openconnect_version.tar.gz
openconnect_sha256=5b32369467db6e5f317aa1ed12cfcbb81ed00bdbc765450b6bfcbdc300944a58
gnutls_version=3.8.13
gnutls_url=https://www.gnupg.org/ftp/gcrypt/gnutls/v3.8/gnutls-$gnutls_version.tar.xz
gnutls_sha256=ffed8ec1bf09c2426d4f14aae377de4753b53e537d685e604e99a8b16ca9c97e
nettle_version=4.0
nettle_url=https://ftpmirror.gnu.org/nettle/nettle-$nettle_version.tar.gz
nettle_sha256=3addbc00da01846b232fb3bc453538ea5468da43033f21bb345cb1e9073f5094
gmp_version=6.3.0
gmp_url=https://ftpmirror.gnu.org/gmp/gmp-$gmp_version.tar.xz
gmp_sha256=a3c2b80201b89e68616f4ad30bc66aee4927c3ce50e33929ca819d5c43538898
vpnc_script_url=https://gitlab.com/openconnect/vpnc-scripts/-/raw/ce9e961bd0f6b867e1c7c35f78f6fb973f6ff101/vpnc-script
vpnc_script_sha256=f0c4d936a382f07711263242699b5e2d85d1ace37136bb78785d352997c17742

root=$(cd "$(dirname "$0")/../.." && pwd)
build=$root/.build/macos/openconnect-build
prefix=$build/prefix
stage=$build/stage
zip=$root/.build/macos/openconnect-$openconnect_version-macos-arm64.zip
ncpu=$(sysctl -n hw.ncpu)
pkg_config=$(command -v pkgconf) || { echo 'pkgconf not found: brew install pkgconf' >&2; exit 1; }

unset CPATH C_INCLUDE_PATH LIBRARY_PATH PKG_CONFIG_PATH CONFIG_SITE
export PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PKG_CONFIG="$pkg_config" PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig"
export MACOSX_DEPLOYMENT_TARGET=15.0
export CPPFLAGS="-I$prefix/include" CFLAGS=-O2 LDFLAGS="-L$prefix/lib -Wl,-headerpad_max_install_names"

fetch() {
    file=$build/${1##*/}
    curl --fail --silent --show-error --location --output "$file" "$1"
    echo "$2  $file" | shasum -a 256 --check --status || { echo "SHA-256 mismatch: $1" >&2; exit 1; }
}

fail() {
    echo "check failed: $1" >&2
    exit 1
}

rm -rf "$build" "$zip"
mkdir -p "$build"
fetch "$openconnect_url" "$openconnect_sha256"
fetch "$gnutls_url" "$gnutls_sha256"
fetch "$nettle_url" "$nettle_sha256"
fetch "$gmp_url" "$gmp_sha256"
fetch "$vpnc_script_url" "$vpnc_script_sha256"
for archive in "$build"/*.tar.*; do
    tar -xf "$archive" -C "$build"
done

cd "$build/gmp-$gmp_version"
./configure --prefix="$prefix" --enable-shared --disable-static --with-pic --disable-cxx
make -j"$ncpu"
make install

cd "$build/nettle-$nettle_version"
./configure --prefix="$prefix" --enable-shared --disable-static --disable-documentation --disable-openssl
make -j"$ncpu"
make install

cd "$build/gnutls-$gnutls_version"
./configure --prefix="$prefix" --enable-shared --disable-static \
    --with-included-libtasn1 --with-included-unistring --without-p11-kit --without-idn \
    --without-tpm --without-tpm2 --without-brotli --without-zstd \
    --disable-nls --disable-doc --disable-tools --disable-tests --disable-cxx --disable-libdane \
    --with-system-priority-file=
make -j"$ncpu"
make install

cd "$build/openconnect-$openconnect_version"
./configure --prefix="$prefix" --enable-shared --disable-static \
    --with-vpnc-script=/Applications/FortiVPNManager.app/Contents/Resources/vpnc-script \
    --disable-nls --without-stoken --without-libpskc --without-libpcsclite --without-gssapi \
    --without-libproxy --without-lz4 --with-builtin-json --without-gnutls-tss2 --without-external-browser \
    LIBXML2_CFLAGS="-I$(xcrun --show-sdk-path)/usr/include/libxml2" LIBXML2_LIBS=-lxml2
make -j"$ncpu"
make install

mkdir -p "$stage/MacOS" "$stage/Frameworks" "$stage/Resources"
cp "$prefix/sbin/openconnect" "$stage/MacOS/openconnect"
install -m 755 "$build/vpnc-script" "$stage/Resources/vpnc-script"
copy_libraries() {
    otool -L "$1" | awk -v lib="$prefix/lib/" 'index($1, lib) == 1 { print substr($1, length(lib) + 1) }' |
        while read -r name; do
            if [ ! -e "$stage/Frameworks/$name" ]; then
                cp "$prefix/lib/$name" "$stage/Frameworks/$name"
                copy_libraries "$stage/Frameworks/$name"
            fi
        done
}
copy_libraries "$stage/MacOS/openconnect"

for file in "$stage/MacOS/openconnect" "$stage"/Frameworks/*; do
    strip -S "$file"
    case $file in
        *.dylib) install_name_tool -id "@rpath/${file##*/}" "$file" ;;
    esac
    otool -L "$file" | awk -v lib="$prefix/lib/" 'NR > 1 && index($1, lib) == 1 { print $1 }' |
        while read -r path; do
            install_name_tool -change "$path" "@rpath/${path##*/}" "$file"
        done
done
install_name_tool -add_rpath @executable_path/../Frameworks "$stage/MacOS/openconnect"

for file in "$stage/MacOS/openconnect" "$stage"/Frameworks/*; do
    codesign -f -s - "$file"
    [ "$(lipo -archs "$file")" = arm64 ] || fail "$file is not arm64 only"
    otool -l "$file" | awk '$1 == "cmd" { build = $2 == "LC_BUILD_VERSION" } build && $1 == "minos" { print $2 }' |
        grep -qx 15.0 || fail "$file does not have minos 15.0"
    otool -hv "$file" | grep -qw TWOLEVEL || fail "$file is not linked with a two-level namespace"
    paths=$(otool -L "$file" | awk 'NR > 1 && $1 !~ /^(@rpath\/|\/usr\/lib\/|\/System\/Library\/)/ { print $1 }')
    [ -z "$paths" ] || fail "$file loads $paths"
    paths=$(otool -l "$file" | awk '$1 == "cmd" { rpath = $2 == "LC_RPATH" } rpath && $1 == "path" && $2 ~ /^\// { print $2 }')
    [ -z "$paths" ] || fail "$file has the run path $paths"
done
if grep -rlF -e /opt/homebrew -e "$build" "$stage"; then
    fail "the files above hold /opt/homebrew or $build"
fi
"$stage/MacOS/openconnect" --version || fail "MacOS/openconnect --version"

ditto -c -k --norsrc --noextattr --noacl "$stage" "$zip"
shasum -a 256 "$zip"
