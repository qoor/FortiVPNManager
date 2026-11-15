#!/bin/bash
# Builds the Windows installer on the Windows build PC with the Helper and the runtime DLL built for
# Windows on this Mac, so that cargo never runs there: the PC's security software holds every new
# executable, build scripts included, for minutes on its first run. Prints the installer's path.
# Usage: scripts/msi-windows.sh <folder>
# <folder> is the PC's copy of this tree under %USERPROFILE%\src, kept current by sync-windows.sh,
# which this runs first. Needs: rustup target x86_64-pc-windows-msvc, cargo-xwin, brew llvm, lld and
# mingw-w64.
set -euo pipefail
folder=${1:?usage: scripts/msi-windows.sh <folder>}
host=${FVM_WINDOWS_HOST:-notebook}
target=x86_64-pc-windows-msvc
cd "$(git rev-parse --show-toplevel)"
export PATH="$(brew --prefix)/opt/llvm/bin:$(brew --prefix)/opt/lld/bin:$PATH" XWIN_ACCEPT_LICENSE=1
# The Helper carries the build and build.ps1 gives the app the same, which the app checks before it
# sends a secret.
build=$(date +%Y%m%d%H%M%S)

scripts/sync-windows.sh "$folder"
FVM_BUILD=$build cargo xwin build --release --target "$target" --package fvm-helper-entry-windows
cargo xwin rustc --release --target "$target" --package fvm-runtime --lib --crate-type cdylib
remote="%USERPROFILE%\\src\\$folder"
ssh "$host" "if not exist $remote\\.build\\windows\\rust mkdir $remote\\.build\\windows\\rust"
scp -q "target/$target/release/fvm-helper.exe" "target/$target/release/fvm_runtime.dll" "$host:src/$folder/.build/windows/rust/"
ssh "$host" "powershell -NoProfile -ExecutionPolicy Bypass -File $remote\\packaging\\windows\\build.ps1 -Build $build -RustOutput $remote\\.build\\windows\\rust"
