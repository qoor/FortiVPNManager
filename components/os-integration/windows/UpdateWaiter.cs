using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

internal sealed class UpdateWaiter
{
    private const string Cmd = @"C:\Windows\System32\cmd.exe";
    private const string App = "FortiVPNManager.exe";
    private const string Installer = "msiexec.exe";

    private Process? _running;

    public PortResponse Start()
    {
        try
        {
            _running = Process.Start(StartInfo(
                Environment.ProcessId,
                Path.Combine(ServiceRegistration.InstallFolder, App),
                App));
            return new PortResponseUpdateWaiterStarted();
        }
        catch (Win32Exception error)
        {
            return new PortResponseUpdateWaiterFailed(error.Message);
        }
    }

    public void Release(uint installerProcessId)
    {
        using Process? waiter = _running;
        _running = null;
        if (waiter is null || waiter.HasExited)
        {
            SystemLog.Write(LogLevel.Error, LogCategory.Update, "the update waiter was not running when given the installer");
            return;
        }
        try
        {
            waiter.StandardInput.WriteLine(installerProcessId.ToString(CultureInfo.InvariantCulture));
            waiter.StandardInput.Close();
        }
        catch (IOException error)
        {
            SystemLog.Write(LogLevel.Error, LogCategory.Update, $"the update waiter did not take the installer: {error.Message}");
        }
    }

    public void Stop()
    {
        using Process? waiter = _running;
        _running = null;
        if (waiter is null || waiter.HasExited)
        {
            SystemLog.Write(LogLevel.Error, LogCategory.Update, "the update waiter was not running when asked to stop");
            return;
        }
        waiter.Kill();
    }

    internal static ProcessStartInfo StartInfo(int appProcessId, string app, string appImage) => new(Cmd)
    {
        Arguments = "/d /q /v:on /c set \"fvm_installer=\" & (set /p fvm_installer=)& if defined fvm_installer for /l %i in (0,0,1) do @(("
            + Running(appProcessId.ToString(CultureInfo.InvariantCulture), appImage)
            + " || "
            + Running("!fvm_installer!", Installer)
            + $" || (start \"\" \"{app}\" & exit)) & ping -n 2 127.0.0.1 >nul)",
        UseShellExecute = false,
        CreateNoWindow = true,
        RedirectStandardInput = true,
        WorkingDirectory = Environment.SystemDirectory,
    };

    private static string Running(string processId, string image) =>
        $"tasklist /nh /fo csv /fi \"PID eq {processId}\" /fi \"IMAGENAME eq {image}\" | find \"\"\"{processId}\"\"\" >nul";
}
