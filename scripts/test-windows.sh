#!/bin/bash
# Builds the Rust tests for Windows on this Mac and runs them on the Windows build PC, so nothing is
# compiled there: the PC's security software holds every new executable for a minute or two on its
# first start, build scripts included. New test executables are copied, started all at once with the
# arguments cargo uses (the hold is per file and arguments) and an environment that makes them exit
# before any test, then run one by one in their package folder, as cargo would.
# Usage: scripts/test-windows.sh <folder> [cargo test options, such as --package fvm-watchdog]
# <folder> is the PC's copy of this tree under %USERPROFILE%\src, kept current by sync-windows.sh,
# which this runs first. Needs: rustup target x86_64-pc-windows-msvc, cargo-xwin, brew llvm and lld.
set -euo pipefail
folder=${1:?usage: scripts/test-windows.sh <folder> [cargo test options]}
shift
host=${FVM_WINDOWS_HOST:-notebook}
target=x86_64-pc-windows-msvc
cd "$(git rev-parse --show-toplevel)"
export PATH="$(brew --prefix)/opt/llvm/bin:$(brew --prefix)/opt/lld/bin:$PATH" XWIN_ACCEPT_LICENSE=1
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

scripts/sync-windows.sh "$folder"
cargo xwin test --no-run --target "$target" --message-format=json-render-diagnostics "$@" >"$work/build.json"
python3 -I - "$PWD" "$work/build.json" >"$work/tests" <<'EOF'
import json, os, sys
root = sys.argv[1]
for line in open(sys.argv[2]):
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("profile", {}).get("test") and m.get("executable"):
        package = os.path.relpath(os.path.dirname(m["manifest_path"]), root).replace("/", "\\")
        print(m["executable"] + "|" + package)
EOF

# Only executables whose content changed since the last copy are sent and started ahead.
state="${XDG_CACHE_HOME:-$HOME/.cache}/fvm-windows-sync/$host-$folder.tests.sha1"
touch "$state"
cut -d'|' -f1 "$work/tests" | xargs shasum -a 1 | LC_ALL=C sort >"$work/now"
LC_ALL=C comm -13 "$state" "$work/now" | cut -c43- >"$work/new"
ssh "$host" "if not exist %USERPROFILE%\\src\\$folder\\.windows-tests mkdir %USERPROFILE%\\src\\$folder\\.windows-tests"
if [ -s "$work/new" ]; then
    tr '\n' '\0' <"$work/new" | xargs -0 -I{} scp -q {} "$host:src/$folder/.windows-tests/"
fi
new_names=$(xargs -n1 basename <"$work/new" | paste -sd, -)
runs=$(while IFS='|' read -r exe package; do printf '%s|%s\n' "$(basename "$exe")" "$package"; done <"$work/tests")

cat >"$work/run.ps1" <<EOF
\$root = Join-Path \$env:USERPROFILE 'src\\$folder'
\$new = '$new_names' -split ',' | Where-Object { \$_ }
\$tests = @'
$runs
'@ -split "\`n" | Where-Object { \$_.Trim() } | ForEach-Object {
    \$exe, \$package = \$_.Trim() -split '\|'
    [pscustomobject]@{ Name = \$exe; Exe = Join-Path \$root ".windows-tests\\\$exe"; Dir = Join-Path \$root \$package }
}
\$started = foreach (\$test in \$tests | Where-Object { \$new -contains \$_.Name }) {
    \$start = New-Object System.Diagnostics.ProcessStartInfo \$test.Exe
    \$start.WorkingDirectory = \$test.Dir
    \$start.EnvironmentVariables['RUST_TEST_THREADS'] = '0'
    \$start.UseShellExecute = \$false
    \$start.CreateNoWindow = \$true
    \$start.RedirectStandardError = \$true
    [System.Diagnostics.Process]::Start(\$start)
}
foreach (\$process in \$started) { \$process.WaitForExit() }
\$failed = 0
foreach (\$test in \$tests) {
    Push-Location \$test.Dir
    \$output = & \$test.Exe 2>&1
    if (\$LASTEXITCODE -ne 0) { \$failed++; \$output }
    "{0}: {1}" -f \$test.Name, ((\$output | Select-String 'test result:') -join ' ')
    Pop-Location
}
"windows tests: \$(\$tests.Count) programs, \$(\$started.Count) new, \$failed failed"
exit [int](\$failed -gt 0)
EOF
code=0
ssh "$host" "powershell -NoProfile -EncodedCommand $(iconv -f UTF-8 -t UTF-16LE "$work/run.ps1" | base64)" | grep -v '^#< CLIXML\|^<Objs' || code=$?
mv "$work/now" "$state"
exit $code
