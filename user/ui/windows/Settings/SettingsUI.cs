namespace FortiVPNManager.user.ui.windows.Settings;

/// <summary>The settings window with its panes: it shows the settings and takes the user's changes.</summary>
internal sealed class SettingsUI
{
    /// <summary>Creates the UI with the app UI it sends what the user does through.</summary>
    public SettingsUI(AppUI app) =>
        throw new NotImplementedException("create the settings UI");

    /// <summary>Makes what the UI needs before it can show anything.</summary>
    public void Initialize() =>
        throw new NotImplementedException("build the settings window with its panes");
    // The panes are general, account, connection, exception rules, advanced and log.
    // What the user confirms in a pane is sent through the app UI: a changed setting, a
    // password, linking or unlinking the Google account, the Gmail test, loading the settings
    // of an existing VPN client, and checking for an update, starting or cancelling its
    // download and installing it.
    // Copying and clearing the log pane stay in the window.

    /// <summary>Shows the settings window.</summary>
    public void Open() =>
        throw new NotImplementedException("show the window and bring it to the front");

    /// <summary>Shows what changed.</summary>
    public void Show(UIUpdate update) =>
        throw new NotImplementedException("show what changed in the pane it belongs to");
    // The connection: which of connect, disconnect and reconnect now can be used.
    // The settings: the values of every pane, a problem in the settings file, and the result
    // of loading the settings of an existing VPN client.
    // The accounts: whether they are ready and what is missing, and the result of a test.
    // The exception routes: the problem of each row.
    // Updates: the version, the progress and a failure.
    // A log line: one more line in the log pane.
}
