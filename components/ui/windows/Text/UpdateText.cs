using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

public static class UpdateText
{
    public static string Headline(this AutoUpdateSection section)
    {
        ArgumentNullException.ThrowIfNull(section);
        return section.Progress switch
        {
            UpdateProgress.Downloading or UpdateProgress.Preparing => L("Updates available"),
            UpdateProgress.ReadyToRestart => L("Restart required"),
            UpdateProgress.Installing => L("Installing…"),
            UpdateProgress.Idle or UpdateProgress.Failed => section.Status switch
            {
                UpdateStatus.Checking => L("Checking for updates…"),
                UpdateStatus.UpToDate => L("You're up to date"),
                UpdateStatus.Available => L("Updates available"),
                UpdateStatus.CheckFailed failed => failed.Reason.Message(),
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
            UpdateProgress.Downloading or UpdateProgress.Preparing => L("Current version: %@", section.CurrentVersion),
            UpdateProgress.ReadyToRestart =>
                L("The update installs when you restart the app. The VPN disconnects briefly during the install."),
            UpdateProgress.Installing => L("The app quits to install the update and opens again when it's done."),
            UpdateProgress.Idle or UpdateProgress.Failed => section.Status switch
            {
                UpdateStatus.Checking => null,
                UpdateStatus.UpToDate => $"FortiVPNManager {section.CurrentVersion}",
                UpdateStatus.Available or UpdateStatus.CheckFailed => L("Current version: %@", section.CurrentVersion),
                _ => throw new InvalidOperationException($"unknown update status {section.Status}"),
            },
            _ => throw new InvalidOperationException($"unknown update progress {section.Progress}"),
        };
    }

    public static string Message(this UpdateCheckFailure failure) => failure switch
    {
        UpdateCheckFailure.HttpStatus status => L("Couldn't check for updates (HTTP %lld).", status.Code),
        UpdateCheckFailure.Network network => L("Couldn't check for updates: %@", network.Detail),
        UpdateCheckFailure.InvalidReply => L("Couldn't check for updates (unexpected reply)."),
        UpdateCheckFailure.InstallerMissing => L("Couldn't check for updates (installer missing)."),
        _ => throw new InvalidOperationException($"unknown update check failure {failure}"),
    };

    public static string? FailedInstallLine(this AutoUpdateSection section)
    {
        ArgumentNullException.ThrowIfNull(section);
        return section.FailedInstallVersion is { } version ? L("Couldn't install version %@.", version) : null;
    }

    public static IReadOnlyList<string>? ReleaseNotes(this UpdateStatus.Available available)
    {
        ArgumentNullException.ThrowIfNull(available);
        return available.ReleaseNoteTitles.GetValueOrDefault(Language) ?? available.ReleaseNoteTitles.GetValueOrDefault("en");
    }

    public static string? ItemStatus(this UpdateProgress progress) => progress switch
    {
        UpdateProgress.Downloading downloading => L("Downloading - %lld%%", downloading.Percent),
        UpdateProgress.Preparing => L("Preparing update"),
        UpdateProgress.ReadyToRestart => L("Pending restart"),
        UpdateProgress.Installing => L("Installing"),
        UpdateProgress.Idle or UpdateProgress.Failed => null,
        _ => throw new InvalidOperationException($"unknown update progress {progress}"),
    };

    public static string Line(this RemainingTime remaining) => remaining switch
    {
        RemainingTime.Calculating => L("Calculating time remaining…"),
        RemainingTime.Minutes minutes => L("About %lld minutes remaining", minutes.Value),
        RemainingTime.Seconds seconds => L("About %lld seconds remaining", seconds.Value),
        _ => throw new InvalidOperationException($"unknown remaining time {remaining}"),
    };

    public static string Message(this InstallFailure failure) => failure switch
    {
        InstallFailure.DownloadFailed => L("Couldn't download the update. Check the network connection."),
        InstallFailure.WaiterFailed => L("Couldn't prepare the install. Try installing again."),
        InstallFailure.HelperUnreachable or InstallFailure.HelperBuildMismatch or InstallFailure.HelperFailed =>
            L("Couldn't reach the Helper, so the update wasn't installed."),
        InstallFailure.HelperRefused refused => refused.Reason switch
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
