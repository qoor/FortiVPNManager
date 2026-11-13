# Builds the Windows Helper and the runtime DLL for release into .build\windows\FortiVPNManager, the
# folder install.ps1 puts in Program Files, with the built-in Google OAuth client when there is one.
# Run it as yourself, without elevation: Rust is installed per user, so an administrator account may
# have none. ASCII only.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\build.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$out = Join-Path $root '.build\windows\FortiVPNManager'
# The Helper and the app carry the same build, which the app checks before it sends a secret. The
# format is the macOS build's.
$build = Get-Date -Format 'yyyyMMddHHmmss'

function Invoke-Cargo([string[]] $arguments) {
    # cargo reports progress on stderr, which Windows PowerShell may turn into errors under Stop.
    $ErrorActionPreference = 'Continue'
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw "cargo $($arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

Push-Location $root
try {
    $env:FVM_BUILD = $build
    Invoke-Cargo @('build', '--release', '--package', 'fvm-helper-entry-windows')
    Invoke-Cargo @('rustc', '--release', '--package', 'fvm-runtime', '--lib', '--crate-type', 'cdylib')
} finally {
    Remove-Item Env:FVM_BUILD -ErrorAction SilentlyContinue
    Pop-Location
}
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory $out | Out-Null
Copy-Item (Join-Path $root 'target\release\fvm-helper.exe') $out
Copy-Item (Join-Path $root 'target\release\fvm_runtime.dll') $out
# The app's built-in Google OAuth client, the Desktop-app client JSON that the macOS build uses too,
# gitignored as it has the client secret. Without it the app still works; users then import a client
# JSON in Settings.
$oauthClient = Join-Path $root 'packaging\GoogleOAuthClient.json'
if (Test-Path $oauthClient -PathType Leaf) {
    Copy-Item $oauthClient $out
} else {
    Write-Warning 'packaging\GoogleOAuthClient.json not found; building without a built-in Google OAuth client'
}
Write-Output "build: $build in $out"
