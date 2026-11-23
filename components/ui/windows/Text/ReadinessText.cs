using FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>The Helper's sentences (components/ui/macos/Sentences/ReadinessText.swift).</summary>
public static class ReadinessText
{
    public static string Label(this HelperState state) => state switch
    {
        HelperStateChecking => L("Checking…"),
        HelperStateNotRegistered => L("Not registered"),
        HelperStateRequiresApproval => L("Needs approval"),
        HelperStateRunning running => L("Running (build %@)", running.Build),
        HelperStateVersionMismatch mismatch => L("Version mismatch (Helper %@, app %@)", mismatch.HelperBuild, mismatch.AppBuild),
        HelperStateUnreachable unreachable => L("No response: %@", unreachable.Failure.Message()),
        _ => throw new InvalidOperationException($"unknown Helper state {state}"),
    };

    public static NoticeStyle Style(this HelperState state) => state switch
    {
        HelperStateRunning => NoticeStyle.Success,
        HelperStateChecking => NoticeStyle.Info,
        HelperStateNotRegistered or HelperStateRequiresApproval or HelperStateVersionMismatch or HelperStateUnreachable => NoticeStyle.Warning,
        _ => throw new InvalidOperationException($"unknown Helper state {state}"),
    };

    /// <summary>Why registering the Helper failed. On Windows the installer registers it, which the description says.</summary>
    public static string HelperMessage(this RegistrationError error) => error switch
    {
        RegistrationErrorFailed failed => failed.Description,
        _ => throw new InvalidOperationException($"Windows does not report {error} for the Helper"),
    };

    /// <summary>Why "Open at login" could not be changed.</summary>
    public static string LoginItemMessage(this RegistrationError error) => error switch
    {
        RegistrationErrorNotInstalledCopy => L("Only available for the app installed in Program Files."),
        RegistrationErrorFailed failed => failed.Description,
        _ => throw new InvalidOperationException($"unknown registration error {error}"),
    };

    public static string Message(this HelperChannelFailure failure) => failure switch
    {
        HelperChannelFailureAppNotSigned => L("The app is not signed with a team certificate."),
        HelperChannelFailureConnection connection => L("Helper connection failed: %@", connection.Description),
        HelperChannelFailureConnectionLost => L("The Helper connection was lost before the request was sent."),
        HelperChannelFailureNoReply => L("The Helper is not responding."),
        HelperChannelFailureUnreadableReply unreadable => L("Could not read the Helper response: %@", unreadable.Description),
        _ => throw new InvalidOperationException($"unknown Helper channel failure {failure}"),
    };
}
