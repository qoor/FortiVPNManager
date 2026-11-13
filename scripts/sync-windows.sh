#!/bin/bash
# Copies this work tree to a folder on the Windows build PC, sending only the files whose content
# changed since the last copy to that folder and removing the ones deleted here. Only the files sent
# get a new modification time there, so cargo and dotnet rebuild only what changed, and the build
# makes as few new executables as possible (the PC's security software holds each new one for
# minutes on its first run).
# Usage: scripts/sync-windows.sh <folder>   (a folder under %USERPROFILE%\src on the PC; the ssh host
# is "notebook" unless FVM_WINDOWS_HOST names another). The first copy to a folder sends everything.
set -euo pipefail
folder=${1:?usage: scripts/sync-windows.sh <folder>}
host=${FVM_WINDOWS_HOST:-notebook}
cd "$(git rev-parse --show-toplevel)"
state_dir="${XDG_CACHE_HOME:-$HOME/.cache}/fvm-windows-sync"
mkdir -p "$state_dir"
sent="$state_dir/$host-$folder.sha1"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# What is here now: one "sha1  path" line per existing file git tracks or would add.
git ls-files -co --exclude-standard -z |
    while IFS= read -r -d '' f; do [ -f "$f" ] && printf '%s\0' "$f"; done |
    xargs -0 shasum -a 1 | LC_ALL=C sort >"$work/now"
touch "$sent"
LC_ALL=C comm -13 "$sent" "$work/now" | cut -c43- >"$work/changed"
cut -c43- "$sent" | LC_ALL=C sort >"$work/before"
cut -c43- "$work/now" | LC_ALL=C sort >"$work/after"
LC_ALL=C comm -23 "$work/before" "$work/after" >"$work/gone"

remote="%USERPROFILE%\\src\\$folder"
ssh "$host" "if not exist $remote mkdir $remote"
if [ -s "$work/changed" ]; then
    COPYFILE_DISABLE=1 tar -cf - -T "$work/changed" | ssh "$host" "tar -xmf - -C $remote"
fi
if [ -s "$work/gone" ]; then
    ssh "$host" "powershell -NoProfile -Command \"[Console]::InputEncoding = [Text.Encoding]::UTF8; \$input | ForEach-Object { Remove-Item -LiteralPath (Join-Path \$env:USERPROFILE\\src\\$folder \$_) -Force -ErrorAction SilentlyContinue }\"" <"$work/gone"
fi
mv "$work/now" "$sent"
echo "sync-windows: $(wc -l <"$work/changed" | tr -d ' ') sent, $(wc -l <"$work/gone" | tr -d ' ') removed ($host:$remote)"
