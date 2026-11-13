using System.Security;
using System.ServiceProcess;
using Microsoft.Win32;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// The Helper service, which the install script registers, and the app's login item, a value under
/// the user's Run key. Only the installed copy registers the login item, so the system never starts a
/// build somewhere else.
/// </summary>
internal static class ServiceRegistration
{
    /// <summary>The service's name (components/helper-entry/windows/src/service.rs).</summary>
    public const string HelperService = "FortiVPNManagerHelper";

    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string RunValue = "FortiVPNManager";

    /// <summary>Where the install script puts the app.</summary>
    public static string InstallFolder =>
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "FortiVPNManager");

    public static bool IsInstalledCopy(string executable) =>
        string.Equals(
            Path.GetDirectoryName(Path.GetFullPath(executable)),
            InstallFolder,
            StringComparison.OrdinalIgnoreCase);

    /// <summary>Registered once installed; whether it runs and answers is the build check's to find.</summary>
    public static PortResponse HelperStatus()
    {
        try
        {
            using var service = new ServiceController(HelperService);
            _ = service.Status;
            return new PortResponse.Registration(RegistrationStatus.Enabled);
        }
        catch (InvalidOperationException)
        {
            return new PortResponse.Registration(RegistrationStatus.NotRegistered);
        }
    }

    public static PortResponse RegisterHelper() =>
        new PortResponse.RegistrationFailed(
            new RegistrationError.Failed("the install script registers the Helper service"));

    public static PortResponse LoginItemStatus()
    {
        using var key = Registry.CurrentUser.OpenSubKey(RunKey);
        var registered = key?.GetValue(RunValue) is string;
        return new PortResponse.Registration(registered ? RegistrationStatus.Enabled : RegistrationStatus.NotRegistered);
    }

    public static PortResponse SetLoginItem(bool enabled)
    {
        var executable = Environment.ProcessPath ?? "";
        if (!IsInstalledCopy(executable))
        {
            return new PortResponse.RegistrationFailed(new RegistrationError.NotInstalledCopy());
        }
        try
        {
            using var key = Registry.CurrentUser.CreateSubKey(RunKey);
            if (enabled)
            {
                key.SetValue(RunValue, $"\"{executable}\"");
            }
            else
            {
                key.DeleteValue(RunValue, throwOnMissingValue: false);
            }
            return new PortResponse.Registered();
        }
        catch (Exception error) when (error is UnauthorizedAccessException or IOException or SecurityException)
        {
            return new PortResponse.RegistrationFailed(new RegistrationError.Failed(error.Message));
        }
    }
}
