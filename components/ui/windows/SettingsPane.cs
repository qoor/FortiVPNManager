namespace FortiVPNManager.Ui;

/// <summary>The settings window's panes, in the order of the navigation (docs/SETTINGS-UI.md §2).</summary>
public enum SettingsPane
{
    General,
    Account,
    Connection,
    ExceptionRules,
    Advanced,
    Log,
}

public static class SettingsPanes
{
    /// <summary>The pane's English title, its key in the catalogs (the macOS pane titles).</summary>
    public static string TitleKey(this SettingsPane pane) => pane switch
    {
        SettingsPane.General => "General",
        SettingsPane.Account => "Account",
        SettingsPane.Connection => "Connection",
        SettingsPane.ExceptionRules => "Exception Rules",
        SettingsPane.Advanced => "Advanced",
        SettingsPane.Log => "Log",
        _ => throw new InvalidOperationException($"unknown pane {pane}"),
    };
}
