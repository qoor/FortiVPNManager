using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.Ui;

public sealed partial class TrayIcon
{
    internal static string IconFile(ConnectionIcon state) => state switch
    {
        ConnectionIcon.Connected => "AppIcon.ico",
        ConnectionIcon.Disconnected => "TrayDisconnected.ico",
        ConnectionIcon.InProgress => "TrayInProgress.ico",
        ConnectionIcon.Error => "TrayError.ico",
        _ => throw new InvalidOperationException($"unknown connection icon {state}"),
    };
}
