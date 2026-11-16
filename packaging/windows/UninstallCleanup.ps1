# Run by the installer as the system account on a real uninstall only, never while an upgrade removes
# the old version, after the app is closed and the Helper stopped. For every user profile of the PC it
# removes the app's open-at-login entries; then it removes the Helper's work folder. Each user's own
# files (config.yaml, state.json, logs, credentials) stay. A user whose registry cannot be read is
# skipped and logged, and nothing here fails the uninstall. Windows PowerShell 5.1, ASCII only.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# components/os-integration/windows/ServiceRegistration.cs
$loginItem = 'FortiVPNManager'
$loginKeys = @(
    'Software\Microsoft\Windows\CurrentVersion\Run',
    'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
)
# components/helper/windows/src/service.rs
$helperFolder = 'C:\Windows\System32\config\systemprofile\AppData\Local\FortiVPNManager'

# reg.exe writes its messages in the system's language; only its exit code counts.
function Invoke-Reg([string[]] $arguments) {
    $ErrorActionPreference = 'Continue'
    & reg.exe @arguments 2>&1 | Out-Null
    $LASTEXITCODE
}

# A signed-in user's registry is loaded already; anyone else's is loaded from NTUSER.DAT for the change.
function Remove-LoginItem([string] $sid, [string] $profilePath) {
    $hive = $sid
    $loaded = Test-Path -LiteralPath "Registry::HKEY_USERS\$sid"
    if (-not $loaded) {
        $hive = "FortiVPNManager-$sid"
        $file = Join-Path $profilePath 'NTUSER.DAT'
        # reg load would create the file if it were missing.
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "no $file" }
        $code = Invoke-Reg @('load', "HKU\$hive", $file)
        if ($code -ne 0) { throw "cannot load $file (reg exit code $code)" }
    }
    try {
        foreach ($path in $loginKeys) {
            $key = [Microsoft.Win32.Registry]::Users.OpenSubKey("$hive\$path", $true)
            if ($null -eq $key) { continue }
            try {
                if ($null -ne $key.GetValue($loginItem)) {
                    $key.DeleteValue($loginItem)
                    Write-Output "${sid}: removed the login item from $path"
                }
            } finally {
                $key.Close()
            }
        }
    } finally {
        if (-not $loaded) {
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            $code = Invoke-Reg @('unload', "HKU\$hive")
            if ($code -ne 0) { Write-Output "${sid}: cannot unload the registry loaded from NTUSER.DAT (reg exit code $code)" }
        }
    }
}

foreach ($entry in Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList') {
    $sid = $entry.PSChildName
    # The system's own accounts, which run no app.
    if (@('S-1-5-18', 'S-1-5-19', 'S-1-5-20') -contains $sid) { continue }
    $profilePath = $entry.GetValue('ProfileImagePath')
    try {
        Remove-LoginItem $sid $profilePath
    } catch {
        Write-Output "${sid}: login item skipped: $($_.Exception.Message)"
    }
}
if (Test-Path -LiteralPath $helperFolder) {
    try {
        Remove-Item -LiteralPath $helperFolder -Recurse -Force
        Write-Output "removed $helperFolder"
    } catch {
        Write-Output "cannot remove ${helperFolder}: $($_.Exception.Message)"
    }
}
