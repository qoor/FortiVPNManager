# Builds the Windows app (with its own .NET) for release into .build\windows\FortiVPNManager, with the
# Helper, the runtime DLL and the OpenConnect libraries built beforehand and the built-in Google OAuth
# client when there is one, and from that folder the installer, .build\windows\FortiVPNManager-<version>.msi.
# OpenConnect comes as a zip that packaging/openconnect/build.sh builds on a Mac.
# It runs no cargo: scripts/msi-windows.sh builds the Rust parts on a Mac, as the build PC's security
# software holds every new executable, build scripts included, for minutes on its first run.
# ASCII only.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\build.ps1 -Build <build> -RustOutput <folder> -OpenConnect <zip>
# <build> is yyyyMMddHHmmss, which the app takes and the Helper was built with (FVM_BUILD); <folder>
# holds fvm-helper.exe and fvm_runtime.dll from release builds for x86_64-pc-windows-msvc:
#   cargo build --release --package fvm-helper-windows
#   cargo rustc --release --package fvm-runtime --lib --crate-type cdylib
[CmdletBinding(DefaultParameterSetName = 'Build')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Prebuilt')] [string] $Build,
    [Parameter(Mandatory, ParameterSetName = 'Prebuilt')] [string] $RustOutput,
    [Parameter(Mandatory, ParameterSetName = 'Prebuilt')] [string] $OpenConnect
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$out = Join-Path $root '.build\windows\FortiVPNManager'
$msiBuild = Join-Path $root '.build\windows\msi'
# The app's version, which the app and the installer carry and which names the installer
# (docs\DESIGN.md 4.4).
$version = (Get-Content (Join-Path $root 'VERSION') -TotalCount 1).Trim()

function Invoke-Dotnet([string[]] $arguments) {
    $ErrorActionPreference = 'Continue'
    & dotnet @arguments
    if ($LASTEXITCODE -ne 0) { throw "dotnet $($arguments[0]) failed with exit code $LASTEXITCODE" }
}

function Invoke-Cargo([string[]] $arguments) {
    $ErrorActionPreference = 'Continue'
    & cargo @arguments --message-format=json-render-diagnostics
    if ($LASTEXITCODE -ne 0) { throw "cargo $($arguments[0]) failed with exit code $LASTEXITCODE" }
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

if ($PSCmdlet.ParameterSetName -eq 'Build') {
    $Build = (Get-Date).ToString('yyyyMMddHHmmss')
    $env:FVM_BUILD = $Build
    $messages = Invoke-Cargo @('build', '--release', '--package', 'fvm-helper-windows')
    Remove-Item Env:FVM_BUILD
    [void](Invoke-Cargo @('rustc', '--release', '--package', 'fvm-runtime', '--lib', '--crate-type', 'cdylib'))
    $RustOutput = Join-Path $root 'target\release'
    $adapter = $messages | Where-Object { $_ -match '"reason":"build-script-executed"' } | ForEach-Object { $_ | ConvertFrom-Json } |
        Where-Object { $_.package_id -match '#fvm-backend-adapter@' }
    $libraries = Join-Path $adapter.out_dir 'bin'
} else {
    $libraries = Join-Path $root '.build\windows\openconnect'
    if (Test-Path $libraries) { Remove-Item $libraries -Recurse -Force }
    Expand-Archive -Path $OpenConnect -DestinationPath $libraries
}

if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory $out | Out-Null
# The app takes the build as its informational version, which it reports and checks against the
# Helper's (the Helper carries the same, so the app sends a secret only to its own build), and the
# version as its file version.
$app = Join-Path $root 'components\os-integration\windows\App\FortiVPNManager.App.csproj'
Invoke-Dotnet @('publish', $app, '-c', 'Release', '-p:Platform=x64', "-p:Version=$version", "-p:InformationalVersion=$Build", '-o', $out, '--nologo')
Copy-Item (Join-Path $RustOutput 'fvm-helper.exe') $out
Copy-Item (Join-Path $RustOutput 'fvm_runtime.dll') $out
$taken = @(Get-ChildItem $libraries -Name | Where-Object { Test-Path -LiteralPath (Join-Path $out $_) })
if ($taken.Count -gt 0) { throw "the OpenConnect libraries have files the app folder has already: $($taken -join ', ')" }
Copy-Item (Join-Path $libraries '*') $out
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
# The app's symbols (its native code's, tens of MB) stay out of the installer (Package.wxs) and next to
# it, for crash dumps of this build; a debugger looks them up as FortiVPNManager.pdb. Copied first, so a
# failed copy leaves no installer without its Korean transform under the final name.
Copy-Item (Join-Path $out 'FortiVPNManager.pdb') ([IO.Path]::ChangeExtension($msi, '.pdb')) -Force
Copy-Item (Join-Path $msiBuild 'en-US\FortiVPNManager.msi') $msi -Force
Add-LanguageTransform $msi (Join-Path $msiBuild 'ko-KR\FortiVPNManager.msi') 1042
Write-Output "build: $Build, installer $version in $msi"
