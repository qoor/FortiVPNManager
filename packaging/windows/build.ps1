# Builds the Windows app (with its own .NET), the Helper and the runtime DLL for release into
# .build\windows\FortiVPNManager, with the built-in Google OAuth client when there is one, and from that
# folder and OpenConnect the installer, .build\windows\FortiVPNManager-<version>.msi. Run it as
# yourself, without elevation: Rust is installed per user, so an administrator account may have none.
# ASCII only.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\build.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$out = Join-Path $root '.build\windows\FortiVPNManager'
$msiBuild = Join-Path $root '.build\windows\msi'
# OpenConnect 9.21 as its Windows installer (openconnect-installer-MinGW64-GnuTLS.exe) puts it in
# C:\Program Files\OpenConnect, but for "Uninstall OpenConnect.exe" and "Online Documentation.url";
# the installer takes every file here.
$openConnect = Join-Path $PSScriptRoot 'OpenConnect'
if (-not (Test-Path (Join-Path $openConnect 'openconnect.exe') -PathType Leaf)) {
    throw "no openconnect.exe in ${openConnect}: install OpenConnect 9.21 with its Windows installer (openconnect-installer-MinGW64-GnuTLS.exe) and copy what it puts in C:\Program Files\OpenConnect there, except 'Uninstall OpenConnect.exe' and 'Online Documentation.url'"
}
$now = Get-Date
# The Helper and the app carry the same build, which the app checks before it sends a secret. The
# format is the macOS build's.
$build = $now.ToString('yyyyMMddHHmmss')
# The installer's version, from the same moment: year, month and the minute of the month, which keeps
# the third field under Windows Installer's limit of 65535.
$version = '{0:yy}.{0:MM}.{1}' -f $now, (($now.Day - 1) * 1440 + $now.Hour * 60 + $now.Minute)

function Invoke-Dotnet([string[]] $arguments) {
    $ErrorActionPreference = 'Continue'
    & dotnet @arguments
    if ($LASTEXITCODE -ne 0) { throw "dotnet $($arguments[0]) failed with exit code $LASTEXITCODE" }
}

function Invoke-Cargo([string[]] $arguments) {
    # cargo reports progress on stderr, which Windows PowerShell may turn into errors under Stop.
    $ErrorActionPreference = 'Continue'
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw "cargo $($arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

# A script the installer runs, as a PowerShell command, which execution policy does not apply to.
function Get-EncodedScript([string] $name) {
    [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes([IO.File]::ReadAllText((Join-Path $PSScriptRoot $name))))
}

# Embeds the transform from the English package to the localized one as a sub-storage named after its
# language ID and adds that ID to the package's Template summary property. Windows Installer then
# applies it by itself where the display language is that one: long-standing but undocumented
# behaviour (https://www.installsite.org/pages/en/msi/articles/embeddedlang/).
function Add-LanguageTransform([string] $package, [string] $localized, [int] $language) {
    # The automation objects carry no type information PowerShell can call them through, and they take
    # objects only without PowerShell's wrapper.
    function Invoke([object] $target, [string] $name, [Reflection.BindingFlags] $kind, [object[]] $arguments) {
        $object = $target.psobject.BaseObject
        [object[]] $plain = foreach ($argument in $arguments) { $argument.psobject.BaseObject }
        $object.GetType().InvokeMember($name, $kind, $null, $object, $plain)
    }
    $transform = Join-Path $msiBuild "$language.mst"
    $installer = New-Object -ComObject WindowsInstaller.Installer
    # Transacted and read-only.
    $base = Invoke $installer 'OpenDatabase' 'InvokeMethod' @($package, 1)
    $target = Invoke $installer 'OpenDatabase' 'InvokeMethod' @($localized, 0)
    [void](Invoke $target 'GenerateTransform' 'InvokeMethod' @($base, $transform))
    # Windows Installer applies the transform with these settings: the code page may change, as each
    # language has its own, and nothing is validated, as it goes only with the package it was made from.
    [void](Invoke $target 'CreateTransformSummaryInfo' 'InvokeMethod' @($base, $transform, 0x20, 0))
    $view = Invoke $base 'OpenView' 'InvokeMethod' @('SELECT `Name`, `Data` FROM `_Storages`')
    [void](Invoke $view 'Execute' 'InvokeMethod' @())
    $record = Invoke $installer 'CreateRecord' 'InvokeMethod' @(2)
    [void](Invoke $record 'StringData' 'SetProperty' @(1, [string] $language))
    [void](Invoke $record 'SetStream' 'InvokeMethod' @(2, $transform))
    # msiViewModifyAssign
    [void](Invoke $view 'Modify' 'InvokeMethod' @(3, $record))
    [void](Invoke $view 'Close' 'InvokeMethod' @())
    $summary = Invoke $base 'SummaryInformation' 'GetProperty' @(1)
    # PID_TEMPLATE, such as x64;1033
    $template = Invoke $summary 'Property' 'GetProperty' @(7)
    [void](Invoke $summary 'Property' 'SetProperty' @(7, "$template,$language"))
    [void](Invoke $summary 'Persist' 'InvokeMethod' @())
    [void](Invoke $base 'Commit' 'InvokeMethod' @())
    foreach ($object in $summary, $record, $view, $target, $base, $installer) {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($object)
    }
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
# The app takes the build as its informational version, which it reports and checks against the Helper's.
$app = Join-Path $root 'components\os-integration\windows\App\FortiVPNManager.App.csproj'
Invoke-Dotnet @('publish', $app, '-c', 'Release', '-p:Platform=x64', "-p:InformationalVersion=$build", '-o', $out, '--nologo')
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

# One package per culture, sharing the product code, so that the Korean one can become a transform.
if (Test-Path $msiBuild) { Remove-Item $msiBuild -Recurse -Force }
$productCode = [Guid]::NewGuid().ToString('B').ToUpperInvariant()
Invoke-Dotnet @('build', (Join-Path $PSScriptRoot 'FortiVPNManager.Installer.wixproj'), '-c', 'Release',
    "-p:OutputPath=$msiBuild", "-p:ProductVersion=$version", "-p:ProductCode=$productCode",
    "-p:CloseRunningScript=$(Get-EncodedScript 'CloseRunning.ps1')",
    "-p:UninstallCleanupScript=$(Get-EncodedScript 'UninstallCleanup.ps1')", '--nologo')
$msi = Join-Path $root ".build\windows\FortiVPNManager-$version.msi"
Copy-Item (Join-Path $msiBuild 'en-US\FortiVPNManager.msi') $msi -Force
Add-LanguageTransform $msi (Join-Path $msiBuild 'ko-KR\FortiVPNManager.msi') 1042
Write-Output "build: $build, installer $version in $msi"
