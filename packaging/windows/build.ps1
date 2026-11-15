# Builds the Windows app (with its own .NET) for release into .build\windows\FortiVPNManager, with the
# Helper and the runtime DLL built beforehand and the built-in Google OAuth client when there is one,
# and from that folder and OpenConnect the installer, .build\windows\FortiVPNManager-<version>.msi.
# OpenConnect comes as the program files of its official Windows installer, in a zip that this
# downloads once into .build\windows and checks against its SHA-256.
# It runs no cargo: scripts/msi-windows.sh builds the Rust parts on a Mac, as the build PC's security
# software holds every new executable, build scripts included, for minutes on its first run.
# ASCII only.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\build.ps1 -Build <build> -RustOutput <folder>
# <build> is yyyyMMddHHmmss, which the app takes and the Helper was built with (FVM_BUILD); <folder>
# holds fvm-helper.exe and fvm_runtime.dll from release builds for x86_64-pc-windows-msvc:
#   cargo build --release --package fvm-helper-entry-windows
#   cargo rustc --release --package fvm-runtime --lib --crate-type cdylib
param(
    [Parameter(Mandatory)] [string] $Build,
    [Parameter(Mandatory)] [string] $RustOutput
)
# The OpenConnect the installer ships: the program files of OpenConnect's official Windows installer, as
# a zip in this repository's release openconnect-<version>, so that building needs no tool that reads
# the installer. For a new version, extract the installer's program files once with 7-Zip, leaving out
# $PLUGINSDIR and the uninstaller it rebuilds ("Uninstall OpenConnect.exe"), upload them as
# openconnect-<version>-files.zip next to the installer in a release openconnect-<version>, and update
# these values and OpenConnect-licenses\LICENSES.txt.
$openConnectVersion = '9.21'
$openConnectUrl = "https://github.com/qoor/FortiVPNManager/releases/download/openconnect-$openConnectVersion/openconnect-$openConnectVersion-files.zip"
$openConnectSha256 = 'ae8f338ee69932f03f326447f36f94d461a103be2c3ce15bc495472977141fe5'

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$out = Join-Path $root '.build\windows\FortiVPNManager'
$msiBuild = Join-Path $root '.build\windows\msi'
# The MSI takes every file in this folder (OpenConnectFolder in FortiVPNManager.Installer.wixproj).
$openConnect = Join-Path $root '.build\windows\OpenConnect'
$openConnectZip = Join-Path $root ".build\windows\openconnect-$openConnectVersion-files.zip"
if (-not (Test-Path $openConnectZip -PathType Leaf)) {
    & curl.exe --fail --silent --show-error --location --create-dirs --output "$openConnectZip.download" $openConnectUrl
    if ($LASTEXITCODE -ne 0) { throw "downloading $openConnectUrl failed with curl exit code $LASTEXITCODE" }
    Move-Item "$openConnectZip.download" $openConnectZip
}
$hash = (Get-FileHash $openConnectZip -Algorithm SHA256).Hash
if ($hash -ne $openConnectSha256) {
    throw "$openConnectZip has SHA-256 $hash, not $openConnectSha256 of the OpenConnect $openConnectVersion files; delete it to download it again"
}
if (Test-Path $openConnect) { Remove-Item $openConnect -Recurse -Force }
Expand-Archive -Path $openConnectZip -DestinationPath $openConnect
# The Helper and the app carry the same build, which the app checks before it sends a secret. The
# format is the macOS build's.
$now = [datetime]::ParseExact($Build, 'yyyyMMddHHmmss', [Globalization.CultureInfo]::InvariantCulture)
# The installer's version, from the same moment: year, month and the minute of the month, which keeps
# the third field under Windows Installer's limit of 65535.
$version = '{0:yy}.{0:MM}.{1}' -f $now, (($now.Day - 1) * 1440 + $now.Hour * 60 + $now.Minute)

function Invoke-Dotnet([string[]] $arguments) {
    $ErrorActionPreference = 'Continue'
    & dotnet @arguments
    if ($LASTEXITCODE -ne 0) { throw "dotnet $($arguments[0]) failed with exit code $LASTEXITCODE" }
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

if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory $out | Out-Null
# The app takes the build as its informational version, which it reports and checks against the Helper's.
$app = Join-Path $root 'components\os-integration\windows\App\FortiVPNManager.App.csproj'
Invoke-Dotnet @('publish', $app, '-c', 'Release', '-p:Platform=x64', "-p:InformationalVersion=$Build", '-o', $out, '--nologo')
Copy-Item (Join-Path $RustOutput 'fvm-helper.exe') $out
Copy-Item (Join-Path $RustOutput 'fvm_runtime.dll') $out
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
