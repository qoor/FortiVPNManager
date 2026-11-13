# Puts the build that build.ps1 made in Program Files and registers the Helper service. Run it again
# to reinstall: the service stops, which ends a VPN connection it runs, the files are replaced and
# the service starts again. Run from an elevated PowerShell. ASCII only.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\install.ps1 [-User <account or SID>]
# Only one user may open the Helper's pipe: the one signed in to Windows unless -User names another.
# Not the account that runs this, since a standard user's elevation runs it as an administrator.
param([string] $User)
$ErrorActionPreference = 'Stop'
$built = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) '.build\windows\FortiVPNManager'
# components/helper-entry/windows/src/service.rs
$service = 'FortiVPNManagerHelper'
$helperSource = 'FortiVPNManager Helper'
# components/os-integration/windows/SystemLog.cs; the app cannot register it without administrator rights.
$appSource = 'FortiVPNManager'
# components/backend-adapter/src/openconnect/platform.rs
$client = 'C:\Program Files\OpenConnect\openconnect.exe'
# components/os-integration/windows/ServiceRegistration.cs
$folder = Join-Path $env:ProgramFiles 'FortiVPNManager'
$new = "$folder.new"
$old = "$folder.old"
# How long Windows waits, when it shuts down, for the Helper to end the run and log out, while the
# network still runs. The Helper asks as long for a stop (STOP_WAIT in service.rs).
$preshutdownTimeout = 30000
$start = Get-Date

function Note([string] $text) { Write-Output "install: $text" }

# Stops the Helper and waits for its process to end too: it reports Stopped just before it exits,
# and its executable is in use until then. A start under way, such as a restart after a failure,
# takes no stop, so it is let finish first.
function Stop-Helper {
    $deadline = (Get-Date).AddSeconds(90)
    $process = 0
    while ($true) {
        $status = Get-CimInstance Win32_Service -Filter "Name = '$service'"
        if ($status.ProcessId -ne 0) { $process = $status.ProcessId }
        if ($status.State -eq 'Stopped') { break }
        if ($status.State -eq 'Running') { & sc.exe stop $service | Out-Null }
        if ((Get-Date) -gt $deadline) { throw "the Helper did not stop within 90 s ($($status.State))" }
        Start-Sleep -Milliseconds 500
    }
    if ($process -ne 0) { Wait-Process -Id $process -Timeout 30 -ErrorAction SilentlyContinue }
}

# The service API sc.exe has no command for.
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class ServicePreshutdown
{
    private const uint ManagerConnect = 0x1;
    private const uint QueryConfig = 0x1;
    private const uint ChangeConfig = 0x2;
    private const uint PreshutdownInfo = 7;

    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern IntPtr OpenSCManagerW(string machine, string database, uint access);

    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern IntPtr OpenServiceW(IntPtr manager, string name, uint access);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool ChangeServiceConfig2W(IntPtr service, uint level, ref uint timeout);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool QueryServiceConfig2W(IntPtr service, uint level, ref uint timeout, uint size, out uint needed);

    [DllImport("advapi32.dll")]
    private static extern bool CloseServiceHandle(IntPtr handle);

    /// <summary>Sets the service's preshutdown timeout and answers it as read back.</summary>
    public static uint Set(string name, uint milliseconds)
    {
        IntPtr manager = OpenSCManagerW(null, null, ManagerConnect);
        if (manager == IntPtr.Zero) throw new Win32Exception();
        try
        {
            IntPtr service = OpenServiceW(manager, name, QueryConfig | ChangeConfig);
            if (service == IntPtr.Zero) throw new Win32Exception();
            try
            {
                if (!ChangeServiceConfig2W(service, PreshutdownInfo, ref milliseconds)) throw new Win32Exception();
                uint read = 0;
                uint needed;
                if (!QueryServiceConfig2W(service, PreshutdownInfo, ref read, 4, out needed)) throw new Win32Exception();
                return read;
            }
            finally { CloseServiceHandle(service); }
        }
        finally { CloseServiceHandle(manager); }
    }
}
'@

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not ([Security.Principal.WindowsPrincipal] $identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'run this from an elevated PowerShell'
}
if (-not (Test-Path (Join-Path $built 'fvm-helper.exe'))) {
    throw "no build in $built; run packaging\windows\build.ps1 first, as yourself without elevation"
}

# The user signed in to Windows is the owner of its desktop, explorer.exe.
if ($User -match '^S-1-') {
    $sid = New-Object Security.Principal.SecurityIdentifier $User
} elseif ($User) {
    $sid = (New-Object Security.Principal.NTAccount $User).Translate([Security.Principal.SecurityIdentifier])
} else {
    $owners = @(Get-CimInstance Win32_Process -Filter "Name = 'explorer.exe'" |
        ForEach-Object { (Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid).Sid } |
        Where-Object { $_ } | Sort-Object -Unique)
    if ($owners.Count -eq 0) { throw 'nobody is signed in to Windows; name the user with -User' }
    if ($owners.Count -gt 1) { throw "several users are signed in ($($owners -join ', ')); name one with -User" }
    $sid = New-Object Security.Principal.SecurityIdentifier $owners[0]
}
try { $account = $sid.Translate([Security.Principal.NTAccount]).Value } catch { $account = $sid.Value }
Note "the Helper's pipe is for $account ($($sid.Value))"
if (-not (Test-Path $client)) {
    Note "OpenConnect is not installed ($client is missing); the app cannot connect until it is"
}

foreach ($leftover in $new, $old) {
    if (Test-Path $leftover) {
        Remove-Item $leftover -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path $leftover) {
            throw "$leftover from an earlier install is still in use, perhaps by the antivirus program; run this again in a moment"
        }
    }
}
Copy-Item $built $new -Recurse

$existing = Get-CimInstance Win32_Service -Filter "Name = '$service'"
$wasRunning = $existing -and $existing.State -ne 'Stopped'
if ($existing) {
    Note 'stopping the Helper'
    Stop-Helper
}
# Each folder moves in one rename, which fails whole while a program uses a file in it, such as the
# app. Move-Item would then move the files one by one and leave the ones in use behind. On a failure
# the Helper starts again first, so tidying up that fails too cannot leave it stopped.
try {
    if (Test-Path $folder) { [IO.Directory]::Move($folder, $old) }
} catch {
    # The base exception has the reason; PowerShell's wrapper names the method call.
    $reason = $_.Exception.GetBaseException().Message
    if ($wasRunning) { & sc.exe start $service | Out-Null }
    Remove-Item $new -Recurse -Force -ErrorAction SilentlyContinue
    throw "cannot replace ${folder}: $reason If FortiVPNManager runs, quit it and run this again."
}
try {
    [IO.Directory]::Move($new, $folder)
} catch {
    $reason = "cannot put the new build in place: $($_.Exception.GetBaseException().Message)"
    if (Test-Path $old) {
        try {
            [IO.Directory]::Move($old, $folder)
            $reason += ' The previous build is back.'
        } catch {
            $reason += " The previous build is in $old, and the Helper cannot start until it is moved back to $folder."
        }
    }
    if ($wasRunning) { & sc.exe start $service | Out-Null }
    Remove-Item $new -Recurse -Force -ErrorAction SilentlyContinue
    throw $reason
}

$starting = $false
try {
    # The path is quoted, as it has spaces; New-Service and Win32_Service take it whole, unlike
    # sc.exe through Windows PowerShell's argument passing.
    $command = "`"$folder\fvm-helper.exe`" --allowed-user $($sid.Value)"
    if ($existing) {
        $changed = Invoke-CimMethod -InputObject $existing -MethodName Change -Arguments @{ PathName = $command; StartMode = 'Automatic' }
        if ($changed.ReturnValue -ne 0) { throw "changing the Helper service failed with code $($changed.ReturnValue)" }
    } else {
        New-Service -Name $service -BinaryPathName $command -DisplayName 'FortiVPNManager Helper' -StartupType Automatic | Out-Null
    }
    # Restarted after every failure, a crash or a failed start: at 5 s, 5 s, then every minute, as
    # Windows repeats the last action; the count starts over after a day without one. The VPN needs
    # the Helper, and a cause such as another program holding the pipe's name may go away.
    & sc.exe failure $service reset= 86400 actions= restart/5000/restart/5000/restart/60000 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "setting the Helper's restarts after a crash failed with exit code $LASTEXITCODE" }
    & sc.exe failureflag $service 1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "setting the Helper's restarts after a failed start failed with exit code $LASTEXITCODE" }
    $read = [ServicePreshutdown]::Set($service, $preshutdownTimeout)
    Note "Windows waits $($read / 1000) s for the Helper when it shuts down"
    if (-not [Diagnostics.EventLog]::SourceExists($appSource)) {
        New-EventLog -LogName Application -Source $appSource
        Note "registered the app's event log source $appSource"
    }

    # The antivirus program may hold a new executable for minutes, past the 30 s Windows gives a
    # service to start; a later start goes through.
    Note 'starting the Helper'
    $starting = $true
    $deadline = (Get-Date).AddMinutes(10)
    do {
        & sc.exe start $service | Out-Null
        do {
            Start-Sleep -Seconds 2
            $state = (Get-Service $service).Status
        } while ($state -eq 'StartPending')
        if ($state -ne 'Running' -and (Get-Date) -lt $deadline) { Note "not started yet ($state); trying again" }
    } while ($state -ne 'Running' -and (Get-Date) -lt $deadline)
    if ($state -ne 'Running') { throw 'the Helper did not start within 10 minutes; the Application event log may say why' }
} catch {
    $reason = $_.Exception.GetBaseException().Message
    # The files are in place, so a setting that failed does not leave the VPN without its Helper.
    if (-not $starting) {
        & sc.exe start $service | Out-Null
        $reason += ' The Helper was started with the new build all the same.'
    }
    if (Test-Path $old) { $reason += " The previous build is in $old until the next install." }
    throw $reason
}
# The install is done; a file the antivirus program still reads only delays the tidying up.
if (Test-Path $old) {
    Remove-Item $old -Recurse -Force -ErrorAction SilentlyContinue
    if (Test-Path $old) { Note "$old is still in use; the next install removes it" }
}
$started = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; ProviderName = $helperSource; StartTime = $start } -ErrorAction SilentlyContinue |
    Where-Object { $_.Message -like 'helper * running for *' } | Select-Object -First 1
if ($started) { Note $started.Message } else { Note 'the Helper runs' }
