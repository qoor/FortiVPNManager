# Runs cargo on the Windows build PC and prints its output once it is done.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\cargo-windows.ps1 test --package fvm-watchdog --test windows
# Write --package, not -p: PowerShell takes -p as its own -PipelineVariable and never passes it on.
# Output that test executables write straight to an SSH session does not arrive, so it goes through
# files.
param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $CargoArgs)
$out = Join-Path $env:TEMP 'fvm-cargo-out.txt'
$err = Join-Path $env:TEMP 'fvm-cargo-err.txt'
$cargo = Start-Process -FilePath cargo -ArgumentList $CargoArgs -NoNewWindow -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
$null = $cargo.Handle
$cargo.WaitForExit()
Get-Content $err
Get-Content $out
"cargo exit code: $($cargo.ExitCode)"
exit $cargo.ExitCode
