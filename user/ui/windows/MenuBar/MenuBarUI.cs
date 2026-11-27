namespace FortiVPNManager.user.ui.windows.MenuBar;

/// <summary>The notification area icon and its menu: it shows the connection and takes the user's commands.</summary>
internal sealed class MenuBarUI
{
    /// <summary>Creates the UI with the app UI it sends what the user does through.</summary>
    public MenuBarUI(AppUI app) =>
        throw new NotImplementedException("create the notification area UI");

    /// <summary>Makes what the UI needs before it can show anything.</summary>
    public void Initialize() =>
        throw new NotImplementedException("put the icon into the notification area and build its menu");
    // The menu's commands are connect, disconnect, reconnect now, settings and quit.
    // Choosing settings asks the app UI to show the settings window.
    // Choosing any other command sends what the user did through the app UI.

    /// <summary>Shows what changed.</summary>
    public void Show(UIUpdate update) =>
        throw new NotImplementedException("show what changed in the icon and the menu");
    // The connection: the icon for its state, and in the menu when it came up, the tunnel
    // address, the next reconnect time, the reason for waiting and the stage it is at.
    // Updates: the mark that a new version is there.
}
