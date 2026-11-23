using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>
/// The messages the app shows when it quits at launch because the Helper's service could not be made
/// to run (components/os-integration/windows/HelperStart.cs): what happens, why, and what to do.
/// </summary>
public static class HelperStartText
{
    /// <summary>When the Helper cannot be started at all.</summary>
    public static string Message(string reason) => string.Join(
        "\n",
        L("The Helper failed to start, so FortiVPNManager will quit."),
        reason,
        L("Reinstall FortiVPNManager or contact your administrator."));

    /// <summary>When the start was made once more and failed again.</summary>
    public static string MessageAfterRetry(string reason) => string.Join(
        "\n",
        L("An error occurred while the Helper was starting. FortiVPNManager will quit. (attempt 2/2)"),
        reason,
        L("Reinstall FortiVPNManager or contact your administrator."));

    public static string NotInstalled() => L("The Helper service is not installed.");

    public static string Disabled() => L("The Helper service is disabled.");

    public static string NoPermission() => L("This account has no permission to start the Helper service.");

    public static string TimedOut() => L("The Helper did not start in time.");

    public static string StoppedWithError() => L("The Helper stopped with an error while starting.");
}
