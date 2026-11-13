# Runs cargo on the Windows build PC and prints its output once it is done.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts\cargo-windows.ps1 test --package fvm-watchdog --test windows
# Write --package, not -p: PowerShell takes -p as its own -PipelineVariable and never passes it on.
# Output that test executables write straight to an SSH session does not arrive, so it goes through
# files.
# The PC holds a new executable for a minute or two on its first start (again when the arguments
# differ), but many side by side, so test first builds the test executables and starts them all at once.
param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $CargoArgs)
# Every copy of the repo on the PC builds into one shared folder next to them, so dependencies are
# built, and their build scripts held, once rather than once per copy.
$env:CARGO_TARGET_DIR = Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) 'cargo-target'
$out = Join-Path $env:TEMP 'fvm-cargo-out.txt'
$err = Join-Path $env:TEMP 'fvm-cargo-err.txt'
function Invoke-Cargo([string[]] $Arguments) {
    $cargo = Start-Process -FilePath cargo -ArgumentList $Arguments -NoNewWindow -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
    $null = $cargo.Handle
    $cargo.WaitForExit()
    $cargo.ExitCode
}
if ($CargoArgs[0] -eq 'test' -and $CargoArgs -notcontains '--no-run') {
    $code = Invoke-Cargo ($CargoArgs + '--no-run' + '--message-format=json-render-diagnostics')
    if ($code -ne 0) {
        Get-Content $err
        "cargo exit code: $code"
        exit $code
    }
    $tests = Get-Content $out | ConvertFrom-Json | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.profile.test -and $_.executable }
    $started = foreach ($test in $tests) {
        # No arguments, as cargo starts them; the harness refuses this value and exits before any test.
        $start = New-Object System.Diagnostics.ProcessStartInfo $test.executable
        $start.EnvironmentVariables['RUST_TEST_THREADS'] = '0'
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        [System.Diagnostics.Process]::Start($start)
    }
    foreach ($process in $started) { $process.WaitForExit() }
}
$code = Invoke-Cargo $CargoArgs
Get-Content $err
Get-Content $out
"cargo exit code: $code"
exit $code
