using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using FortiVPNManager.Contract;
using Xunit;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.Ui.Tests;

/// <summary>
/// The Log pane's lines (components/ui/tests/macos/LogLinesTests.swift): what it keeps, and Copy All
/// against the shared fixture contract/tests/fixtures/app-internal/log_copy_all.json, whose copyAll
/// lines are also the log file's lines.
/// </summary>
public class LogLinesTests
{
    [Fact]
    public void CopyAllWritesTheLogFileFormat()
    {
        var fixture = JsonNode.Parse(File.ReadAllText(FixturePath("log_copy_all.json")))!;
        var message = fixture["message"].Deserialize<FromRuntime>(MessageCoding.Options);
        var log = new LogLines();
        log.Append(Assert.IsType<FromRuntime.LogLines>(message).Lines);
        var zone = TimeZoneInfo.FindSystemTimeZoneById(fixture["timeZone"]!.GetValue<string>());
        var expected = string.Join("\n", fixture["copyAll"]!.AsArray().Select(line => line!.GetValue<string>()));
        Assert.Equal(expected, log.CopyAllText(zone));
    }

    [Fact]
    public void KeepsTheNewestLinesWithinTheLineLimit()
    {
        var log = new LogLines();
        log.Append(Enumerable.Range(0, LogLines.MaxLines + 3).Select(index => Line($"{index}")).ToList());
        Assert.Equal(LogLines.MaxLines, log.Entries.Count);
        Assert.Equal("3", log.Entries[0].Line.Text);
        Assert.Equal($"{LogLines.MaxLines + 2}", log.Entries[^1].Line.Text);
    }

    [Fact]
    public void KeepsTheNewestLinesWithinTheSizeLimit()
    {
        var log = new LogLines();
        // Each line costs its UTF-8 length plus one for its line break: 1 KiB.
        var text = new string('x', 1023);
        log.Append(Enumerable.Range(0, (2 * 1024) + 2).Select(_ => Line(text)).ToList());
        Assert.Equal(2 * 1024, log.Entries.Count);
    }

    [Fact]
    public void ClearEmptiesThePaneAndKeepsCounting()
    {
        var log = new LogLines();
        log.Append([Line("a"), Line("b")]);
        var lastId = log.Entries[^1].Id;
        log.Clear();
        Assert.Empty(log.Entries);
        Assert.Equal("", log.CopyAllText(TimeZoneInfo.Utc));
        log.Append([Line("c")]);
        Assert.Equal(["c"], log.Entries.Select(entry => entry.Line.Text));
        Assert.True(log.Entries[0].Id > lastId);
    }

    [Fact]
    public void EntriesAfterAnIdAreTheNewerOnes()
    {
        var log = new LogLines();
        log.Append([Line("a"), Line("b"), Line("c")]);
        Assert.Equal(["b", "c"], log.After(log.Entries[0].Id).Select(entry => entry.Line.Text));
        Assert.Equal(3, log.After(null).Count());
    }

    [Fact]
    public void UniversalTimeEndsWithZ()
    {
        var line = new LogLine(DateTimeOffset.FromUnixTimeMilliseconds(1_762_412_345_250), LogSource.Note, "x");
        Assert.Equal("2025-11-06T06:59:05.250Z [▸] x", LogLines.FileLine(line, TimeZoneInfo.Utc));
    }

    private static LogLine Line(string text) => new(DateTimeOffset.FromUnixTimeSeconds(1_762_412_345), LogSource.Stdout, text);

    private static string FixturePath(string name, [CallerFilePath] string here = "") =>
        Path.Combine(Path.GetDirectoryName(here)!, "..", "..", "..", "contract", "tests", "fixtures", "app-internal", name);
}
