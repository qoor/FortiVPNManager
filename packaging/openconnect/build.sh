#!/bin/sh
set -eu
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
libxml2_version=2.15.4
libxml2_url=https://download.gnome.org/sources/libxml2/2.15/libxml2-$libxml2_version.tar.xz
libxml2_sha256=98087fd181d9070724f3fbc65c7377db03038eb92bd882374daff44940138821
libiconv_version=1.19.1
libiconv_url=https://ftpmirror.gnu.org/libiconv/libiconv-$libiconv_version.tar.gz
libiconv_sha256=9a06844846d664d7122161700c10a64480d7c7bcead3091925a785b38c91edab
zlib_version=1.4
zlib_url=https://github.com/madler/zlib/releases/download/v$zlib_version/zlib-$zlib_version.tar.xz
zlib_sha256=4f11c512053d7bab8d9e9b89bde3575add3e4c50a383b42ef6cb837035fc5a7c
wintun_url=https://www.wintun.net/builds/wintun-0.14.1.zip
wintun_sha256=07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51

usage='usage: packaging/openconnect/build.sh <macos|windows> [path]'
os=${1-}
case $os in
    macos) name=macos-arm64 ;;
    windows) name=windows-x64 ;;
    *) echo "$usage" >&2; exit 2 ;;
esac
root=$(cd "$(dirname "$0")/../.." && pwd)
brew=/opt/homebrew
host=x86_64-w64-mingw32
toolchain=$brew/opt/mingw-w64/toolchain-x86_64/bin
gitlink=$(git -C "$root" rev-parse :third_party/openconnect)

if [ "${2-}" != build ]; then
    formulas="autoconf automake libtool pkgconf"
    tools="autoconf automake glibtoolize pkgconf"
    if [ "$os" = windows ]; then
        formulas="$formulas mingw-w64"
        tools="$tools $host-gcc"
    fi
    for tool in $tools; do
        [ -x "$brew/bin/$tool" ] || { echo "error: $brew/bin/$tool not found: brew install $formulas" >&2; exit 1; }
    done
    hash=$(
        {
            echo "$os"
            echo "$gitlink"
            shasum -a 256 < "$0"
            for formula in $formulas; do
                readlink "$brew/opt/$formula"
            done
            if [ "$os" = macos ]; then
                xcrun clang --version | head -1
                xcrun --show-sdk-version
            fi
        } | shasum -a 256 | cut -c 1-16
    )
    zip=${XDG_CACHE_HOME:-$HOME/.cache}/fvm-openconnect/$name-$hash.zip
    if [ "${2-}" = path ] || [ -f "$zip" ]; then
        echo "$zip"
        exit 0
    fi
    git -C "$root/third_party/openconnect" cat-file -e "$gitlink^{commit}" 2>/dev/null || {
        echo "error: third_party/openconnect does not have the commit $gitlink: git submodule update --init third_party/openconnect" >&2
        exit 1
    }
    work=$(mktemp -d)
    if ! env -i HOME="$HOME" TMPDIR="$TMPDIR" PATH=/usr/bin:/bin:/usr/sbin:/sbin DEVELOPER_DIR="$(xcode-select -p)" \
        "$0" "$os" build "$work" > "$work/build.log" 2>&1; then
        echo "error: OpenConnect build failed, log: $work/build.log" >&2
        tail -n 40 "$work/build.log" >&2
        exit 1
    fi
    mkdir -p "${zip%/*}"
    mv -f "$work/openconnect.zip" "$zip.$$"
    mv -f "$zip.$$" "$zip"
    rm -rf "$work"
    echo "$zip"
    exit 0
fi

work=$3
prefix=$work/prefix
stage=$work/stage
ncpu=$(sysctl -n hw.ncpu)
export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig"
export CPPFLAGS="-I$prefix/include" CFLAGS=-O2 LDFLAGS="-L$prefix/lib"

fetch() {
    file=$work/${1##*/}
    curl --fail --silent --show-error --location --output "$file" "$1"
    echo "$2  $file" | shasum -a 256 --check --status || { echo "SHA-256 mismatch: $1" >&2; exit 1; }
}

fail() {
    echo "check failed: $1" >&2
    exit 1
}

imports() {
    "$host-objdump" -p "$1" | awk '/DLL Name:/ { print $3 }'
}

copy_libraries() {
    imports "$1" | while read -r name; do
        [ ! -e "$stage/$name" ] || continue
        for directory in "$prefix/bin" "$toolchain/../$host/lib" "$toolchain/../$host/bin"; do
            if [ -e "$directory/$name" ]; then
                cp "$directory/$name" "$stage/$name"
                copy_libraries "$stage/$name"
                break
            fi
        done
    done
}

mkdir "$work/openconnect"
git -C "$root/third_party/openconnect" archive "$gitlink" | tar -x -C "$work/openconnect"
cd "$work/openconnect"
PATH="$brew/bin:$PATH" ./autogen.sh
RPM_PACKAGE_VERSION=$(sed -n 's/^AC_INIT(\[openconnect\], \[\(.*\)\])$/\1/p' configure.ac)
RPM_PACKAGE_RELEASE=fvm-$(echo "$gitlink" | cut -c 1-8)
export RPM_PACKAGE_VERSION RPM_PACKAGE_RELEASE

if [ "$os" = macos ]; then
    export PKG_CONFIG="$brew/bin/pkgconf --static" MACOSX_DEPLOYMENT_TARGET=15.0
    fetch "$gnutls_url" "$gnutls_sha256"
    fetch "$nettle_url" "$nettle_sha256"
    fetch "$gmp_url" "$gmp_sha256"
    fetch "$vpnc_script_url" "$vpnc_script_sha256"
    for archive in "$work"/*.tar.*; do
        tar -xf "$archive" -C "$work"
    done

    cd "$work/gmp-$gmp_version"
    ./configure --prefix="$prefix" --enable-static --disable-shared --with-pic --disable-cxx
    make -j"$ncpu"
    make install

    cd "$work/nettle-$nettle_version"
    ./configure --prefix="$prefix" --enable-static --disable-shared --disable-documentation --disable-openssl
    make -j"$ncpu"
    make install

    cd "$work/gnutls-$gnutls_version"
    ./configure --prefix="$prefix" --enable-static --disable-shared \
        --with-included-libtasn1 --with-included-unistring --without-p11-kit --without-idn \
        --without-tpm --without-tpm2 --without-brotli --without-zstd \
        --disable-nls --disable-doc --disable-tools --disable-tests --disable-cxx --disable-libdane \
        --with-system-priority-file=
    make -j"$ncpu"
    make install

    cd "$work/openconnect"
    ./configure --prefix="$prefix" --disable-shared --enable-static \
        --with-vpnc-script=/Applications/FortiVPNManager.app/Contents/Resources/vpnc-script \
        --disable-nls --without-stoken --without-libpskc --without-libpcsclite --without-gssapi \
        --without-libproxy --without-lz4 --with-builtin-json --without-gnutls-tss2 --without-external-browser \
        LIBXML2_CFLAGS="-I$(xcrun --show-sdk-path)/usr/include/libxml2" LIBXML2_LIBS=-lxml2
    make -j"$ncpu" install-sbinPROGRAMS

    file=$stage/MacOS/openconnect
    mkdir -p "$stage/MacOS" "$stage/Resources"
    cp "$prefix/sbin/openconnect" "$file"
    install -m 755 "$work/vpnc-script" "$stage/Resources/vpnc-script"
    strip -S "$file"
    codesign -f -s - "$file"
    [ "$(lipo -archs "$file")" = arm64 ] || fail "$file is not arm64 only"
    otool -l "$file" | awk '$1 == "cmd" { build = $2 == "LC_BUILD_VERSION" } build && $1 == "minos" { print $2 }' |
        grep -qx 15.0 || fail "$file does not have minos 15.0"
    otool -hv "$file" | grep -qw TWOLEVEL || fail "$file is not linked with a two-level namespace"
    paths=$(otool -L "$file" | awk 'NR > 1 && $1 !~ /^(\/usr\/lib\/|\/System\/Library\/)/ { print $1 }')
    [ -z "$paths" ] || fail "$file loads $paths"
    if grep -rlF -e /opt/homebrew -e "$work" "$stage"; then
        fail "the files above hold /opt/homebrew or $work"
    fi
    "$file" --version || fail "MacOS/openconnect --version"
else
    export PATH="$toolchain:$PATH" PKG_CONFIG="$brew/bin/pkgconf"
    fetch "$gnutls_url" "$gnutls_sha256"
    fetch "$nettle_url" "$nettle_sha256"
    fetch "$gmp_url" "$gmp_sha256"
    fetch "$libxml2_url" "$libxml2_sha256"
    fetch "$libiconv_url" "$libiconv_sha256"
    fetch "$zlib_url" "$zlib_sha256"
    fetch "$wintun_url" "$wintun_sha256"
    for archive in "$work"/*.tar.*; do
        tar -xf "$archive" -C "$work"
    done

    cd "$work/zlib-$zlib_version"
    make -f win32/Makefile.gcc PREFIX="$host-" SHARED_MODE=1 prefix="$prefix" \
        BINARY_PATH="$prefix/bin" INCLUDE_PATH="$prefix/include" LIBRARY_PATH="$prefix/lib" install
    rm "$prefix/lib/libz.a"

    cd "$work/gmp-$gmp_version"
    ./configure --host="$host" --prefix="$prefix" --enable-shared --disable-static --with-pic --disable-cxx \
        CC="$host-gcc" CFLAGS="$CFLAGS -std=gnu17"
    make -j"$ncpu"
    make install

    cd "$work/nettle-$nettle_version"
    ./configure --host="$host" --prefix="$prefix" --enable-shared --disable-static --disable-documentation --disable-openssl
    make -j"$ncpu"
    make install

    cd "$work/gnutls-$gnutls_version"
    ./configure --host="$host" --prefix="$prefix" --enable-shared --disable-static \
        --with-included-libtasn1 --with-included-unistring --without-p11-kit --without-idn \
        --without-tpm --without-tpm2 --without-brotli --without-zstd \
        --disable-nls --disable-doc --disable-tools --disable-tests --disable-cxx --disable-libdane \
        --with-system-priority-file=
    make -j"$ncpu"
    make install

    cd "$work/libiconv-$libiconv_version"
    ./configure --host="$host" --prefix="$prefix" --enable-shared --disable-static --disable-nls
    make -j"$ncpu"
    make install

    cd "$work/libxml2-$libxml2_version"
    ./configure --host="$host" --prefix="$prefix" --enable-shared --disable-static --without-python
    make -j"$ncpu"
    make install

    cd "$work/openconnect"
    ./configure --host="$host" --prefix="$prefix" --enable-shared --disable-static \
        --with-vpnc-script=vpnc-script-win.js \
        --disable-nls --without-stoken --without-libpskc --without-libpcsclite --without-gssapi \
        --without-libproxy --without-lz4 --with-builtin-json --without-gnutls-tss2 --without-external-browser
    make -j"$ncpu" install-libLTLIBRARIES install-sbinPROGRAMS

    mkdir -p "$stage"
    cp "$prefix/sbin/openconnect.exe" "$stage/"
    copy_libraries "$stage/openconnect.exe"
    for file in "$stage"/*; do
        "$host-strip" -S "$file"
    done
    unzip -q -j "$work/${wintun_url##*/}" wintun/bin/amd64/wintun.dll -d "$stage"
    for file in "$stage"/*; do
        "$host-objdump" -f "$file" | grep -q 'file format pei-x86-64' || fail "$file is not x86-64 PE"
        if imports "$file" | grep -qix msvcrt.dll; then
            fail "$file imports msvcrt.dll"
        fi
    done
fi

ditto -c -k --norsrc --noextattr --noacl "$stage" "$work/openconnect.zip"
