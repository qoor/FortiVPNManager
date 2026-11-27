using System.Diagnostics.CodeAnalysis;
using FortiVPNManager.user.ui.windows;

[assembly: SuppressMessage("Security", "CA5392:Use DefaultDllImportSearchPaths attribute for P/Invokes", Scope = "member", Target = "~M:Microsoft.Windows.Foundation.UndockedRegFreeWinRTCS.NativeMethods.WindowsAppRuntime_EnsureIsLoaded~System.Int32", Justification = "The Windows App SDK package adds this source to the build; it cannot be changed here.")]

namespace FortiVPNManager.user.app.windows;

internal static class Program
{
    public static void Main()
    {
        StartService();
        AppUI ui = new();
        ui.Initialize();
        RunUILoop(ui);
        StopService();
    }

    private static void StartService() =>
        throw new NotImplementedException("start the app's main service");

    private static void RunUILoop(AppUI ui) =>
        throw new NotImplementedException("run the UI loop of the OS on this thread");
    // The loop ends when the app UI is told that the app can quit.

    private static void StopService() =>
        throw new NotImplementedException("stop the app's main service");
}
