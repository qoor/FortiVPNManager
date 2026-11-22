#!/bin/sh
set -eu
openssl_version=3.5.9
openssl_url=https://github.com/openssl/openssl/releases/download/openssl-$openssl_version/openssl-$openssl_version.tar.gz
openssl_sha256=603f5602e2eef00d77fbd429d34dcd5822bb301757a1bc9cdb24c670f1eb859a
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
system=$(uname -s)
if [ "$system" = Darwin ]; then
    cross=$host-
    sha256() { shasum -a 256 "$@"; }
else
    cross=
    sha256() { sha256sum "$@"; }
fi

if [ "${2-}" != build ]; then
    if [ "$system" = Darwin ]; then
        formulas="autoconf automake libtool pkgconf"
        tools="autoconf automake glibtoolize pkgconf"
        if [ "$os" = windows ]; then
            formulas="$formulas mingw-w64"
            tools="$tools $host-gcc"
        fi
        for tool in $tools; do
            [ -x "$brew/bin/$tool" ] || { echo "error: $brew/bin/$tool not found: brew install $formulas" >&2; exit 1; }
        done
    else
        packages="mingw-w64-ucrt-x86_64-gcc mingw-w64-ucrt-x86_64-pkgconf autoconf automake libtool make perl curl git zip unzip"
        for tool in gcc pkgconf autoconf automake libtoolize make perl curl git zip unzip; do
            command -v "$tool" >/dev/null || { echo "error: $tool not found: pacman -S --needed $packages" >&2; exit 1; }
        done
    fi
    hash=$(
        {
            echo "$os"
            echo "$gitlink"
            sha256 < "$0"
            if [ "$os" = macos ]; then
                xcrun clang --version | head -1
                xcrun --show-sdk-version
            elif [ "$system" = Darwin ]; then
                readlink "$brew/opt/mingw-w64"
            else
                gcc --version | head -1
            fi
        } | sha256 | cut -c 1-16
    )
    zip=${XDG_CACHE_HOME:-$HOME/.cache}/fvm-openconnect/$name-$hash.zip
    shown=$zip
    [ "$system" = Darwin ] || shown=$(cygpath -m "$zip")
    if [ "${2-}" = path ] || [ -f "$zip" ]; then
        echo "$shown"
        exit 0
    fi
    git -C "$root/third_party/openconnect" cat-file -e "$gitlink^{commit}" 2>/dev/null || {
        echo "error: third_party/openconnect does not have the commit $gitlink: git submodule update --init third_party/openconnect" >&2
        exit 1
    }
    work=$(mktemp -d)
    if [ "$system" = Darwin ]; then
        set -- env -i HOME="$HOME" TMPDIR="$TMPDIR" PATH=/usr/bin:/bin:/usr/sbin:/sbin DEVELOPER_DIR="$(xcode-select -p)"
    else
        set -- env -i HOME="$HOME" TMPDIR=/tmp TMP=/tmp TEMP=/tmp MSYSTEM=UCRT64 PATH=/ucrt64/bin:/usr/bin \
            ACLOCAL_PATH=/ucrt64/share/aclocal:/usr/share/aclocal COMSPEC="$COMSPEC" SYSTEMROOT="$SYSTEMROOT"
    fi
    if ! "$@" "$0" "$os" build "$work" > "$work/build.log" 2>&1; then
        echo "error: OpenConnect build failed, log: $work/build.log" >&2
        tail -n 40 "$work/build.log" >&2
        exit 1
    fi
    mkdir -p "${zip%/*}"
    mv -f "$work/openconnect.zip" "$zip.$$"
    mv -f "$zip.$$" "$zip"
    rm -rf "$work"
    echo "$shown"
    exit 0
fi

work=$3
prefix=$work/prefix
openssl=$work/openssl-$openssl_version
stage=$work/stage
if [ "$system" = Darwin ]; then
    ncpu=$(sysctl -n hw.ncpu)
else
    ncpu=$(nproc)
fi
export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig"
export CPPFLAGS="-I$prefix/include" CFLAGS=-O2 LDFLAGS="-L$prefix/lib"

fetch() {
    file=$work/${1##*/}
    curl --fail --silent --show-error --location --output "$file" "$1"
    echo "$2  $file" | sha256 --check --status || { echo "SHA-256 mismatch: $1" >&2; exit 1; }
}

fail() {
    echo "check failed: $1" >&2
    exit 1
}

imports() {
    "${cross}objdump" -p "$1" | awk '/DLL Name:/ { print $3 }'
}

copy_libraries() {
    imports "$1" | while read -r name; do
        [ ! -e "$stage/$name" ] || continue
        for directory in "$prefix/bin" "$openssl" "$toolchain/../$host/lib" "$toolchain/../$host/bin" /ucrt64/bin; do
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
    fetch "$openssl_url" "$openssl_sha256"
    for archive in "$work"/*.tar.*; do
        tar -xf "$archive" -C "$work"
    done

    cd "$openssl"
    env -u CFLAGS -u CPPFLAGS -u LDFLAGS ./Configure darwin64-arm64-cc --prefix=/var/empty \
        no-shared no-engine no-autoload-config
    make -j"$ncpu" build_libs

    cd "$work/openconnect"
    ./configure --prefix="$prefix" --disable-shared --enable-static \
        --with-vpnc-script=/dev/null --without-gnutls \
        --disable-nls --without-stoken --without-libpskc --without-libpcsclite --without-gssapi \
        --without-libproxy --without-lz4 --with-builtin-json --without-external-browser \
        LIBXML2_CFLAGS="-I$(xcrun --show-sdk-path)/usr/include/libxml2" LIBXML2_LIBS=-lxml2 \
        OPENSSL_CFLAGS="-I$openssl/include" OPENSSL_LIBS="-L$openssl -lssl -lcrypto"
    make -j"$ncpu" install-libLTLIBRARIES

    mkdir -p "$stage/lib"
    cp "$prefix/lib/libopenconnect.a" "$openssl/libssl.a" "$openssl/libcrypto.a" "$stage/lib/"
    strip -S "$stage"/lib/*.a
    for library in "$stage"/lib/*.a; do
        [ "$(lipo -archs "$library")" = arm64 ] || fail "$library is not arm64 only"
        minos=$(otool -l "$library" | awk '$1 == "cmd" { build = $2 == "LC_BUILD_VERSION" } build && $1 == "minos" { print $2 }' | sort -u)
        [ "$minos" = 15.0 ] || fail "$library does not have minos 15.0"
    done
    printf '#include <stdio.h>\nconst char *openconnect_get_version(void);\nconst char *OpenSSL_version(int type);\nint main(void) { puts(openconnect_get_version()); printf("Using %%s\\n", OpenSSL_version(0)); return 0; }\n' >"$work/probe.c"
    cc -o "$work/probe" "$work/probe.c" -L"$stage/lib" -lopenconnect -lssl -lcrypto -lxml2 -lz -liconv
    "$work/probe" || fail "a program linked against lib/ does not run"
    paths=$(otool -L "$work/probe" | awk 'NR > 1 && $1 !~ /^(\/usr\/lib\/|\/System\/Library\/)/ { print $1 }')
    [ -z "$paths" ] || fail "$work/probe loads $paths"
    if grep -rlF -e /opt/homebrew -e "$work" "$stage"; then
        fail "the files above hold /opt/homebrew or $work"
    fi
else
    if [ "$system" = Darwin ]; then
        export PATH="$toolchain:$PATH" PKG_CONFIG="$brew/bin/pkgconf"
    else
        export PKG_CONFIG=pkgconf
    fi
    fetch "$openssl_url" "$openssl_sha256"
    fetch "$libxml2_url" "$libxml2_sha256"
    fetch "$libiconv_url" "$libiconv_sha256"
    fetch "$zlib_url" "$zlib_sha256"
    fetch "$wintun_url" "$wintun_sha256"
    for archive in "$work"/*.tar.*; do
        tar -xf "$archive" -C "$work"
    done

    cd "$work/zlib-$zlib_version"
    make -f win32/Makefile.gcc PREFIX="$cross" SHARED_MODE=1 prefix="$prefix" \
        BINARY_PATH="$prefix/bin" INCLUDE_PATH="$prefix/include" LIBRARY_PATH="$prefix/lib" install
    rm "$prefix/lib/libz.a"

    cd "$openssl"
    env -u CFLAGS -u CPPFLAGS -u LDFLAGS ./Configure mingw64 --cross-compile-prefix="$cross" \
        --prefix=C:/Windows/System32/OpenSSL no-engine no-autoload-config
    make -j"$ncpu" build_libs

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
        --with-vpnc-script=vpnc-script-win.js --without-gnutls \
        --disable-nls --without-stoken --without-libpskc --without-libpcsclite --without-gssapi \
        --without-libproxy --without-lz4 --with-builtin-json --without-external-browser \
        OPENSSL_CFLAGS="-I$openssl/include" OPENSSL_LIBS="-L$openssl -lssl -lcrypto"
    make -j"$ncpu" install-libLTLIBRARIES

    mkdir -p "$stage"
    cp "$prefix/bin/libopenconnect-5.dll" "$stage/"
    copy_libraries "$stage/libopenconnect-5.dll"
    for file in "$stage"/*; do
        "${cross}strip" -S "$file"
    done
    unzip -q -j "$work/${wintun_url##*/}" wintun/bin/amd64/wintun.dll -d "$stage"
    for file in "$stage"/*; do
        "${cross}objdump" -f "$file" | grep -q 'file format pei-x86-64' || fail "$file is not x86-64 PE"
        if imports "$file" | grep -qix msvcrt.dll; then
            fail "$file imports msvcrt.dll"
        fi
    done
fi

if [ "$system" = Darwin ]; then
    ditto -c -k --norsrc --noextattr --noacl "$stage" "$work/openconnect.zip"
else
    (cd "$stage" && zip -q -r "$work/openconnect.zip" .)
fi
