namespace FortiVPNManager.user.ui.windows.Notification;

/// <summary>The notifications of the OS: it tells the user what the app has to tell.</summary>
internal sealed class NotificationUI
{
    /// <summary>Makes what the UI needs before it can notify.</summary>
    public void Initialize() =>
        throw new NotImplementedException("get ready to post notifications of the OS");

    /// <summary>Shows what changed.</summary>
    public void Show(UIUpdate update) =>
        throw new NotImplementedException("post the notice as a notification of the OS");
}
