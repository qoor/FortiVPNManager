using System.Globalization;
using System.Text.Json;
using System.Text.RegularExpressions;
using FortiVPNManager.Ui.Text;
using Xunit;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

[assembly: CollectionBehavior(DisableTestParallelization = true)]

namespace FortiVPNManager.Ui.Tests;

/// <summary>The sentences: every key the UI uses is in a catalog, and they come out as macOS writes them.</summary>
public class StringsTests
{
    internal static readonly string s_repository = FindRepository();

    public StringsTests() => Language = "en";

    [Fact]
    public void EveryKeyTheUiUsesIsInACatalog()
    {
        IEnumerable<string> sources = Directory.GetFiles(Path.Combine(s_repository, "components", "ui", "windows"), "*.cs", SearchOption.AllDirectories)
            .Where(path => !path.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}") && !path.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}"));
        var missing = new List<string>();
        foreach (string source in sources)
        {
            foreach (Match match in Regex.Matches(File.ReadAllText(source), @"\bL\(""((?:[^""\\]|\\.)*)"""))
            {
                string key = Regex.Unescape(match.Groups[1].Value);
                if (!Has(key))
                {
                    missing.Add($"{Path.GetFileName(source)}: {key}");
                }
            }
        }
        Assert.Empty(missing);
    }

    [Fact]
    public void SpecifiersTakeTheArgumentsInOrderOrByPosition()
    {
        Assert.Equal("a 7 b", Strings.Format("%@ %lld %@", ["a", 7L, "b"]));
        Assert.Equal("second first", Strings.Format("%2$@ %1$@", ["first", "second"]));
        Assert.Equal("100% 1234", Strings.Format("100%% %lld", [1234L]));
    }

    [Fact]
    public void PluralFormsFollowTheNumber()
    {
        Assert.Equal("1 second", L("%lld seconds", 1UL));
        Assert.Equal("2 seconds", L("%lld seconds", 2UL));
    }

    [Fact]
    public void KoreanComesFromTheCatalog()
    {
        Language = "ko";
        try
        {
            Assert.Equal(Korean("Localizable.xcstrings", "Not connected"), L("Not connected"));
            Assert.Equal(Korean("Windows.xcstrings", "Check Helper Service…"), L("Check Helper Service…"));
        }
        finally
        {
            Language = "en";
        }
    }

    [Fact]
    public void TimesAreWrittenAsOnMacOs()
    {
        DateTimeOffset now = DateTimeOffset.Now;
        var time = new TimeStyle(now, CultureInfo.GetCultureInfo("en-US"));
        Assert.Equal("1 hr 23 min", TimeStyle.Duration(TimeSpan.FromMinutes(83)));
        Assert.Equal("2 hr", TimeStyle.Duration(TimeSpan.FromMinutes(120)));
        Assert.Equal("less than a minute", TimeStyle.Duration(TimeSpan.FromSeconds(59)));
        Assert.Equal("1 hour, 5 minutes", TimeStyle.SpelledOut(3900));
        Assert.Equal("50 seconds", TimeStyle.SpelledOut(50));
        Assert.Equal("1:12", TimeStyle.Countdown(TimeSpan.FromSeconds(71.2)));
        Assert.Equal("Reconnects soon", time.PlannedReconnect(now.AddSeconds(30)));
        Assert.Equal("Reconnects in about 1 hr 32 min", time.PlannedReconnect(now.AddMinutes(92).AddSeconds(20)));
    }

    [Fact]
    public void TheStatusLineDescribesTheConnection()
    {
        DateTimeOffset now = DateTimeOffset.Now;
        var time = new TimeStyle(now, CultureInfo.GetCultureInfo("en-US"));
        var connected = new ConnectionSection(
            ConnectionIcon.Connected,
            new ConnectionStatus.Connected(now.AddMinutes(-83), new ProactiveReconnect.At(now.AddMinutes(92).AddSeconds(20)), "10.212.0.25"),
            ButtonState.Disabled, ButtonState.Enabled, ButtonState.Enabled, false);
        Assert.Equal("Connected · 1 hr 23 min · Reconnects in about 1 hr 32 min", connected.StatusLine(null, time));
        Assert.Equal("Connected · 1 hr 23 min", connected.StatusLine(null, time, includesPlannedReconnect: false));
        ConnectionSection waiting = connected with { Status = new ConnectionStatus.WaitingToReconnect(now.AddSeconds(72)) };
        Assert.Equal("Waiting to reconnect (1:12)", waiting.StatusLine(null, time));
        ConnectionSection refused = connected with
        {
            Status = new ConnectionStatus.Failed(new FailureReason.Helper(HelperErrorCode.InvalidRequest)),
        };
        Assert.Equal("Error: Invalid connection request", refused.StatusLine(null, time));
    }

    [Fact]
    public void TheHelperStartMessageSaysWhatWhyAndWhatToDo()
    {
        Assert.Equal(
            "The Helper failed to start, so FortiVPNManager will quit.\nThe Helper service is disabled.\nReinstall FortiVPNManager or contact your administrator.",
            HelperStartText.Message(HelperStartText.Disabled()));
        Assert.Equal(
            "An error occurred while the Helper was starting. FortiVPNManager will quit. (attempt 2/2)\nThe Helper did not start in time.\nReinstall FortiVPNManager or contact your administrator.",
            HelperStartText.MessageAfterRetry(HelperStartText.TimedOut()));
    }

    private static readonly AutoUpdateSection s_updateSection = new(
        "26.1.0", new UpdateStatus.Checking(), new UpdateProgress.Idle(),
        ButtonState.Disabled, ButtonState.Hidden, ButtonState.Hidden, ButtonState.Hidden, false, false);

    private static readonly UpdateProgress.Downloading s_downloading = new(19_500_000, 30_800_000, 63, new RemainingTime.Minutes(2));
    private static readonly UpdateProgress.Preparing s_preparing = new(95);
    private static readonly UpdateProgress.ReadyToRestart s_ready = new("26.2.0");
    private static readonly UpdateProgress.Failed s_downloadFailed = new(new InstallFailure.DownloadFailed("timed out"));

    private static UpdateStatus.Available Available(Dictionary<string, IReadOnlyList<string>>? titles = null) =>
        new("26.2.0", "https://github.com/qoor/FortiVPNManager/releases/tag/v26.2.0", 30_800_000, titles ?? []);

    private static AutoUpdateSection Section(UpdateStatus status, UpdateProgress? progress = null) =>
        s_updateSection with { Status = status, Progress = progress ?? new UpdateProgress.Idle() };

    private static AutoUpdateSection CheckFailed(UpdateCheckFailure reason) => Section(new UpdateStatus.CheckFailed(reason));

    [Fact]
    public void TheUpdatePageSaysWhatIsAvailableAndHowTheInstallGoes()
    {
        Assert.Equal("Checking for updates…", s_updateSection.Headline());
        Assert.Null(s_updateSection.Subtitle());
        AutoUpdateSection upToDate = Section(new UpdateStatus.UpToDate());
        Assert.Equal("You're up to date", upToDate.Headline());
        Assert.Equal("FortiVPNManager 26.1.0", upToDate.Subtitle());
        foreach (UpdateProgress progress in new UpdateProgress[] { new UpdateProgress.Idle(), s_downloading, s_preparing, s_downloadFailed })
        {
            Assert.Equal("Updates available", Section(Available(), progress).Headline());
            Assert.Equal("Current version: 26.1.0", Section(Available(), progress).Subtitle());
        }
        Assert.Equal("Restart required", Section(Available(), s_ready).Headline());
        Assert.Equal(
            "The update installs when you restart the app. The VPN disconnects briefly during the install.",
            Section(Available(), s_ready).Subtitle());
        AutoUpdateSection installing = Section(Available(), new UpdateProgress.Installing("26.2.0"));
        Assert.Equal("Installing…", installing.Headline());
        Assert.Equal("The app quits to install the update and opens again when it's done.", installing.Subtitle());
        AutoUpdateSection checkFailed = CheckFailed(new UpdateCheckFailure.HttpStatus(403));
        Assert.Equal("Couldn't check for updates (HTTP 403).", checkFailed.Headline());
        Assert.Equal("Current version: 26.1.0", checkFailed.Subtitle());
    }

    [Fact]
    public void ACheckFailureSaysWhatWentWrong()
    {
        Assert.Equal("Couldn't check for updates (HTTP 403).", new UpdateCheckFailure.HttpStatus(403).Message());
        Assert.Equal("Couldn't check for updates: timed out", new UpdateCheckFailure.Network("timed out").Message());
        Assert.Equal("Couldn't check for updates (unexpected reply).", new UpdateCheckFailure.InvalidReply().Message());
        Assert.Equal("Couldn't check for updates (installer missing).", new UpdateCheckFailure.InstallerMissing().Message());
    }

    [Fact]
    public void TheUpdateItemSaysHowFarTheInstallGot()
    {
        Assert.Null(new UpdateProgress.Idle().ItemStatus());
        Assert.Null(s_downloadFailed.ItemStatus());
        Assert.Equal("Downloading - 63%", s_downloading.ItemStatus());
        Assert.Equal("Preparing update", s_preparing.ItemStatus());
        Assert.Equal("Pending restart", s_ready.ItemStatus());
        Assert.Equal("Installing", new UpdateProgress.Installing("26.2.0").ItemStatus());
        Assert.Equal("Calculating time remaining…", new RemainingTime.Calculating().Line());
        Assert.Equal("About 1 minute remaining", new RemainingTime.Minutes(1).Line());
        Assert.Equal("About 2 minutes remaining", new RemainingTime.Minutes(2).Line());
        Assert.Equal("About 45 seconds remaining", new RemainingTime.Seconds(45).Line());
    }

    [Fact]
    public void AFailedInstallSaysWhatToDo()
    {
        Assert.Null(s_updateSection.FailedInstallLine());
        Assert.Equal("Couldn't install version 26.2.0.", (s_updateSection with { FailedInstallVersion = "26.2.0" }).FailedInstallLine());
        Assert.Equal("Couldn't download the update. Check the network connection.", new InstallFailure.DownloadFailed("timed out").Message());
        string prepare = "Couldn't prepare the install. Try installing again.";
        Assert.Equal(prepare, new InstallFailure.WaiterFailed("access denied").Message());
        Assert.Equal(prepare, new InstallFailure.HelperRefused(UpdateRefusal.NothingPrepared, "").Message());
        string helper = "Couldn't reach the Helper, so the update wasn't installed.";
        Assert.Equal(helper, new InstallFailure.HelperUnreachable(new HelperChannelFailure.NoReply()).Message());
        Assert.Equal(helper, new InstallFailure.HelperBuildMismatch("1", "2").Message());
        Assert.Equal(helper, new InstallFailure.HelperFailed("failed").Message());
        string verify = "Couldn't verify the update file. Try installing again.";
        Assert.Equal(verify, new InstallFailure.HelperRefused(UpdateRefusal.SignatureInvalid, "").Message());
        Assert.Equal(verify, new InstallFailure.HelperRefused(UpdateRefusal.ChecksumMismatch, "").Message());
        Assert.Equal(verify, new InstallFailure.HelperRefused(UpdateRefusal.FilesUnreadable, "").Message());
        Assert.Equal("The same or a newer version is already installed.", new InstallFailure.HelperRefused(UpdateRefusal.VersionNotNewer, "").Message());
        Assert.Equal("Another update is already being installed.", new InstallFailure.HelperRefused(UpdateRefusal.AlreadyInstalling, "").Message());
    }

    [Fact]
    public void ReleaseNotesFollowTheAppsLanguageThenEnglish()
    {
        UpdateStatus.Available both = Available(new() { ["ko"] = ["한국어 제목"], ["en"] = ["English title"] });
        UpdateStatus.Available english = Available(new() { ["en"] = ["English title"] });
        UpdateStatus.Available korean = Available(new() { ["ko"] = ["한국어 제목"] });
        Assert.Equal(["English title"], both.ReleaseNotes()!);
        Assert.Null(korean.ReleaseNotes());
        Assert.Null(Available().ReleaseNotes());
        Language = "ko";
        try
        {
            Assert.Equal(["한국어 제목"], both.ReleaseNotes()!);
            Assert.Equal(["English title"], english.ReleaseNotes()!);
        }
        finally
        {
            Language = "en";
        }
    }

    [Fact]
    public void TheUpdateNotificationNamesTheVersionAndOffersTheUpdate()
    {
        var notification = new UserNotification.UpdateAvailable("26.2.0");
        Assert.Equal(BypassText.UpdateAvailableIdentifier, notification.Identifier());
        Assert.Equal("Software Update Available", notification.Title());
        Assert.Equal("Update to FortiVPNManager 26.2.0 to get the latest features.", notification.Body());
        Assert.Equal("Update Now", notification.ButtonTitle());
        Assert.Null(new UserNotification.ExceptionRulesNotApplied(3, new BypassFailure.RoutesNotKept()).ButtonTitle());
    }

    [Fact]
    public void TheUpdateSentencesInKoreanAreTheApprovedOnes()
    {
        Language = "ko";
        try
        {
            Assert.Equal("소프트웨어 업데이트", L("Software Update"));
            Assert.Equal("업데이트 확인 중…", s_updateSection.Headline());
            Assert.Equal("최신 상태입니다", Section(new UpdateStatus.UpToDate()).Headline());
            Assert.Equal("업데이트 사용 가능", Section(Available()).Headline());
            Assert.Equal("현재 버전: 26.1.0", Section(Available()).Subtitle());
            Assert.Equal("재시작 필요", Section(Available(), s_ready).Headline());
            Assert.Equal("앱을 다시 시작하면 업데이트가 설치됩니다. 설치하는 동안 VPN 연결이 잠시 끊깁니다.", Section(Available(), s_ready).Subtitle());
            Assert.Equal("설치 중…", Section(Available(), new UpdateProgress.Installing("26.2.0")).Headline());
            Assert.Equal("앱을 종료하고 업데이트를 설치합니다. 설치가 끝나면 앱이 다시 열립니다.", Section(Available(), new UpdateProgress.Installing("26.2.0")).Subtitle());
            Assert.Equal("업데이트 확인", L("Check for updates"));
            Assert.Equal("지금 설치", L("Install now"));
            Assert.Equal("다시 시도", L("Try again"));
            Assert.Equal("앱 재시작", L("Restart app"));
            Assert.Equal("취소", L("Cancel"));
            Assert.Equal("다운로드 중 - 63%", s_downloading.ItemStatus());
            Assert.Equal("업데이트 준비 중", s_preparing.ItemStatus());
            Assert.Equal("재시작 대기 중", s_ready.ItemStatus());
            Assert.Equal("설치 중", new UpdateProgress.Installing("26.2.0").ItemStatus());
            Assert.Equal("남은 시간 계산 중…", new RemainingTime.Calculating().Line());
            Assert.Equal("약 1분 남음", new RemainingTime.Minutes(1).Line());
            Assert.Equal("약 2분 남음", new RemainingTime.Minutes(2).Line());
            Assert.Equal("약 45초 남음", new RemainingTime.Seconds(45).Line());
            Assert.Equal("변경 사항", L("What's new"));
            Assert.Equal("GitHub에서 보기", L("View on GitHub"));
            Assert.Equal("설치하는 동안 VPN 연결이 잠시 끊깁니다.", L("The VPN disconnects briefly during the install."));
            Assert.Equal("업데이트를 확인하지 못했습니다(HTTP 403).", new UpdateCheckFailure.HttpStatus(403).Message());
            Assert.Equal("업데이트를 확인하지 못했습니다: timed out", new UpdateCheckFailure.Network("timed out").Message());
            Assert.Equal("업데이트를 확인하지 못했습니다(응답 형식 오류).", new UpdateCheckFailure.InvalidReply().Message());
            Assert.Equal("업데이트를 확인하지 못했습니다(설치 파일 없음).", new UpdateCheckFailure.InstallerMissing().Message());
            Assert.Equal("26.2.0 버전을 설치하지 못했습니다.", (s_updateSection with { FailedInstallVersion = "26.2.0" }).FailedInstallLine());
            Assert.Equal("업데이트를 받지 못했습니다. 네트워크 연결을 확인하세요.", new InstallFailure.DownloadFailed("").Message());
            Assert.Equal("Helper와 통신하지 못해 설치하지 못했습니다.", new InstallFailure.HelperFailed("").Message());
            Assert.Equal("업데이트 파일을 확인하지 못했습니다. 다시 설치하세요.", new InstallFailure.HelperRefused(UpdateRefusal.SignatureInvalid, "").Message());
            Assert.Equal("같거나 더 새로운 버전이 이미 설치되어 있습니다.", new InstallFailure.HelperRefused(UpdateRefusal.VersionNotNewer, "").Message());
            Assert.Equal("다른 업데이트를 설치하고 있습니다.", new InstallFailure.HelperRefused(UpdateRefusal.AlreadyInstalling, "").Message());
            Assert.Equal("설치를 준비하지 못했습니다. 다시 설치하세요.", new InstallFailure.WaiterFailed("").Message());
            Assert.Equal("설치를 준비하지 못했습니다. 다시 설치하세요.", new InstallFailure.HelperRefused(UpdateRefusal.NothingPrepared, "").Message());
            var notification = new UserNotification.UpdateAvailable("26.2.0");
            Assert.Equal("소프트웨어 업데이트 사용 가능", notification.Title());
            Assert.Equal("최신 기능을 사용하려면 FortiVPNManager 26.2.0 버전으로 업데이트하세요.", notification.Body());
            Assert.Equal("지금 업데이트", notification.ButtonTitle());
        }
        finally
        {
            Language = "en";
        }
    }

    private static string Korean(string catalog, string key)
    {
        using var document = JsonDocument.Parse(File.ReadAllText(catalog == "Windows.xcstrings"
            ? Path.Combine(s_repository, "components", "ui", "windows", catalog)
            : Path.Combine(s_repository, "components", "ui", "macos", catalog)));
        return document.RootElement.GetProperty("strings").GetProperty(key).GetProperty("localizations")
            .GetProperty("ko").GetProperty("stringUnit").GetProperty("value").GetString()!;
    }

    private static string FindRepository()
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null && !Directory.Exists(Path.Combine(directory.FullName, "components", "ui")))
        {
            directory = directory.Parent;
        }
        return directory?.FullName ?? throw new InvalidOperationException("the repository is not above the test output");
    }
}
