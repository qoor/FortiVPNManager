using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>The Helper's sentences (components/ui/macos/Sentences/ReadinessText.swift).</summary>
public static class ReadinessText
{
    public static string Label(this HelperState state) => state switch
    {
        HelperState.Checking => L("Checking…"),
        HelperState.NotRegistered => L("Not registered"),
        HelperState.RequiresApproval => L("Needs approval"),
        HelperState.Running running => L("Running (build %@)", running.Build),
        HelperState.VersionMismatch mismatch => L("Version mismatch (Helper %@, app %@)", mismatch.HelperBuild, mismatch.AppBuild),
        HelperState.Unreachable unreachable => L("No response: %@", unreachable.Failure.Message()),
        _ => throw new InvalidOperationException($"unknown Helper state {state}"),
    };

    public static NoticeStyle Style(this HelperState state) => state switch
    {
        HelperState.Running => NoticeStyle.Success,
        HelperState.Checking => NoticeStyle.Info,
        HelperState.NotRegistered or HelperState.RequiresApproval or HelperState.VersionMismatch or HelperState.Unreachable => NoticeStyle.Warning,
        _ => throw new InvalidOperationException($"unknown Helper state {state}"),
    };

    /// <summary>Why registering the Helper failed. On Windows the install script registers it, which the description says.</summary>
    public static string HelperMessage(this RegistrationError error) => error switch
    {
        RegistrationError.Failed failed => failed.Description,
        _ => throw new InvalidOperationException($"Windows does not report {error} for the Helper"),
    };

    public static string Message(this UntestedVersion untested) =>
        L("Not a tested version (%@.x), so connecting may fail.", untested.TestedSeries);

    /// <summary>What is wrong with openconnect. Windows looks for it in one place and checks only these.</summary>
    public static string Message(this ClientProblem problem) => problem.Kind switch
    {
        ClientProblemKind.NotFound => L("Not found at %@. Install OpenConnect for Windows in its default folder.", problem.Path),
        ClientProblemKind.NotRegularFile => L("openconnect is not a regular file: %@", problem.Path),
        ClientProblemKind.NotExecutable => L("openconnect did not report its version: %@", problem.Path),
        _ => throw new InvalidOperationException($"Windows does not report {problem.Kind} for openconnect"),
    };

    /// <summary>Why "Open at login" could not be changed.</summary>
    public static string LoginItemMessage(this RegistrationError error) => error switch
    {
        RegistrationError.NotInstalledCopy => L("Only available for the app installed in Program Files."),
        RegistrationError.Failed failed => failed.Description,
        _ => throw new InvalidOperationException($"unknown registration error {error}"),
    };

    public static string Message(this HelperChannelFailure failure) => failure switch
    {
        HelperChannelFailure.AppNotSigned => L("The app is not signed with a team certificate."),
        HelperChannelFailure.Connection connection => L("Helper connection failed: %@", connection.Description),
        HelperChannelFailure.ConnectionLost => L("The Helper connection was lost before the request was sent."),
        HelperChannelFailure.NoReply => L("The Helper is not responding."),
        HelperChannelFailure.UnreadableReply unreadable => L("Could not read the Helper response: %@", unreadable.Description),
        _ => throw new InvalidOperationException($"unknown Helper channel failure {failure}"),
    };
}
