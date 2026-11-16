# Run by the installer as the system account, right after it asks the Helper to stop and before it
# replaces or removes files, so that none of them is in use; Windows Installer would otherwise set a
# file in use aside and ask for a restart. The app had its end-session message and a few seconds to
# quit, so a copy still running is ended now. The Helper may take up to 30 s to end its run and exits
# only after it reports itself stopped; it, and anything it started from the folder, gets twice that
# before it is ended too. Windows PowerShell 5.1, ASCII only.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$folder = (Join-Path $env:ProgramFiles 'FortiVPNManager') + '\'
# components/helper/windows/src/service.rs
$service = 'FortiVPNManagerHelper'
$deadline = (Get-Date).AddSeconds(60)

function Get-Running {
    @(Get-CimInstance Win32_Process | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($folder, [StringComparison]::OrdinalIgnoreCase)
    })
}

function Stop-Running($processes) {
    foreach ($process in $processes) {
        Write-Output "ending $($process.ExecutablePath) ($($process.ProcessId))"
        Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
        Wait-Process -Id $process.ProcessId -Timeout 10 -ErrorAction SilentlyContinue
    }
}

Stop-Running (Get-Running | Where-Object { $_.Name -eq 'FortiVPNManager.exe' })
while (($running = @(Get-Running)).Count -gt 0 -and (Get-Date) -lt $deadline) {
    # A Helper still starting refuses a stop, so it is asked again once it runs; ended at the limit
    # instead, it would be started again by Windows' recovery 5 s later.
    if ((Get-Service $service -ErrorAction SilentlyContinue).Status -eq 'Running') {
        Stop-Service $service -NoWait -ErrorAction SilentlyContinue
    }
    Start-Sleep -Milliseconds 500
}
Stop-Running $running
