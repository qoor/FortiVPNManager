using FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

public static class UpdateText
{
    public static string Headline(this AutoUpdateSection section)
    {
        ArgumentNullException.ThrowIfNull(section);
        return section.Progress switch
        {
            UpdateProgressDownloading or UpdateProgressPreparing => L("Updates available"),
            UpdateProgressReadyToRestart => L("Restart required"),
            UpdateProgressInstalling => L("Installing…"),
            UpdateProgressIdle or UpdateProgressFailed => section.Status switch
            {
                UpdateStatusChecking => L("Checking for updates…"),
                UpdateStatusUpToDate => L("You're up to date"),
                UpdateStatusAvailable => L("Updates available"),
                UpdateStatusCheckFailed failed => failed.Reason.Message(),
                _ => throw new InvalidOperationException($"unknown update status {section.Status}"),
            },
            _ => throw new InvalidOperationException($"unknown update progress {section.Progress}"),
        };
    }

    public static string? Subtitle(this AutoUpdateSection section)
    {
        ArgumentNullException.ThrowIfNull(section);
        return section.Progress switch
        {
            UpdateProgressDownloading or UpdateProgressPreparing => L("Current version: %@", section.CurrentVersion),
            UpdateProgressReadyToRestart =>
                L("The update installs when you restart the app. The VPN disconnects briefly during the install."),
            UpdateProgressInstalling => L("The app quits to install the update and opens again when it's done."),
            UpdateProgressIdle or UpdateProgressFailed => section.Status switch
            {
                UpdateStatusChecking => null,
                UpdateStatusUpToDate => $"FortiVPNManager {section.CurrentVersion}",
                UpdateStatusAvailable or UpdateStatusCheckFailed => L("Current version: %@", section.CurrentVersion),
                _ => throw new InvalidOperationException($"unknown update status {section.Status}"),
            },
            _ => throw new InvalidOperationException($"unknown update progress {section.Progress}"),
        };
    }

    public static string Message(this UpdateCheckFailure failure) => failure switch
    {
        UpdateCheckFailureHttpStatus status => L("Couldn't check for updates (HTTP %lld).", status.Code),
        UpdateCheckFailureNetwork network => L("Couldn't check for updates: %@", network.Detail),
        UpdateCheckFailureInvalidReply => L("Couldn't check for updates (unexpected reply)."),
        UpdateCheckFailureInstallerMissing => L("Couldn't check for updates (installer missing)."),
        _ => throw new InvalidOperationException($"unknown update check failure {failure}"),
    };

    public static string? FailedInstallLine(this AutoUpdateSection section)
    {
        ArgumentNullException.ThrowIfNull(section);
        return section.FailedInstallVersion is { } version ? L("Couldn't install version %@.", version) : null;
    }

    public static IReadOnlyList<string>? ReleaseNotes(this UpdateStatusAvailable available)
    {
        ArgumentNullException.ThrowIfNull(available);
        return available.ReleaseNoteTitles.GetValueOrDefault(Language) ?? available.ReleaseNoteTitles.GetValueOrDefault("en");
    }

    public static string? ItemStatus(this UpdateProgress progress) => progress switch
    {
        UpdateProgressDownloading downloading => L("Downloading - %lld%%", downloading.Percent),
        UpdateProgressPreparing => L("Preparing update"),
        UpdateProgressReadyToRestart => L("Pending restart"),
        UpdateProgressInstalling => L("Installing"),
        UpdateProgressIdle or UpdateProgressFailed => null,
        _ => throw new InvalidOperationException($"unknown update progress {progress}"),
    };

    public static string Line(this RemainingTime remaining) => remaining switch
    {
        RemainingTimeCalculating => L("Calculating time remaining…"),
        RemainingTimeMinutes minutes => L("About %lld minutes remaining", minutes.Value),
        RemainingTimeSeconds seconds => L("About %lld seconds remaining", seconds.Value),
        _ => throw new InvalidOperationException($"unknown remaining time {remaining}"),
    };

    public static string Message(this InstallFailure failure) => failure switch
    {
        InstallFailureDownloadFailed => L("Couldn't download the update. Check the network connection."),
        InstallFailureWaiterFailed => L("Couldn't prepare the install. Try installing again."),
        InstallFailureHelperUnreachable or InstallFailureHelperBuildMismatch or InstallFailureHelperFailed =>
            L("Couldn't reach the Helper, so the update wasn't installed."),
        InstallFailureHelperRefused refused => refused.Reason switch
        {
            UpdateRefusal.SignatureInvalid or UpdateRefusal.ChecksumMismatch or UpdateRefusal.FilesUnreadable =>
                L("Couldn't verify the update file. Try installing again."),
            UpdateRefusal.VersionNotNewer => L("The same or a newer version is already installed."),
            UpdateRefusal.AlreadyInstalling => L("Another update is already being installed."),
            UpdateRefusal.NothingPrepared => L("Couldn't prepare the install. Try installing again."),
            _ => throw new InvalidOperationException($"unknown update refusal {refused.Reason}"),
        },
        _ => throw new InvalidOperationException($"unknown install failure {failure}"),
    };
}
