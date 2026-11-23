using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;

namespace FortiVPNManager;

/// <summary>
/// Windows notifications, for problems and updates the user should see while Settings is closed
/// (components/os-integration/macos/UserNotifier.swift). The text comes from the UI's sentences;
/// clicking a notification or its button calls the constructor's onOpen with its identifier and
/// whether the button was clicked, or starts the app when it is not running.
/// </summary>
internal sealed partial class Notifier : IDisposable
{
    private const string Group = "FortiVPNManager";
    private const string IdentifierArgument = "identifier";
    private const string ButtonArgument = "button";

    public Notifier(Action<string?, bool> onOpen)
    {
        AppNotificationManager.Default.NotificationInvoked += (_, args) => onOpen(Identifier(args), FromButton(args));
        AppNotificationManager.Default.Register();
    }

    public static string? Identifier(AppNotificationActivatedEventArgs args) =>
        args.Arguments.TryGetValue(IdentifierArgument, out string? identifier) ? identifier : null;

    public static bool FromButton(AppNotificationActivatedEventArgs args) => args.Arguments.ContainsKey(ButtonArgument);

    /// <summary>Replaces an earlier notification with the same <paramref name="identifier"/>.</summary>
    public static void Post(string identifier, string title, string body, string? button)
    {
        AppNotificationBuilder builder = new AppNotificationBuilder()
            .AddArgument(IdentifierArgument, identifier)
            .AddText(title)
            .AddText(body);
        if (button is not null)
        {
            _ = builder.AddButton(new AppNotificationButton(button)
                .AddArgument(IdentifierArgument, identifier)
                .AddArgument(ButtonArgument, ButtonArgument));
        }
        AppNotification notification = builder.BuildNotification();
        notification.Tag = identifier;
        notification.Group = Group;
        AppNotificationManager.Default.Show(notification);
    }

    public void Dispose() => AppNotificationManager.Default.Unregister();
}
