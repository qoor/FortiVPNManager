using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;

namespace FortiVPNManager.App;

/// <summary>
/// Windows notifications, for problems the user should see while Settings is closed
/// (components/os-integration/macos/UserNotifier.swift). The text comes from the UI's sentences;
/// clicking a notification calls <paramref name="onOpen"/>, or starts the app when it is not running.
/// </summary>
internal sealed class Notifier : IDisposable
{
    private const string Group = "FortiVPNManager";

    public Notifier(Action onOpen)
    {
        AppNotificationManager.Default.NotificationInvoked += (_, _) => onOpen();
        AppNotificationManager.Default.Register();
    }

    /// <summary>Replaces an earlier notification with the same <paramref name="identifier"/>.</summary>
    public void Post(string identifier, string title, string body)
    {
        var notification = new AppNotificationBuilder()
            .AddText(title)
            .AddText(body)
            .BuildNotification();
        notification.Tag = identifier;
        notification.Group = Group;
        AppNotificationManager.Default.Show(notification);
    }

    public void Dispose() => AppNotificationManager.Default.Unregister();
}
