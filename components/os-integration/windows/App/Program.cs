using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;

namespace FortiVPNManager.App;

/// <summary>
/// The entry: one copy per user. Starting the app again, from the Start menu or a notification,
/// hands the start to the running copy, which shows its settings window (as reopening the app does
/// on macOS), and ends.
/// </summary>
public static class Program
{
    private const string InstanceKey = "FortiVPNManager";

    [STAThread]
    public static void Main()
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        var main = AppInstance.FindOrRegisterForKey(InstanceKey);
        if (!main.IsCurrent)
        {
            var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
            // The handover is a call to the other process, made off this single-threaded thread.
            Task.Run(() => main.RedirectActivationToAsync(activation).AsTask()).Wait();
            return;
        }
        main.Activated += (_, args) => App.Running?.Reopened(args);
        Application.Start(start =>
        {
            SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            _ = new App();
        });
    }
}
