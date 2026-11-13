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
    private static readonly string Repository = FindRepository();

    public StringsTests()
    {
        Language = "en";
    }

    [Fact]
    public void EveryKeyTheUiUsesIsInACatalog()
    {
        var sources = Directory.GetFiles(Path.Combine(Repository, "components", "ui", "windows"), "*.cs", SearchOption.AllDirectories)
            .Where(path => !path.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}") && !path.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}"));
        var missing = new List<string>();
        foreach (var source in sources)
        {
            foreach (Match match in Regex.Matches(File.ReadAllText(source), @"\bL\(""((?:[^""\\]|\\.)*)"""))
            {
                var key = Regex.Unescape(match.Groups[1].Value);
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
            Assert.Equal(Korean("Windows.xcstrings", "openconnect not found"), L("openconnect not found"));
        }
        finally
        {
            Language = "en";
        }
    }

    [Fact]
    public void TimesAreWrittenAsOnMacOs()
    {
        var now = DateTimeOffset.Now;
        var time = new TimeStyle(now, CultureInfo.GetCultureInfo("en-US"));
        Assert.Equal("1 hr 23 min", time.Duration(TimeSpan.FromMinutes(83)));
        Assert.Equal("2 hr", time.Duration(TimeSpan.FromMinutes(120)));
        Assert.Equal("less than a minute", time.Duration(TimeSpan.FromSeconds(59)));
        Assert.Equal("1 hour, 5 minutes", time.SpelledOut(3900));
        Assert.Equal("50 seconds", time.SpelledOut(50));
        Assert.Equal("1:12", TimeStyle.Countdown(TimeSpan.FromSeconds(71.2)));
        Assert.Equal("Reconnects soon", time.PlannedReconnect(now.AddSeconds(30)));
        Assert.Equal("Reconnects in about 1 hr 32 min", time.PlannedReconnect(now.AddMinutes(92).AddSeconds(20)));
    }

    [Fact]
    public void TheStatusLineDescribesTheConnection()
    {
        var now = DateTimeOffset.Now;
        var time = new TimeStyle(now, CultureInfo.GetCultureInfo("en-US"));
        var connected = new ConnectionSection(
            ConnectionIcon.Connected,
            new ConnectionStatus.Connected(now.AddMinutes(-83), new ProactiveReconnect.At(now.AddMinutes(92).AddSeconds(20)), "10.212.0.25"),
            ButtonState.Disabled, ButtonState.Enabled, ButtonState.Enabled, false);
        Assert.Equal("Connected · 1 hr 23 min · Reconnects in about 1 hr 32 min", connected.StatusLine(null, time));
        Assert.Equal("Connected · 1 hr 23 min", connected.StatusLine(null, time, includesPlannedReconnect: false));
        var waiting = connected with { Status = new ConnectionStatus.WaitingToReconnect(now.AddSeconds(72)) };
        Assert.Equal("Waiting to reconnect (1:12)", waiting.StatusLine(null, time));
    }

    private static string Korean(string catalog, string key)
    {
        using var document = JsonDocument.Parse(File.ReadAllText(catalog == "Windows.xcstrings"
            ? Path.Combine(Repository, "components", "ui", "windows", catalog)
            : Path.Combine(Repository, "components", "ui", "macos", catalog)));
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
