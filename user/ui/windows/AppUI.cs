using FortiVPNManager.user.ui.windows.MenuBar;
using FortiVPNManager.user.ui.windows.Notification;
using FortiVPNManager.user.ui.windows.Settings;

namespace FortiVPNManager.user.ui.windows;

/// <summary>What the main service has for the UI to show.</summary>
internal readonly record struct UIUpdate;

/// <summary>What the user did in the UI.</summary>
internal readonly record struct UserAction;

/// <summary>The app's UI as a whole: it makes the three UIs and stands between them and the main service.</summary>
internal sealed class AppUI
{
    private readonly MenuBarUI _menuBar;
    private readonly SettingsUI _settings;
    private readonly NotificationUI _notification;

    /// <summary>Creates the app's UI with its three UIs.</summary>
    public AppUI()
    {
        _menuBar = new MenuBarUI(this);
        _settings = new SettingsUI(this);
        _notification = new NotificationUI();
    }

    /// <summary>Makes what the UI needs before it can show anything.</summary>
    public void Initialize()
    {
        _menuBar.Initialize();
        _settings.Initialize();
        _notification.Initialize();
    }

    /// <summary>Takes what the main service has for the UI. It can be called from any thread.</summary>
    public void Push(UIUpdate update) =>
        throw new NotImplementedException("hand what the main service has to the UI it concerns");
    // 1. Move over to the UI thread.
    // 2. The connection and updates go to the notification area UI and the settings UI.
    // 3. The settings, the result of a load, the accounts, the exception routes and the log
    //    lines go to the settings UI.
    // 4. A notice goes to the notification UI.
    // 5. Being told that the app can quit ends the UI loop.

    /// <summary>Sends what the user did to the main service.</summary>
    public void Send(UserAction action) =>
        throw new NotImplementedException("hand what the user did to the main service");

    /// <summary>Shows the settings window. The notification area UI asks for it.</summary>
    public void OpenSettings() =>
        throw new NotImplementedException("have the settings UI show its window");
}
